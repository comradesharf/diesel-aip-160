//! AIP-160 AST validation and compilation into backend predicates.

use super::parser::{self, Argument, Comparable, Comparator, Expression, Value};
use anyhow::{Result, bail, ensure};

/// Validated scalar values passed to Diesel; integers retain their exact value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilterLiteral {
    String(String),
    Int(i32),
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonOperator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Clone, Copy, Debug)]
pub enum FieldType {
    String { nullable: bool },
    Int32,
    Jsonb,
}

/// Typed operations required by an AIP-160 backend. No expression runtime is
/// involved: the Diesel implementations build parameterized SQL directly.
pub trait Aip160FilterCompiler {
    type Predicate;

    fn compare(
        &self,
        field: &str,
        operator: ComparisonOperator,
        value: FilterLiteral,
    ) -> Result<Self::Predicate>;
    fn constant(&self, value: bool) -> Self::Predicate;
    fn and(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate;
    fn or(&self, left: Self::Predicate, right: Self::Predicate) -> Self::Predicate;
    fn not(&self, predicate: Self::Predicate) -> Self::Predicate;
    fn field_type(&self, field: &str) -> Result<FieldType>;
    fn text_fields(&self) -> &'static [&'static str];
    fn like(&self, field: &str, pattern: String) -> Result<Self::Predicate>;
    fn json_compare(
        &self,
        field: &str,
        operator: ComparisonOperator,
        value: serde_json::Value,
    ) -> Result<Self::Predicate> {
        let _ = (operator, value);
        bail!("JSONB comparison is not supported for `{field}`")
    }
    fn json_contains(&self, field: &str, value: serde_json::Value) -> Result<Self::Predicate> {
        let _ = value;
        bail!("JSONB containment is not supported for `{field}`")
    }
    fn json_present(&self, field: &str) -> Result<Self::Predicate> {
        bail!("JSONB presence is not supported for `{field}`")
    }
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
                let predicate = compiler.like(first, pattern.clone())?;
                fields.try_fold(predicate, |left, field| {
                    Ok(compiler.or(left, compiler.like(field, pattern.clone())?))
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
    let field_type = compiler.field_type(field)?;
    if operator == Comparator::Has && !value.quoted && value.text == "*" {
        // Documented scalar presence extension: NULL is absent; nonnullable
        // scalar columns are always present, including empty strings and zero.
        return match field_type {
            FieldType::String { nullable: true } => {
                compiler.compare(field, ComparisonOperator::NotEqual, FilterLiteral::Null)
            }
            FieldType::Jsonb => compiler.json_present(field),
            _ => Ok(compiler.constant(true)),
        };
    }
    if matches!(field_type, FieldType::Jsonb) {
        let json = if value.quoted {
            if value.text.starts_with('[') || value.text.starts_with('{') {
                serde_json::from_str(&value.text)
                    .map_err(|error| anyhow::anyhow!("invalid JSON containment value: {error}"))?
            } else {
                serde_json::Value::String(value.text.clone())
            }
        } else {
            serde_json::from_str(&value.text)
                .unwrap_or_else(|_| serde_json::Value::String(value.text.clone()))
        };
        return if operator == Comparator::Has {
            compiler.json_contains(field, json)
        } else {
            let operator = match operator {
                Comparator::Equal => ComparisonOperator::Equal,
                Comparator::NotEqual => ComparisonOperator::NotEqual,
                Comparator::LessThan => ComparisonOperator::LessThan,
                Comparator::LessThanOrEqual => ComparisonOperator::LessThanOrEqual,
                Comparator::GreaterThan => ComparisonOperator::GreaterThan,
                Comparator::GreaterThanOrEqual => ComparisonOperator::GreaterThanOrEqual,
                Comparator::Has => unreachable!(),
            };
            compiler.json_compare(field, operator, json)
        };
    }
    let value = match field_type {
        FieldType::String { nullable } => {
            if !value.quoted && value.text == "null" && nullable {
                ensure!(
                    matches!(operator, Comparator::Equal | Comparator::NotEqual),
                    "null supports only = and != comparisons"
                );
                FilterLiteral::Null
            } else {
                if operator == Comparator::Has {
                    return compiler.like(field, substring_pattern(&value.text));
                }
                if matches!(operator, Comparator::Equal | Comparator::NotEqual)
                    && value.text.contains('*')
                {
                    let predicate = compiler.like(field, wildcard_pattern(&value.text))?;
                    return Ok(if operator == Comparator::NotEqual {
                        compiler.not(predicate)
                    } else {
                        predicate
                    });
                }
                FilterLiteral::String(value.text.clone())
            }
        }
        FieldType::Int32 => {
            ensure!(
                !value.quoted,
                "field `{field}` requires an unquoted signed 32-bit integer"
            );
            let number = value.text.parse::<i32>().map_err(|_| {
                anyhow::anyhow!(
                    "field `{field}` requires a signed 32-bit integer, received `{}`",
                    value.text
                )
            })?;
            FilterLiteral::Int(number)
        }
        FieldType::Jsonb => unreachable!(),
    };
    let operator = match operator {
        Comparator::Equal | Comparator::Has => ComparisonOperator::Equal,
        Comparator::NotEqual => ComparisonOperator::NotEqual,
        Comparator::LessThan => ComparisonOperator::LessThan,
        Comparator::LessThanOrEqual => ComparisonOperator::LessThanOrEqual,
        Comparator::GreaterThan => ComparisonOperator::GreaterThan,
        Comparator::GreaterThanOrEqual => ComparisonOperator::GreaterThanOrEqual,
    };
    compiler.compare(field, operator, value)
}

fn wildcard_pattern(value: &str) -> String {
    let mut pattern = String::new();
    for character in value.chars() {
        match character {
            '*' => pattern.push('%'),
            '%' | '_' | '\\' => {
                pattern.push('\\');
                pattern.push(character);
            }
            _ => pattern.push(character),
        }
    }
    pattern
}

fn substring_pattern(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records backend operations without depending on Diesel or an application schema.
    #[derive(Debug, PartialEq, Eq)]
    enum Predicate {
        Compare(String, ComparisonOperator, FilterLiteral),
        Like(String, String),
        Constant(bool),
        And(Box<Predicate>, Box<Predicate>),
        Or(Box<Predicate>, Box<Predicate>),
        Not(Box<Predicate>),
    }

    struct TestCompiler {
        text_fields: &'static [&'static str],
    }

    impl Default for TestCompiler {
        fn default() -> Self {
            Self {
                text_fields: &["name", "nickname"],
            }
        }
    }

    impl Aip160FilterCompiler for TestCompiler {
        type Predicate = Predicate;

        fn compare(
            &self,
            field: &str,
            operator: ComparisonOperator,
            value: FilterLiteral,
        ) -> Result<Predicate> {
            Ok(Predicate::Compare(field.to_owned(), operator, value))
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
                "name" => Ok(FieldType::String { nullable: false }),
                "nickname" => Ok(FieldType::String { nullable: true }),
                "age" => Ok(FieldType::Int32),
                _ => bail!("unknown test field `{field}`"),
            }
        }

        fn text_fields(&self) -> &'static [&'static str] {
            self.text_fields
        }

        fn like(&self, field: &str, pattern: String) -> Result<Predicate> {
            Ok(Predicate::Like(field.to_owned(), pattern))
        }
    }

    fn compiled(source: &str) -> Predicate {
        compile(source, &TestCompiler::default()).unwrap().unwrap()
    }

    fn comparison(field: &str, operator: ComparisonOperator, value: FilterLiteral) -> Predicate {
        Predicate::Compare(field.to_owned(), operator, value)
    }

    fn equal(field: &str, value: &str) -> Predicate {
        comparison(
            field,
            ComparisonOperator::Equal,
            FilterLiteral::String(value.to_owned()),
        )
    }

    fn like(field: &str, pattern: &str) -> Predicate {
        Predicate::Like(field.to_owned(), pattern.to_owned())
    }

    #[test]
    fn comparisons_dispatch_typed_values_and_operators() {
        for (token, operator) in [
            ("=", ComparisonOperator::Equal),
            ("!=", ComparisonOperator::NotEqual),
            ("<", ComparisonOperator::LessThan),
            ("<=", ComparisonOperator::LessThanOrEqual),
            (">", ComparisonOperator::GreaterThan),
            (">=", ComparisonOperator::GreaterThanOrEqual),
        ] {
            for field in ["name", "nickname"] {
                assert_eq!(
                    compiled(&format!("{field}{token}Alice")),
                    comparison(field, operator, FilterLiteral::String("Alice".into()))
                );
            }
            assert_eq!(
                compiled(&format!("age{token}30")),
                comparison("age", operator, FilterLiteral::Int(30))
            );
        }
    }

    #[test]
    fn precedence_and_negation_produce_the_expected_backend_tree() {
        let expected = Predicate::And(
            Box::new(equal("name", "A")),
            Box::new(Predicate::Or(
                Box::new(equal("nickname", "B")),
                Box::new(equal("name", "C")),
            )),
        );
        assert_eq!(compiled("name=A AND nickname=B OR name=C"), expected);
        assert_eq!(compiled("name=A nickname=B OR name=C"), expected);
        assert_eq!(
            compiled("NOT (name=A OR nickname=B)"),
            Predicate::Not(Box::new(Predicate::Or(
                Box::new(equal("name", "A")),
                Box::new(equal("nickname", "B"))
            )))
        );
        for source in ["-name=A", "NOT name=A"] {
            assert_eq!(
                compiled(source),
                Predicate::Not(Box::new(equal("name", "A")))
            );
        }
    }

    #[test]
    fn composite_arguments_apply_the_comparison_to_each_literal() {
        assert_eq!(
            compiled("name=(A OR B)"),
            Predicate::Or(Box::new(equal("name", "A")), Box::new(equal("name", "B")))
        );
        assert_eq!(
            compiled("name:(Central AND (Station OR Depot))"),
            Predicate::And(
                Box::new(like("name", "%Central%")),
                Box::new(Predicate::Or(
                    Box::new(like("name", "%Station%")),
                    Box::new(like("name", "%Depot%"))
                )),
            )
        );
        assert_eq!(
            compiled("name=(NOT A)"),
            Predicate::Not(Box::new(equal("name", "A")))
        );
    }

    #[test]
    fn null_and_presence_checks_dispatch_the_expected_operations() {
        assert_eq!(
            compiled("nickname=null"),
            comparison("nickname", ComparisonOperator::Equal, FilterLiteral::Null)
        );
        for source in ["nickname!=null", "nickname:*"] {
            assert_eq!(
                compiled(source),
                comparison(
                    "nickname",
                    ComparisonOperator::NotEqual,
                    FilterLiteral::Null
                )
            );
        }
        for source in ["name:*", "age:*"] {
            assert_eq!(compiled(source), Predicate::Constant(true));
        }
    }

    #[test]
    fn strings_preserve_literal_spelling_and_quoting() {
        for (source, field, value) in [
            ("name=001", "name", "001"),
            ("name=true", "name", "true"),
            ("name=null", "name", "null"),
            ("nickname='null'", "nickname", "null"),
            ("name=nickname", "name", "nickname"),
            ("name=a.b", "name", "a.b"),
            (r#"name="x' OR 1=1 --""#, "name", "x' OR 1=1 --"),
        ] {
            assert_eq!(compiled(source), equal(field, value));
        }
    }

    #[test]
    fn like_patterns_distinguish_wildcards_from_literal_substrings() {
        for (source, pattern) in [
            ("name='Central*'", "Central%"),
            ("name='*Central'", "%Central"),
            ("name='*Central*'", "%Central%"),
            ("name:Central", "%Central%"),
            ("name:'*'", "%*%"),
            ("name:'50%_off'", "%50\\%\\_off%"),
            (r#"name="a%_*b\\c""#, "a\\%\\_%b\\\\c"),
        ] {
            assert_eq!(compiled(source), like("name", pattern));
        }
        assert_eq!(
            compiled("name!='Central*'"),
            Predicate::Not(Box::new(like("name", "Central%")))
        );
    }

    #[test]
    fn global_search_uses_only_backend_declared_text_fields() {
        for literal in [
            "Central",
            "true",
            "false",
            "null",
            "001",
            "2.997e9",
            "some.domain",
        ] {
            let expected = Predicate::Or(
                Box::new(like("name", &format!("%{literal}%"))),
                Box::new(like("nickname", &format!("%{literal}%"))),
            );
            assert_eq!(compiled(literal), expected);
            assert_eq!(compiled(&format!("'{literal}'")), expected);
        }
        let backend = TestCompiler {
            text_fields: &["nickname"],
        };
        assert_eq!(
            compile("Central", &backend).unwrap(),
            Some(like("nickname", "%Central%"))
        );
        let backend = TestCompiler { text_fields: &[] };
        assert_eq!(
            compile("Central", &backend).unwrap_err().to_string(),
            "global search is not supported"
        );
    }

    #[test]
    fn integer_conversion_is_exact_and_range_checked() {
        for (literal, value) in [
            ("-2147483648", i32::MIN),
            ("2147483647", i32::MAX),
            ("0", 0),
            ("001", 1),
            ("-30", -30),
        ] {
            assert_eq!(
                compiled(&format!("age={literal}")),
                comparison("age", ComparisonOperator::Equal, FilterLiteral::Int(value))
            );
        }
        assert_eq!(
            compiled("age:30"),
            comparison("age", ComparisonOperator::Equal, FilterLiteral::Int(30))
        );
        for value in [
            "2147483648",
            "-2147483649",
            "9007199254740993",
            "1.5",
            "1e3",
            "true",
            "null",
            "NaN",
            "'30'",
        ] {
            assert!(
                compile(&format!("age={value}"), &TestCompiler::default()).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn invalid_fields_values_and_unsupported_syntax_are_rejected() {
        for source in [
            "unknown=A",
            "point.x=1",
            "resource.name=A",
            "name.0=A",
            "'name'=A",
            "42=name",
            "regex(name, 'a')",
            "name=helper()",
            "name=(other=A)",
            "nickname>null",
            "nickname:null",
            "name == 'A'",
            "name=A && nickname=B",
            "name.contains('A')",
        ] {
            assert!(
                compile(source, &TestCompiler::default()).is_err(),
                "unexpectedly accepted: {source}"
            );
        }
        assert_eq!(
            compile("missing:*", &TestCompiler::default())
                .unwrap_err()
                .to_string(),
            "unknown test field `missing`"
        );
    }

    #[test]
    fn empty_filters_return_no_predicate() {
        for source in ["", " \n "] {
            assert_eq!(compile(source, &TestCompiler::default()).unwrap(), None);
        }
    }
}
