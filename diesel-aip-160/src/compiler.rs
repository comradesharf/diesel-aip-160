//! AIP-160 AST validation and compilation into backend predicates.

use super::parser::{self, Argument, Comparable, Comparator, Expression, Value};
use anyhow::{Result, bail, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonOperator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

/// Selects who interprets a field's literals and builds its predicates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldType {
    /// Built-in Diesel SQL type, including text, numbers, and JSON paths.
    Native,
    /// Application-defined type handled by a field adapter.
    Custom { nullable: bool },
}

/// Operations required to compile an AIP-160 expression. Native handlers and
/// custom adapters receive the original literal spelling and quoting.
pub trait Aip160FilterCompiler {
    type Predicate;

    fn native_compare(
        &self,
        field: &str,
        operator: Comparator,
        value: &Value,
    ) -> Result<Self::Predicate>;
    fn custom_compare(
        &self,
        field: &str,
        operator: ComparisonOperator,
        value: &Value,
    ) -> Result<Self::Predicate> {
        let _ = (operator, value);
        bail!("custom comparison is not supported for `{field}`")
    }
    fn constant(&self, value: bool) -> Self::Predicate;
    fn and(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate;
    fn or(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate;
    fn not(&self, predicate: Self::Predicate) -> Self::Predicate;
    fn field_type(&self, field: &str) -> Result<FieldType>;
    fn text_fields(&self) -> &'static [&'static str];
    /// Match an escaped substring pattern without regard to case, preserving
    /// SQL NULL semantics.
    fn ilike(&self, field: &str, pattern: String) -> Result<Self::Predicate>;
}

/// Parse, validate and compile an AIP-160 filter.
/// Empty filters return None, allowing callers to omit `.filter(...)` entirely.
pub fn compile<C: Aip160FilterCompiler>(
    source: &str,
    compiler: &C,
) -> Result<Option<C::Predicate>> {
    parser::parse_filter(source)?
        .map(|expression| compile_expression(&expression, compiler, None))
        .transpose()
}

fn compile_expression<C: Aip160FilterCompiler>(
    expression: &Expression,
    compiler: &C,
    restriction: Option<(&str, Comparator)>,
) -> Result<C::Predicate> {
    match expression {
        Expression::And(expressions) | Expression::Or(expressions) => {
            let mut expressions = expressions.iter();
            let first = expressions.next().expect("parser produces nonempty groups");
            expressions.try_fold(
                compile_expression(first, compiler, restriction)?,
                |left, right| {
                    let right = compile_expression(right, compiler, restriction)?;
                    Ok(if matches!(expression, Expression::And(_)) {
                        compiler.and(left, right)
                    } else {
                        compiler.or(left, right)
                    })
                },
            )
        }
        Expression::Not(expression) => {
            Ok(compiler.not(compile_expression(expression, compiler, restriction)?))
        }
        Expression::Restriction {
            left,
            operator,
            right,
        } => {
            ensure!(
                restriction.is_none(),
                "a comparison's right side accepts only literals and logical operators"
            );
            let field = field_name(left)?;
            // Validate even for composite arguments, so unknown fields fail consistently.
            compiler.field_type(&field)?;
            match right {
                Argument::Comparable(value) => {
                    compare(compiler, &field, *operator, &literal(value)?)
                }
                Argument::Composite(expression) => {
                    compile_expression(expression, compiler, Some((&field, *operator)))
                }
            }
        }
        Expression::Global(value) => {
            let value = literal(value)?;
            if let Some((field, operator)) = restriction {
                compare(compiler, field, operator, &value)
            } else {
                // Bare true, false and null are search terms, not boolean constants.
                let mut fields = compiler.text_fields().iter();
                let first = fields
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("global search is not supported"))?;
                let pattern = substring_pattern(&value.text);
                let predicate = compiler.ilike(first, pattern.clone())?;
                fields.try_fold(predicate, |left, field| {
                    Ok(compiler.or(left, compiler.ilike(field, pattern.clone())?))
                })
            }
        }
    }
}

fn field_name(comparable: &Comparable) -> Result<String> {
    match comparable {
        Comparable::Member(parts) => {
            ensure!(
                parts
                    .first()
                    .is_some_and(|part| !part.quoted && parser::valid_name(&part.text))
                    && parts
                        .iter()
                        .skip(1)
                        .all(|part| !part.quoted && !part.text.is_empty()),
                "left side must be a resource field name"
            );
            Ok(parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>()
                .join("."))
        }
        Comparable::Function { name, .. } => bail!("unsupported AIP-160 function `{name}`"),
    }
}

fn literal(comparable: &Comparable) -> Result<Value> {
    match comparable {
        Comparable::Member(parts) => {
            // Dotted text on the right is a literal (e.g. 1.5 or example.com),
            // never a field-to-field comparison.
            ensure!(
                parts.len() == 1 || parts.iter().all(|part| !part.quoted),
                "a literal cannot traverse a quoted value"
            );
            Ok(Value {
                text: parts
                    .iter()
                    .map(|part| part.text.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
                quoted: parts[0].quoted,
            })
        }
        Comparable::Function { name, .. } => bail!("unsupported AIP-160 function `{name}`"),
    }
}

fn compare<C: Aip160FilterCompiler>(
    compiler: &C,
    field: &str,
    operator: Comparator,
    value: &Value,
) -> Result<C::Predicate> {
    match compiler.field_type(field)? {
        FieldType::Native => compiler.native_compare(field, operator, value),
        FieldType::Custom { nullable } => {
            if operator == Comparator::Has && !value.quoted && value.text == "*" {
                return if nullable {
                    compiler.custom_compare(
                        field,
                        ComparisonOperator::NotEqual,
                        &Value {
                            text: "null".into(),
                            quoted: false,
                        },
                    )
                } else {
                    Ok(compiler.constant(true))
                };
            }
            if !value.quoted && value.text == "null" {
                ensure!(nullable, "field `{field}` is not nullable");
                ensure!(
                    matches!(operator, Comparator::Equal | Comparator::NotEqual),
                    "null supports only = and != comparisons"
                );
            }
            compiler.custom_compare(field, comparison_operator(operator), value)
        }
    }
}

fn comparison_operator(operator: Comparator) -> ComparisonOperator {
    match operator {
        Comparator::Equal | Comparator::Has => ComparisonOperator::Equal,
        Comparator::NotEqual => ComparisonOperator::NotEqual,
        Comparator::LessThan => ComparisonOperator::LessThan,
        Comparator::LessThanOrEqual => ComparisonOperator::LessThanOrEqual,
        Comparator::GreaterThan => ComparisonOperator::GreaterThan,
        Comparator::GreaterThanOrEqual => ComparisonOperator::GreaterThanOrEqual,
    }
}

pub(crate) fn substring_pattern(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum Predicate {
        Native(String, Comparator, Value),
        Custom(String, ComparisonOperator, Value),
        Search(String, String),
        Constant(bool),
        And(Box<Self>, Box<Self>),
        Or(Box<Self>, Box<Self>),
        Not(Box<Self>),
    }

    struct TestCompiler(&'static [&'static str]);

    impl Aip160FilterCompiler for TestCompiler {
        type Predicate = Predicate;

        fn native_compare(&self, field: &str, op: Comparator, value: &Value) -> Result<Predicate> {
            Ok(Predicate::Native(field.into(), op, value.clone()))
        }
        fn custom_compare(
            &self,
            field: &str,
            op: ComparisonOperator,
            value: &Value,
        ) -> Result<Predicate> {
            Ok(Predicate::Custom(field.into(), op, value.clone()))
        }
        fn constant(&self, value: bool) -> Predicate {
            Predicate::Constant(value)
        }
        fn and(&self, left: Predicate, right: Predicate) -> Predicate {
            Predicate::And(Box::new(left), Box::new(right))
        }
        fn or(&self, left: Predicate, right: Predicate) -> Predicate {
            Predicate::Or(Box::new(left), Box::new(right))
        }
        fn not(&self, predicate: Predicate) -> Predicate {
            Predicate::Not(Box::new(predicate))
        }
        fn field_type(&self, field: &str) -> Result<FieldType> {
            match field {
                "name" | "age" | "payload.details" => Ok(FieldType::Native),
                "code" => Ok(FieldType::Custom { nullable: false }),
                "optional" => Ok(FieldType::Custom { nullable: true }),
                _ => bail!("unknown test field `{field}`"),
            }
        }
        fn text_fields(&self) -> &'static [&'static str] {
            self.0
        }
        fn ilike(&self, field: &str, pattern: String) -> Result<Predicate> {
            Ok(Predicate::Search(field.into(), pattern))
        }
    }

    fn compiled(source: &str) -> Predicate {
        compile(source, &TestCompiler(&["name"])).unwrap().unwrap()
    }

    fn value(text: &str, quoted: bool) -> Value {
        Value {
            text: text.into(),
            quoted,
        }
    }
    fn native(field: &str, op: Comparator, text: &str, quoted: bool) -> Predicate {
        Predicate::Native(field.into(), op, value(text, quoted))
    }
    fn custom(field: &str, op: ComparisonOperator, text: &str, quoted: bool) -> Predicate {
        Predicate::Custom(field.into(), op, value(text, quoted))
    }

    #[test]
    fn native_handlers_receive_original_literals_and_all_operators() {
        for (token, op) in [
            ("=", Comparator::Equal),
            ("!=", Comparator::NotEqual),
            ("<", Comparator::LessThan),
            ("<=", Comparator::LessThanOrEqual),
            (">", Comparator::GreaterThan),
            (">=", Comparator::GreaterThanOrEqual),
            (":", Comparator::Has),
        ] {
            for (literal, text, quoted) in [
                ("001", "001", false),
                ("'001'", "001", true),
                ("null", "null", false),
                ("'*'", "*", true),
                ("a.b", "a.b", false),
            ] {
                assert_eq!(
                    compiled(&format!("age{token}{literal}")),
                    native("age", op, text, quoted)
                );
            }
        }
        assert_eq!(
            compiled("payload.details:*"),
            native("payload.details", Comparator::Has, "*", false)
        );
    }

    #[test]
    fn adapters_receive_literals_without_wildcard_or_numeric_conversion() {
        for (source, text, quoted) in [
            ("code=001", "001", false),
            ("code='001'", "001", true),
            ("code='a*b'", "a*b", true),
            ("code='null'", "null", true),
            ("code:'*'", "*", true),
        ] {
            assert_eq!(
                compiled(source),
                custom("code", ComparisonOperator::Equal, text, quoted)
            );
        }
        assert_eq!(
            compiled("code>=100"),
            custom("code", ComparisonOperator::GreaterThanOrEqual, "100", false)
        );
    }

    #[test]
    fn custom_null_and_presence_respect_nullability_and_quoting() {
        assert_eq!(
            compiled("optional=null"),
            custom("optional", ComparisonOperator::Equal, "null", false)
        );
        for source in ["optional!=null", "optional:*"] {
            assert_eq!(
                compiled(source),
                custom("optional", ComparisonOperator::NotEqual, "null", false)
            );
        }
        assert_eq!(compiled("code:*"), Predicate::Constant(true));
        assert_eq!(
            compiled("optional='null'"),
            custom("optional", ComparisonOperator::Equal, "null", true)
        );
        for source in ["code=null", "optional>null", "optional:null", "missing:*"] {
            assert!(compile(source, &TestCompiler(&[])).is_err(), "{source}");
        }
    }

    #[test]
    fn composite_arguments_and_mixed_fields_preserve_boolean_structure() {
        assert_eq!(
            compiled("age>=(001 OR NOT '002') AND code=3"),
            Predicate::And(
                Box::new(Predicate::Or(
                    Box::new(native("age", Comparator::GreaterThanOrEqual, "001", false)),
                    Box::new(Predicate::Not(Box::new(native(
                        "age",
                        Comparator::GreaterThanOrEqual,
                        "002",
                        true
                    ))))
                )),
                Box::new(custom("code", ComparisonOperator::Equal, "3", false))
            )
        );
        assert_eq!(
            compiled("name=A AND code=B OR code=C"),
            Predicate::And(
                Box::new(native("name", Comparator::Equal, "A", false)),
                Box::new(Predicate::Or(
                    Box::new(custom("code", ComparisonOperator::Equal, "B", false)),
                    Box::new(custom("code", ComparisonOperator::Equal, "C", false))
                ))
            )
        );
    }

    #[test]
    fn global_search_escapes_patterns_and_uses_only_declared_text_fields() {
        for literal in ["true", "null", "001", "a.b", "*"] {
            assert_eq!(
                compiled(literal),
                Predicate::Search("name".into(), format!("%{literal}%"))
            );
        }
        assert_eq!(
            compiled(r"'50%_off\\path'"),
            Predicate::Search("name".into(), r"%50\%\_off\\path%".into())
        );
        assert!(compile("search", &TestCompiler(&[])).is_err());
        assert_eq!(
            compile("search", &TestCompiler(&["name", "other"])).unwrap(),
            Some(Predicate::Or(
                Box::new(Predicate::Search("name".into(), "%search%".into())),
                Box::new(Predicate::Search("other".into(), "%search%".into()))
            ))
        );
    }

    #[test]
    fn invalid_fields_and_nested_restrictions_fail_before_dispatch() {
        for source in [
            "unknown=A",
            "unknown=(A OR B)",
            "'name'=A",
            "42=name",
            "name=(code=A)",
            "name=helper()",
            "regex(name, 'a')",
            "name='a'.b",
            "name == 'A'",
        ] {
            assert!(compile(source, &TestCompiler(&[])).is_err(), "{source}");
        }
        for source in ["", " \n "] {
            assert_eq!(compile(source, &TestCompiler(&[])).unwrap(), None);
        }
    }
}
