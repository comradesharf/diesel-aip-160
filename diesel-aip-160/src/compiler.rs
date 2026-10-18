//! AIP-160 AST validation and compilation into backend predicates.

use super::parser::{self, Argument, Comparable, Comparator, Expression, Value};
use anyhow::{Result, bail, ensure};

/// Scalar comparison delivered to custom field adapters; `:` maps to `Equal`.
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
    /// Schema- or type-driven comparisons, including built-ins and enum wrappers.
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
    /// Searchable field names. Include explicitly registered custom search
    /// fields here as well as ordinary text fields. Nontext containment fields
    /// must be omitted unless they deliberately opt into a search adapter.
    fn text_fields(&self) -> &'static [&'static str];
    /// Search a registered field using the original literal, including quoting.
    /// The default is an escaped, case-insensitive substring match. Compilation
    /// routes `:` here only for registered searchable fields; unquoted `*` and
    /// `null` retain their existing presence/comparison dispatch.
    fn field_search(&self, field: &str, value: &Value) -> Result<Self::Predicate> {
        self.ilike(field, substring_pattern(&value.text))
    }

    /// Search a global term. By default, OR together each registered text field's
    /// `field_search`, so field-specific search configuration also applies here.
    fn global_search(&self, value: &Value) -> Result<Self::Predicate> {
        let mut fields = self.text_fields().iter();
        let first = fields
            .next()
            .ok_or_else(|| anyhow::anyhow!("global search is not supported"))?;
        fields.try_fold(self.field_search(first, value)?, |left, field| {
            Ok(self.or(left, self.field_search(field, value)?))
        })
    }

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

/// A predicate and the independent ranking contributions produced by an operation.
pub struct QueryFilter<P, S> {
    /// Predicate truth is independent of whether the caller selects an ordering.
    pub predicate: P,
    /// Independent contributions; generic composition does not normalize metrics.
    pub signals: Vec<S>,
}

impl<P, S> QueryFilter<P, S> {
    /// Construct a predicate-only result, with no ranking contributions.
    pub fn predicate(predicate: P) -> Self {
        Self {
            predicate,
            signals: Vec::new(),
        }
    }
    fn combine(self, other: Self, combine: impl FnOnce(P, P) -> P) -> Self {
        let mut signals = self.signals;
        signals.extend(other.signals);
        Self {
            predicate: combine(self.predicate, other.predicate),
            signals,
        }
    }
}

/// Opt-in extension of the existing compiler. Default hooks preserve existing
/// predicates and emit no signals. Composition retains positive leaf signals,
/// including leaves in unmatched OR branches. An odd number of NOTs suppresses
/// a signal; double negation restores it. Adapters own metric semantics.
pub trait QueryFilterCompiler: Aip160FilterCompiler {
    /// Application-selected metric representation. Diesel derives use `RankingSignal`.
    type Signal;
    /// Compile one registered field search; defaults to predicate-only behavior.
    fn query_field_search(
        &self,
        field: &str,
        value: &Value,
    ) -> Result<QueryFilter<Self::Predicate, Self::Signal>> {
        self.field_search(field, value).map(QueryFilter::predicate)
    }
    /// Compile one bare term. The default preserves `global_search` overrides.
    /// Use `query_global_fields` to collect ranked searches from registered fields.
    fn query_global_search(
        &self,
        value: &Value,
    ) -> Result<QueryFilter<Self::Predicate, Self::Signal>> {
        self.global_search(value).map(QueryFilter::predicate)
    }
    /// Compile a custom comparison after shared presence/NULL validation.
    fn query_custom_compare(
        &self,
        field: &str,
        operator: ComparisonOperator,
        value: &Value,
    ) -> Result<QueryFilter<Self::Predicate, Self::Signal>> {
        self.custom_compare(field, operator, value)
            .map(QueryFilter::predicate)
    }
    /// Apply an operation-specific policy after negative contributions are removed.
    /// The default retains independent signals exactly as returned by the hooks.
    fn finish_signals(&self, signals: Vec<Self::Signal>) -> Result<Vec<Self::Signal>> {
        Ok(signals)
    }
}

/// Expand a global search using the same registered fields as predicate compilation.
pub fn query_global_fields<C: QueryFilterCompiler>(
    compiler: &C,
    value: &Value,
) -> Result<QueryFilter<C::Predicate, C::Signal>> {
    let mut fields = compiler.text_fields().iter();
    let first = fields
        .next()
        .ok_or_else(|| anyhow::anyhow!("global search is not supported"))?;
    fields.try_fold(compiler.query_field_search(first, value)?, |left, field| {
        let right = compiler.query_field_search(field, value)?;
        Ok(left.combine(right, |a, b| compiler.or(a, b)))
    })
}

/// Parse and validate once through the existing compiler, carrying signals with
/// each predicate. No model registration or second AST walk is needed.
pub fn compile_query<C: QueryFilterCompiler>(
    source: &str,
    compiler: &C,
) -> Result<Option<QueryFilter<C::Predicate, C::Signal>>> {
    compile(source, &WithSignals(compiler))?
        .map(|result| {
            let signals = result
                .signals
                .into_iter()
                .filter_map(|(positive, signal)| positive.then_some(signal))
                .collect();
            Ok(QueryFilter {
                predicate: result.predicate,
                signals: compiler.finish_signals(signals)?,
            })
        })
        .transpose()
}

struct WithSignals<'a, C>(&'a C);
impl<C: QueryFilterCompiler> Aip160FilterCompiler for WithSignals<'_, C> {
    type Predicate = QueryFilter<C::Predicate, (bool, C::Signal)>;
    fn native_compare(&self, f: &str, o: Comparator, v: &Value) -> Result<Self::Predicate> {
        self.0.native_compare(f, o, v).map(QueryFilter::predicate)
    }
    fn custom_compare(&self, f: &str, o: ComparisonOperator, v: &Value) -> Result<Self::Predicate> {
        self.0.query_custom_compare(f, o, v).map(mark_positive)
    }
    fn field_search(&self, f: &str, v: &Value) -> Result<Self::Predicate> {
        self.0.query_field_search(f, v).map(mark_positive)
    }
    fn global_search(&self, v: &Value) -> Result<Self::Predicate> {
        self.0.query_global_search(v).map(mark_positive)
    }
    fn constant(&self, v: bool) -> Self::Predicate {
        QueryFilter::predicate(self.0.constant(v))
    }
    fn field_type(&self, f: &str) -> Result<FieldType> {
        self.0.field_type(f)
    }
    fn text_fields(&self) -> &'static [&'static str] {
        self.0.text_fields()
    }
    fn ilike(&self, f: &str, p: String) -> Result<Self::Predicate> {
        self.0.ilike(f, p).map(QueryFilter::predicate)
    }
    fn and(&self, a: Self::Predicate, b: Self::Predicate) -> Self::Predicate {
        a.combine(b, |left, right| self.0.and(left, right))
    }
    fn or(&self, a: Self::Predicate, b: Self::Predicate) -> Self::Predicate {
        a.combine(b, |left, right| self.0.or(left, right))
    }
    fn not(&self, mut a: Self::Predicate) -> Self::Predicate {
        a.predicate = self.0.not(a.predicate);
        for (positive, _) in &mut a.signals {
            *positive = !*positive;
        }
        a
    }
}
fn mark_positive<P, S>(q: QueryFilter<P, S>) -> QueryFilter<P, (bool, S)> {
    QueryFilter {
        predicate: q.predicate,
        signals: q.signals.into_iter().map(|s| (true, s)).collect(),
    }
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
                // Bare true, false and null remain search terms.
                compiler.global_search(&value)
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
    // Presence and NULL validation stay on the comparison path. Nontext
    // containment never enters the search hooks.
    if operator == Comparator::Has
        && (value.quoted || (value.text != "*" && value.text != "null"))
        && compiler.text_fields().contains(&field)
    {
        return compiler.field_search(field, value);
    }
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

#[doc(hidden)]
pub fn search_pattern(value: &str) -> String {
    substring_pattern(value)
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
        SearchValue(String, Value),
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

    struct SearchCompiler {
        global_override: bool,
    }

    impl Aip160FilterCompiler for SearchCompiler {
        type Predicate = Predicate;
        fn native_compare(&self, field: &str, op: Comparator, value: &Value) -> Result<Predicate> {
            TestCompiler(&[]).native_compare(field, op, value)
        }
        fn field_type(&self, field: &str) -> Result<FieldType> {
            TestCompiler(&[]).field_type(field)
        }
        fn text_fields(&self) -> &'static [&'static str] {
            &["name", "optional"]
        }
        fn ilike(&self, field: &str, pattern: String) -> Result<Predicate> {
            TestCompiler(&[]).ilike(field, pattern)
        }
        fn field_search(&self, field: &str, value: &Value) -> Result<Predicate> {
            // Record the raw value instead of converting it to a LIKE pattern.
            Ok(Predicate::SearchValue(field.into(), value.clone()))
        }
        fn global_search(&self, value: &Value) -> Result<Predicate> {
            if self.global_override {
                return self.field_search("document", value);
            }
            // Exercise the actual trait default through a wrapper without an override.
            compile_default_global(self, value)
        }
        fn constant(&self, value: bool) -> Predicate {
            TestCompiler(&[]).constant(value)
        }
        fn and(&self, a: Predicate, b: Predicate) -> Predicate {
            TestCompiler(&[]).and(a, b)
        }
        fn or(&self, a: Predicate, b: Predicate) -> Predicate {
            TestCompiler(&[]).or(a, b)
        }
        fn not(&self, a: Predicate) -> Predicate {
            TestCompiler(&[]).not(a)
        }
    }

    fn compile_default_global(compiler: &SearchCompiler, value: &Value) -> Result<Predicate> {
        struct DefaultGlobal<'a>(&'a SearchCompiler);
        impl Aip160FilterCompiler for DefaultGlobal<'_> {
            type Predicate = Predicate;
            fn native_compare(&self, f: &str, o: Comparator, v: &Value) -> Result<Predicate> {
                self.0.native_compare(f, o, v)
            }
            fn field_type(&self, f: &str) -> Result<FieldType> {
                self.0.field_type(f)
            }
            fn text_fields(&self) -> &'static [&'static str] {
                self.0.text_fields()
            }
            fn ilike(&self, f: &str, p: String) -> Result<Predicate> {
                self.0.ilike(f, p)
            }
            fn field_search(&self, f: &str, v: &Value) -> Result<Predicate> {
                self.0.field_search(f, v)
            }
            fn constant(&self, v: bool) -> Predicate {
                self.0.constant(v)
            }
            fn and(&self, a: Predicate, b: Predicate) -> Predicate {
                self.0.and(a, b)
            }
            fn or(&self, a: Predicate, b: Predicate) -> Predicate {
                self.0.or(a, b)
            }
            fn not(&self, a: Predicate) -> Predicate {
                self.0.not(a)
            }
        }
        DefaultGlobal(compiler).global_search(value)
    }

    fn searched(field: &str, text: &str, quoted: bool) -> Predicate {
        Predicate::SearchValue(field.into(), value(text, quoted))
    }

    #[test]
    fn search_hooks_preserve_values_override_global_and_compose() {
        let compiler = SearchCompiler {
            global_override: true,
        };
        let run = |s| compile(s, &compiler).unwrap().unwrap();
        for (source, text, quoted) in [
            ("name:001", "001", false),
            ("name:'diesel engine'", "diesel engine", true),
            ("name:'50%_*'", "50%_*", true),
            ("name:'null'", "null", true),
            ("name:'*'", "*", true),
        ] {
            assert_eq!(run(source), searched("name", text, quoted));
        }
        assert_eq!(
            run("'diesel engine'"),
            searched("document", "diesel engine", true)
        );
        assert_eq!(
            run("name:diesel engine"),
            Predicate::And(
                Box::new(searched("name", "diesel", false)),
                Box::new(searched("document", "engine", false))
            )
        );
        assert_eq!(
            run("NOT name:(diesel OR 'engine room') AND age>=001"),
            Predicate::And(
                Box::new(Predicate::Not(Box::new(Predicate::Or(
                    Box::new(searched("name", "diesel", false)),
                    Box::new(searched("name", "engine room", true))
                )))),
                Box::new(native("age", Comparator::GreaterThanOrEqual, "001", false))
            )
        );
        let compiler = SearchCompiler {
            global_override: false,
        };
        assert_eq!(
            compile("'diesel engine'", &compiler).unwrap(),
            Some(Predicate::Or(
                Box::new(searched("name", "diesel engine", true)),
                Box::new(searched("optional", "diesel engine", true))
            ))
        );
        // Presence, NULL and nontext containment bypass field search.
        for source in ["name:*", "name:null", "name='a*'", "payload.details:diesel"] {
            assert_eq!(run(source), compiled(source));
        }
    }

    #[test]
    fn query_compilation_dispatches_each_operation_once_and_keeps_boolean_truth() {
        use std::cell::Cell;
        struct Counting(Cell<usize>);
        impl Aip160FilterCompiler for Counting {
            type Predicate = Predicate;
            fn native_compare(&self, f: &str, o: Comparator, v: &Value) -> Result<Predicate> {
                self.0.set(self.0.get() + 1);
                TestCompiler(&[]).native_compare(f, o, v)
            }
            fn field_type(&self, f: &str) -> Result<FieldType> {
                TestCompiler(&[]).field_type(f)
            }
            fn text_fields(&self) -> &'static [&'static str] {
                &["name"]
            }
            fn ilike(&self, _: &str, _: String) -> Result<Predicate> {
                panic!("ranked operation must not compile its predicate a second time")
            }
            fn constant(&self, v: bool) -> Predicate {
                TestCompiler(&[]).constant(v)
            }
            fn and(&self, a: Predicate, b: Predicate) -> Predicate {
                TestCompiler(&[]).and(a, b)
            }
            fn or(&self, a: Predicate, b: Predicate) -> Predicate {
                TestCompiler(&[]).or(a, b)
            }
            fn not(&self, a: Predicate) -> Predicate {
                TestCompiler(&[]).not(a)
            }
        }
        impl QueryFilterCompiler for Counting {
            type Signal = String;
            fn query_field_search(
                &self,
                f: &str,
                v: &Value,
            ) -> Result<QueryFilter<Predicate, String>> {
                self.0.set(self.0.get() + 1);
                Ok(QueryFilter {
                    predicate: TestCompiler(&[]).field_search(f, v)?,
                    signals: vec![v.text.clone()],
                })
            }
            fn query_global_search(&self, v: &Value) -> Result<QueryFilter<Predicate, String>> {
                query_global_fields(self, v)
            }
        }
        let source = "name:(diesel OR NOT engine) AND age>=3 AND NOT (NOT manual)";
        let compiler = Counting(Cell::new(0));
        let result = compile_query(source, &compiler).unwrap().unwrap();
        assert_eq!(compiler.0.get(), 4);
        assert_eq!(result.signals, ["diesel", "manual"]);
        assert_eq!(result.predicate, compiled(source));
        assert!(compile_query("name=(age=3)", &compiler).is_err());
        assert!(compile_query("", &compiler).unwrap().is_none());
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
