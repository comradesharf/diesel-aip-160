//! Schema-driven native filtering. SQL names originate only from sealed Diesel
//! type implementations; literals always travel as bind parameters.
use crate::compiler::{FieldType, substring_pattern};
use crate::diesel_helpers::Predicate;
use crate::parser::{Comparator, Value};
#[cfg(not(feature = "postgres"))]
use anyhow::bail;
use anyhow::{Result, ensure};
use diesel::expression::{
    AppearsOnTable, Expression, SelectableExpression, ValidGrouping, is_aggregate::No,
};
#[cfg(feature = "mysql")]
use diesel::mysql::Mysql as Backend;
#[cfg(feature = "postgres")]
use diesel::pg::Pg as Backend;
use diesel::query_builder::{AstPass, QueryFragment, QueryId};
use diesel::sql_types::{self as st, SqlType};
#[cfg(feature = "sqlite")]
use diesel::sqlite::Sqlite as Backend;
use diesel::{Column, QueryResult};

#[doc(hidden)]
#[derive(Clone, Debug)]
pub enum Kind {
    Text,
    Bool,
    Signed(u8),
    Unsigned(u8),
    Float(u8),
    Numeric,
    Binary,
    Date,
    Time,
    Timestamp,
    Timestamptz,
    Json,
    Jsonb,
    Uuid,
    Network,
    Mac(usize),
    Lsn,
    Char,
    Other,
    Array(Box<Type>),
    Range(Box<Type>),
    Multirange(Box<Type>),
    Record(Vec<Type>),
}
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct Type {
    pub name: String,
    pub kind: Kind,
    pub nullable: bool,
    pub unsigned: bool,
}
impl Type {
    fn new(name: &str, kind: Kind) -> Self {
        Self {
            name: name.into(),
            kind,
            nullable: false,
            unsigned: false,
        }
    }
}
mod sealed {
    pub trait Sealed {}
}
/// Implemented for Diesel's built-in SQL types. User-defined types use adapters.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "unsupported Diesel SQL type `{Self}`; use #[aip160(adapter = MyAdapter)] or #[aip160(skip)] for application-defined SQL types"
)]
pub trait NativeSqlType: SqlType + sealed::Sealed {
    fn native_type() -> Type;
}
#[doc(hidden)]
pub trait NativeType<T>: NativeSqlType {
    fn normalize(text: String) -> Result<String> {
        Ok(text)
    }
    fn path(field: &str, column: &str) -> Option<Vec<String>> {
        (field == column).then(Vec::new)
    }
}
macro_rules! scalar {
    ($sql:ty, $name:expr, $kind:expr) => {
        impl sealed::Sealed for $sql {}
        impl NativeSqlType for $sql {
            fn native_type() -> Type {
                Type::new($name, $kind)
            }
        }
        impl<T> NativeType<T> for $sql {}
    };
}
scalar!(st::Text, "text", Kind::Text);
scalar!(st::Bool, "boolean", Kind::Bool);
scalar!(st::SmallInt, "smallint", Kind::Signed(16));
scalar!(st::Integer, "integer", Kind::Signed(32));
scalar!(st::BigInt, "bigint", Kind::Signed(64));
scalar!(st::Float, "real", Kind::Float(32));
scalar!(st::Double, "double precision", Kind::Float(64));
scalar!(st::Numeric, "numeric", Kind::Numeric);
scalar!(st::Binary, "bytea", Kind::Binary);
#[cfg(not(feature = "sqlite"))]
scalar!(st::Date, "date", Kind::Date);
#[cfg(feature = "sqlite")]
impl sealed::Sealed for st::Date {}
#[cfg(feature = "sqlite")]
impl NativeSqlType for st::Date {
    fn native_type() -> Type {
        Type::new("date", Kind::Date)
    }
}
#[cfg(feature = "sqlite")]
impl<T: SqliteTemporal> NativeType<T> for st::Date {
    fn normalize(text: String) -> Result<String> {
        T::canonical(&text)
    }
}
#[cfg(not(feature = "sqlite"))]
scalar!(st::Time, "time", Kind::Time);
#[cfg(feature = "sqlite")]
impl sealed::Sealed for st::Time {}
#[cfg(feature = "sqlite")]
impl NativeSqlType for st::Time {
    fn native_type() -> Type {
        Type::new("time", Kind::Time)
    }
}
#[cfg(feature = "sqlite")]
impl<T: SqliteTemporal> NativeType<T> for st::Time {
    fn normalize(text: String) -> Result<String> {
        T::canonical(&text)
    }
}
#[cfg(not(feature = "sqlite"))]
scalar!(st::Timestamp, "timestamp", Kind::Timestamp);
#[cfg(feature = "sqlite")]
impl sealed::Sealed for st::Timestamp {}
#[cfg(feature = "sqlite")]
impl NativeSqlType for st::Timestamp {
    fn native_type() -> Type {
        Type::new("timestamp", Kind::Timestamp)
    }
}
#[cfg(feature = "sqlite")]
impl<T: SqliteTemporal> NativeType<T> for st::Timestamp {
    fn normalize(text: String) -> Result<String> {
        T::canonical(&text)
    }
}
scalar!(st::Json, "json", Kind::Json);
#[cfg(feature = "postgres")]
scalar!(st::Interval, "interval", Kind::Other);
#[cfg(feature = "mysql")]
scalar!(st::TinyInt, "tinyint", Kind::Signed(8));
#[cfg(feature = "sqlite")]
impl sealed::Sealed for st::TimestamptzSqlite {}
#[cfg(feature = "sqlite")]
impl NativeSqlType for st::TimestamptzSqlite {
    fn native_type() -> Type {
        Type::new("text", Kind::Timestamptz)
    }
}
#[cfg(feature = "sqlite")]
impl<T: SqliteTemporal> NativeType<T> for st::TimestamptzSqlite {
    fn normalize(text: String) -> Result<String> {
        T::canonical(&text)
    }
}
#[cfg(feature = "sqlite")]
scalar!(st::Jsonb, "jsonb", Kind::Json);
#[cfg(feature = "mysql")]
scalar!(st::Datetime, "datetime", Kind::Timestamp);
#[cfg(feature = "postgres")]
mod pg {
    use super::*;
    scalar!(st::Uuid, "uuid", Kind::Uuid);
    scalar!(st::Timestamptz, "timestamptz", Kind::Timestamptz);
    scalar!(st::Oid, "oid", Kind::Unsigned(32));
    scalar!(st::Money, "money", Kind::Other);
    scalar!(st::Inet, "inet", Kind::Network);
    scalar!(st::Cidr, "cidr", Kind::Network);
    scalar!(st::MacAddr, "macaddr", Kind::Mac(6));
    scalar!(st::MacAddr8, "macaddr8", Kind::Mac(8));
    scalar!(st::CChar, "\"char\"", Kind::Char);
    scalar!(st::Citext, "citext", Kind::Text);
    scalar!(st::PgLsn, "pg_lsn", Kind::Lsn);
    impl sealed::Sealed for st::Jsonb {}
    impl NativeSqlType for st::Jsonb {
        fn native_type() -> Type {
            Type::new("jsonb", Kind::Jsonb)
        }
    }
    impl<T: crate::JsonbPath> NativeType<T> for st::Jsonb {
        fn path(field: &str, column: &str) -> Option<Vec<String>> {
            crate::json_path::<T>(field, column)
        }
    }
    macro_rules! container {
        ($sql:ident, $kind:ident, $name:expr) => {
            impl<S: NativeSqlType + 'static> sealed::Sealed for st::$sql<S> {}
            impl<S: NativeSqlType + 'static> NativeSqlType for st::$sql<S> {
                fn native_type() -> Type {
                    let inner = S::native_type();
                    Type::new(&$name(&inner), Kind::$kind(Box::new(inner)))
                }
            }
            impl<T, S: NativeSqlType + 'static> NativeType<T> for st::$sql<S> {}
        };
    }
    container!(Array, Array, |inner: &Type| format!("{}[]", inner.name));
    fn range_name(inner: &Type, multi: bool) -> String {
        let base = match inner.name.as_str() {
            "integer" => "int4",
            "bigint" => "int8",
            "numeric" => "num",
            "date" => "date",
            "timestamp" => "ts",
            "timestamptz" => "tstz",
            _ => "unsupported",
        };
        format!("{base}{}range", if multi { "multi" } else { "" })
    }
    container!(Range, Range, |inner: &Type| range_name(inner, false));
    container!(Multirange, Multirange, |inner: &Type| range_name(
        inner, true
    ));
    pub trait TupleTypes {
        fn types() -> Vec<Type>;
    }
    macro_rules! tuple {
        ($($t:ident),+) => { impl<$($t: NativeSqlType),+> TupleTypes for ($($t,)+) {
            fn types() -> Vec<Type> { vec![$($t::native_type()),+] }
        } };
    }
    macro_rules! tuples {
        ($head:ident $(,$tail:ident)*) => { tuple!($head $(,$tail)*); tuples!($($tail),*); };
        () => {};
    }
    tuples!(
        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z, AA, AB, AC,
        AD, AE, AF
    );
    impl<S: TupleTypes + 'static> sealed::Sealed for st::Record<S> {}
    impl<S: TupleTypes + 'static> NativeSqlType for st::Record<S> {
        fn native_type() -> Type {
            Type::new("record", Kind::Record(S::types()))
        }
    }
    impl<T, S: TupleTypes + 'static> NativeType<T> for st::Record<S> {}
}
#[cfg(feature = "mysql")]
mod mysql {
    use super::*;
    macro_rules! unsigned {
        ($sql:ty, $bits:expr) => {
            impl sealed::Sealed for st::Unsigned<$sql> {}
            impl NativeSqlType for st::Unsigned<$sql> {
                fn native_type() -> Type {
                    Type::new("unsigned", Kind::Unsigned($bits))
                }
            }
            impl<T> NativeType<T> for st::Unsigned<$sql> {}
        };
    }
    unsigned!(st::TinyInt, 8);
    unsigned!(st::SmallInt, 16);
    unsigned!(st::Integer, 32);
    unsigned!(st::BigInt, 64);
    macro_rules! unsigned_decimal {
        ($sql:ty) => {
            impl sealed::Sealed for st::Unsigned<$sql> {}
            impl NativeSqlType for st::Unsigned<$sql> {
                fn native_type() -> Type {
                    let mut ty = <$sql>::native_type();
                    ty.unsigned = true;
                    ty
                }
            }
            impl<T> NativeType<T> for st::Unsigned<$sql> {}
        };
    }
    unsigned_decimal!(st::Float);
    unsigned_decimal!(st::Double);
    unsigned_decimal!(st::Numeric);
}
impl<S: NativeSqlType> sealed::Sealed for st::Nullable<S> {}
impl<S: NativeSqlType> NativeSqlType for st::Nullable<S> {
    fn native_type() -> Type {
        let mut ty = S::native_type();
        ty.nullable = true;
        ty
    }
}
impl<T, S: NativeType<T>> NativeType<T> for st::Nullable<S> {
    fn normalize(text: String) -> Result<String> {
        S::normalize(text)
    }
    fn path(field: &str, column: &str) -> Option<Vec<String>> {
        S::path(field, column)
    }
}

/// Fallback methods used by the filter derive for built-in types. Enum wrappers
/// supply inherent methods with the same names, selected at each concrete field.
#[doc(hidden)]
pub trait FilterField: Sized {
    fn __aip160_field_type<C: Column>(column: C, field: &str, name: &str) -> Option<FieldType>
    where
        C::SqlType: NativeType<Self>,
    {
        field_type::<C, Self>(column, field, name)
    }

    fn __aip160_compare<C>(
        column: C,
        field: &str,
        name: &str,
        op: Comparator,
        value: &Value,
    ) -> Result<Predicate<C::Table>>
    where
        C: Column + QueryFragment<Backend> + Send + 'static,
        C::SqlType: NativeType<Self>,
    {
        compare::<C, Self>(column, field, name, op, value)
    }

    fn __aip160_is_text<C: Column>(_: C) -> bool
    where
        C::SqlType: NativeSqlType,
    {
        is_text::<C>()
    }

    fn __aip160_like<C>(
        column: C,
        pattern: String,
        insensitive: bool,
    ) -> Result<Predicate<C::Table>>
    where
        C: Column + QueryFragment<Backend> + Send + 'static,
        C::SqlType: NativeSqlType,
    {
        like(column, pattern, insensitive)
    }
}
impl<T> FilterField for T {}

#[doc(hidden)]
pub fn field_type<C: Column, T>(_: C, field: &str, column: &str) -> Option<FieldType>
where
    C::SqlType: NativeType<T>,
{
    <C::SqlType as NativeType<T>>::path(field, column)?;
    Some(FieldType::Native)
}
#[doc(hidden)]
pub fn is_text<C: Column>() -> bool
where
    C::SqlType: NativeSqlType,
{
    matches!(C::SqlType::native_type().kind, Kind::Text)
}

#[derive(Debug)]
enum Bound {
    Text(String),
    Bytes(Vec<u8>),
    Int(i64),
    #[cfg(feature = "mysql")]
    UInt(u64),
    #[cfg(feature = "mysql")]
    Float32(f32),
    #[cfg(feature = "postgres")]
    Int32(i32),
    #[cfg(not(feature = "postgres"))]
    Float(f64),
    Null,
    Record(Vec<(Type, Bound)>),
    #[cfg(feature = "postgres")]
    Json(serde_json::Value),
    #[cfg(feature = "postgres")]
    CChar(u8),
}
#[derive(Debug)]
struct NativePredicate<C> {
    column: C,
    ty: Type,
    operator: &'static str,
    value: Bound,
    #[cfg_attr(not(feature = "postgres"), allow(dead_code))]
    path: Option<Vec<String>>,
    pattern: bool,
    insensitive: bool,
}
impl<C> Expression for NativePredicate<C> {
    type SqlType = st::Bool;
}
impl<C: Column> AppearsOnTable<C::Table> for NativePredicate<C> {}
impl<C: Column> SelectableExpression<C::Table> for NativePredicate<C> {}
impl<C> ValidGrouping<()> for NativePredicate<C> {
    type IsAggregate = No;
}
impl<C> QueryId for NativePredicate<C> {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}
impl<C: QueryFragment<Backend>> QueryFragment<Backend> for NativePredicate<C> {
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Backend>) -> QueryResult<()> {
        out.unsafe_to_cache_prepared();
        out.push_sql("(");
        #[cfg(feature = "mysql")]
        if self.operator == "JSON_CONTAINS" {
            out.push_sql("JSON_CONTAINS(");
            self.column.walk_ast(out.reborrow())?;
            out.push_sql(", ");
            bind(&self.ty, &self.value, out.reborrow())?;
            out.push_sql(")");
            out.push_sql(")");
            return Ok(());
        }
        if self.insensitive {
            out.push_sql("lower(");
        }
        #[cfg(feature = "postgres")]
        if self.path.is_some() {
            out.push_sql("(");
        }
        // PostgreSQL JSON has no comparison operators. Its JSONB conversion
        // supplies structural equality/containment without changing storage.
        #[cfg(feature = "postgres")]
        if matches!(self.ty.kind, Kind::Json) {
            out.push_sql("CAST(");
        }
        #[cfg(feature = "sqlite")]
        if matches!(self.ty.kind, Kind::Json) {
            out.push_sql("json(");
        }
        self.column.walk_ast(out.reborrow())?;
        #[cfg(feature = "sqlite")]
        if matches!(self.ty.kind, Kind::Json) {
            out.push_sql(")");
        }
        #[cfg(feature = "postgres")]
        {
            if matches!(self.ty.kind, Kind::Json) {
                out.push_sql(" AS jsonb)");
            }
            if let Some(path) = &self.path {
                out.push_sql(" #> ");
                out.push_bind_param::<st::Array<st::Text>, _>(path)?;
                out.push_sql(")");
            }
        }
        if self.insensitive {
            out.push_sql(")");
        }
        out.push_sql(self.operator);
        if !matches!(self.value, Bound::Null) {
            if self.insensitive {
                out.push_sql("lower(");
            }
            bind(&self.ty, &self.value, out.reborrow())?;
            if self.insensitive {
                out.push_sql(")");
            }
            if self.pattern {
                out.push_sql(" ESCAPE ");
                out.push_bind_param::<st::Text, _>("\\")?;
            }
        }
        out.push_sql(")");
        Ok(())
    }
}
fn bind<'b>(ty: &'b Type, value: &'b Bound, mut out: AstPass<'_, 'b, Backend>) -> QueryResult<()> {
    match value {
        Bound::Bytes(v) => out.push_bind_param::<st::Binary, _>(v)?,
        Bound::Int(v) => out.push_bind_param::<st::BigInt, _>(v)?,
        #[cfg(feature = "mysql")]
        Bound::UInt(v) => out.push_bind_param::<st::Unsigned<st::BigInt>, _>(v)?,
        #[cfg(feature = "mysql")]
        Bound::Float32(v) => out.push_bind_param::<st::Float, _>(v)?,
        #[cfg(feature = "postgres")]
        Bound::Int32(v) => out.push_bind_param::<st::Integer, _>(v)?,
        #[cfg(not(feature = "postgres"))]
        Bound::Float(v) => out.push_bind_param::<st::Double, _>(v)?,
        Bound::Null => {
            #[cfg(feature = "postgres")]
            {
                out.push_sql("CAST(NULL AS ");
                out.push_sql(&ty.name);
                out.push_sql(")");
            }
            #[cfg(not(feature = "postgres"))]
            out.push_sql("NULL");
        }
        Bound::Record(fields) => {
            out.push_sql("ROW(");
            for (i, (ty, value)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_sql(", ");
                }
                bind(ty, value, out.reborrow())?;
            }
            out.push_sql(")");
        }
        #[cfg(feature = "postgres")]
        Bound::Json(v) => out.push_bind_param::<st::Jsonb, _>(v)?,
        #[cfg(feature = "postgres")]
        Bound::CChar(v) => out.push_bind_param::<st::CChar, _>(v)?,
        Bound::Text(v) => {
            #[cfg(feature = "postgres")]
            {
                let cast = !matches!(ty.kind, Kind::Text) || ty.name != "text";
                if cast {
                    out.push_sql("CAST(");
                }
                out.push_bind_param::<st::Text, _>(v)?;
                if cast {
                    out.push_sql(" AS ");
                    out.push_sql(&ty.name);
                    out.push_sql(")");
                }
            }
            #[cfg(feature = "mysql")]
            {
                let cast = match ty.kind {
                    Kind::Numeric => Some(ty.name.as_str()),
                    Kind::Unsigned(_) => Some("UNSIGNED"),
                    Kind::Date => Some("DATE"),
                    Kind::Time => Some("TIME(6)"),
                    Kind::Timestamp => Some("DATETIME(6)"),
                    Kind::Json => Some("JSON"),
                    _ => None,
                };
                if cast.is_some() {
                    out.push_sql("CAST(");
                }
                out.push_bind_param::<st::Text, _>(v)?;
                if let Some(cast) = cast {
                    out.push_sql(" AS ");
                    out.push_sql(cast);
                    out.push_sql(")");
                }
            }
            #[cfg(feature = "sqlite")]
            {
                if matches!(ty.kind, Kind::Json) {
                    out.push_sql("json(");
                }
                out.push_bind_param::<st::Text, _>(v)?;
                if matches!(ty.kind, Kind::Json) {
                    out.push_sql(")");
                }
            }
        }
    }
    Ok(())
}
fn operator(op: Comparator) -> &'static str {
    match op {
        Comparator::Equal => " = ",
        Comparator::NotEqual => " != ",
        Comparator::LessThan => " < ",
        Comparator::LessThanOrEqual => " <= ",
        Comparator::GreaterThan => " > ",
        Comparator::GreaterThanOrEqual => " >= ",
        Comparator::Has => " @> ",
    }
}
#[doc(hidden)]
pub fn compare<C, T>(
    column: C,
    field: &str,
    name: &str,
    op: Comparator,
    value: &Value,
) -> Result<Predicate<C::Table>>
where
    C: Column + QueryFragment<Backend> + Send + 'static,
    C::SqlType: NativeType<T>,
{
    let path = <C::SqlType as NativeType<T>>::path(field, name)
        .ok_or_else(|| anyhow::anyhow!("unknown filter field `{field}`"))?;
    #[allow(unused_mut)]
    let mut ty = C::SqlType::native_type();
    let jsonb = matches!(ty.kind, Kind::Jsonb);
    #[allow(unused_mut)]
    let mut path = if jsonb && !path.is_empty() {
        Some(path)
    } else {
        None
    };
    let text = matches!(ty.kind, Kind::Text);
    let null_literal = !value.quoted && value.text == "null";
    // Required text columns retain `null` as a search string.
    if text && !(null_literal && ty.nullable) {
        if op == Comparator::Has && (value.quoted || value.text != "*") {
            return like(column, substring_pattern(&value.text), true);
        }
        if matches!(op, Comparator::Equal | Comparator::NotEqual) && value.text.contains('*') {
            let predicate = like(column, wildcard_pattern(&value.text), false)?;
            return Ok(if op == Comparator::NotEqual {
                Box::new(diesel::dsl::not(predicate))
            } else {
                predicate
            });
        }
    }
    // Scalar `:` is equality; containers retain native containment semantics.
    let op = if op == Comparator::Has
        && matches!(ty.kind, Kind::Signed(32 | 64))
        && (value.quoted || value.text != "*")
        && !null_literal
    {
        Comparator::Equal
    } else {
        op
    };
    let present = op == Comparator::Has && !value.quoted && value.text == "*";
    let null = null_literal && (!text || ty.nullable);
    let (token, bound) = if present || (null && !jsonb) || (null && path.is_none() && ty.nullable) {
        if !present {
            ensure!(ty.nullable, "field `{field}` is not nullable");
            ensure!(
                matches!(op, Comparator::Equal | Comparator::NotEqual),
                "null supports only = and != comparisons"
            );
        }
        (
            if present || op == Comparator::NotEqual {
                " IS NOT NULL"
            } else {
                " IS NULL"
            },
            Bound::Null,
        )
    } else if jsonb {
        #[cfg(feature = "postgres")]
        {
            // Keep JSON null distinct from SQL NULL, and preserve existing
            // JSONB string/object/array literal interpretation.
            let json = if value.quoted {
                if value.text.starts_with(['[', '{']) {
                    serde_json::from_str(&value.text)?
                } else {
                    serde_json::Value::String(value.text.clone())
                }
            } else {
                serde_json::from_str(&value.text)
                    .unwrap_or_else(|_| serde_json::Value::String(value.text.clone()))
            };
            // Preserve the existing root JSONB #> [] expression.
            if path.is_none() {
                path = Some(Vec::new());
            }
            (operator(op), Bound::Json(json))
        }
        #[cfg(not(feature = "postgres"))]
        {
            bail!("JSONB filtering is available only with PostgreSQL");
        }
    } else {
        if op == Comparator::Has {
            ensure!(
                matches!(
                    ty.kind,
                    Kind::Array(_) | Kind::Range(_) | Kind::Multirange(_) | Kind::Json
                ),
                "type {} does not support containment",
                ty.name
            );
            #[cfg(feature = "sqlite")]
            bail!("JSON containment is not supported on SQLite");
        }
        if matches!(ty.kind, Kind::Json) {
            ensure!(
                matches!(
                    op,
                    Comparator::Equal | Comparator::NotEqual | Comparator::Has
                ),
                "JSON supports equality and containment only"
            );
        }
        #[cfg(feature = "mysql")]
        if matches!(ty.kind, Kind::Numeric) {
            ty.name = mysql_decimal_type(&value.text)?;
        }
        let bound =
            parse(&ty, value).map_err(|e| anyhow::anyhow!("field `{field}` ({}): {e}", ty.name))?;
        let bound = match bound {
            Bound::Text(text) => Bound::Text(<C::SqlType as NativeType<T>>::normalize(text)?),
            bound => bound,
        };
        #[cfg(feature = "postgres")]
        if matches!(ty.kind, Kind::Json) {
            ty.name = "jsonb".into();
        }
        #[cfg(feature = "mysql")]
        let token = if op == Comparator::Has && matches!(ty.kind, Kind::Json) {
            "JSON_CONTAINS"
        } else {
            operator(op)
        };
        #[cfg(not(feature = "mysql"))]
        let token = operator(op);
        (token, bound)
    };
    Ok(Box::new(NativePredicate {
        column,
        ty,
        operator: token,
        value: bound,
        path,
        pattern: false,
        insensitive: false,
    }))
}
#[doc(hidden)]
pub fn like<C>(column: C, pattern: String, insensitive: bool) -> Result<Predicate<C::Table>>
where
    C: Column + QueryFragment<Backend> + Send + 'static,
    C::SqlType: NativeSqlType,
{
    let ty = C::SqlType::native_type();
    ensure!(matches!(ty.kind, Kind::Text), "field is not text");
    Ok(Box::new(NativePredicate {
        column,
        ty,
        operator: " LIKE ",
        value: Bound::Text(pattern),
        path: None,
        pattern: true,
        insensitive,
    }))
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

fn parse(ty: &Type, value: &Value) -> Result<Bound> {
    let text = &value.text;
    ensure!(
        !ty.unsigned || !text.starts_with('-'),
        "unsigned value cannot be negative"
    );
    match &ty.kind {
        Kind::Signed(bits) => {
            ensure!(
                !value.quoted,
                "requires an unquoted signed {bits}-bit integer"
            );
            let n: i64 = text.parse().map_err(|error| {
                anyhow::anyhow!("requires a signed {bits}-bit integer: {error}")
            })?;
            ensure!(
                *bits == 64 || (n >= -(1i64 << (bits - 1)) && n < (1i64 << (bits - 1))),
                "signed {bits}-bit integer overflow"
            );
            // PostgreSQL must retain exact narrow types inside records.
            #[cfg(feature = "postgres")]
            {
                Ok(match bits {
                    32 => Bound::Int32(n as i32),
                    64 => Bound::Int(n),
                    _ => Bound::Text(n.to_string()),
                })
            }
            #[cfg(not(feature = "postgres"))]
            {
                Ok(Bound::Int(n))
            }
        }
        Kind::Unsigned(bits) => {
            ensure!(
                !value.quoted,
                "requires an unquoted unsigned {bits}-bit integer"
            );
            let n: u64 = text.parse()?;
            ensure!(*bits == 64 || n < (1u64 << bits), "integer overflow");
            #[cfg(feature = "mysql")]
            {
                Ok(Bound::UInt(n))
            }
            #[cfg(not(feature = "mysql"))]
            {
                Ok(Bound::Text(n.to_string()))
            }
        }
        Kind::Bool => {
            ensure!(
                !value.quoted && matches!(text.as_str(), "true" | "false"),
                "requires unquoted true or false"
            );
            #[cfg(feature = "postgres")]
            {
                Ok(Bound::Text(text.clone()))
            }
            #[cfg(not(feature = "postgres"))]
            {
                Ok(Bound::Int(i64::from(text == "true")))
            }
        }
        Kind::Float(bits) => {
            ensure!(!value.quoted, "requires an unquoted float");
            let n: f64 = text.parse()?;
            ensure!(
                n.is_finite() && (*bits != 32 || (n as f32).is_finite()),
                "float must be finite and within range"
            );
            #[cfg(feature = "postgres")]
            {
                Ok(Bound::Text(text.clone()))
            }
            #[cfg(feature = "mysql")]
            {
                Ok(if *bits == 32 {
                    Bound::Float32(n as f32)
                } else {
                    Bound::Float(n)
                })
            }
            #[cfg(feature = "sqlite")]
            {
                Ok(Bound::Float(n))
            }
        }
        Kind::Numeric => {
            ensure!(!value.quoted, "requires an unquoted decimal");
            validate_decimal(text)?;
            #[cfg(feature = "numeric")]
            {
                let _: bigdecimal::BigDecimal = text.parse()?;
            }
            #[cfg(feature = "sqlite")]
            {
                let n: f64 = text.parse()?;
                ensure!(n.is_finite(), "numeric out of range");
                Ok(Bound::Float(n))
            }
            #[cfg(not(feature = "sqlite"))]
            {
                Ok(Bound::Text(text.clone()))
            }
        }
        Kind::Binary => {
            ensure!(value.quoted, "requires quoted hexadecimal bytes (0x...)");
            let hex = text
                .strip_prefix("0x")
                .or_else(|| text.strip_prefix("\\x"))
                .ok_or_else(|| anyhow::anyhow!("expected 0x hexadecimal prefix"))?;
            ensure!(
                hex.len() % 2 == 0 && hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid hexadecimal bytes"
            );
            Ok(Bound::Bytes(
                hex.as_bytes()
                    .chunks(2)
                    .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
                    .collect(),
            ))
        }
        Kind::Json => {
            let _: serde_json::Value = serde_json::from_str(text)?;
            Ok(Bound::Text(text.clone()))
        }
        Kind::Record(types) => {
            ensure!(value.quoted, "requires a quoted PostgreSQL record literal");
            let fields = record_fields(text)?;
            ensure!(
                fields.len() == types.len(),
                "record requires {} fields",
                types.len()
            );
            Ok(Bound::Record(
                types
                    .iter()
                    .cloned()
                    .zip(fields)
                    .map(|(ty, field)| {
                        let value = match field {
                            None => Bound::Null,
                            Some(text) => parse(
                                &ty,
                                &Value {
                                    text,
                                    quoted: !matches!(
                                        ty.kind,
                                        Kind::Signed(_)
                                            | Kind::Unsigned(_)
                                            | Kind::Float(_)
                                            | Kind::Numeric
                                            | Kind::Bool
                                    ),
                                },
                            )?,
                        };
                        Ok((ty, value))
                    })
                    .collect::<Result<_>>()?,
            ))
        }
        Kind::Date | Kind::Time | Kind::Timestamp | Kind::Timestamptz => {
            ensure!(value.quoted, "requires a quoted ISO date/time");
            validate_temporal(&ty.kind, text)?;
            Ok(Bound::Text(text.clone()))
        }
        Kind::Uuid => {
            ensure!(value.quoted, "requires a quoted UUID");
            ensure!(
                text.len() == 36
                    && text
                        .char_indices()
                        .all(|(i, c)| if [8, 13, 18, 23].contains(&i) {
                            c == '-'
                        } else {
                            c.is_ascii_hexdigit()
                        }),
                "invalid UUID"
            );
            #[cfg(feature = "uuid")]
            {
                let _: uuid::Uuid = text.parse()?;
            }
            Ok(Bound::Text(text.clone()))
        }
        Kind::Network => {
            ensure!(value.quoted, "requires a quoted network address");
            let (ip, prefix) = text
                .split_once('/')
                .map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
            let ip: std::net::IpAddr = ip.parse()?;
            if let Some(prefix) = prefix {
                let prefix: u8 = prefix.parse()?;
                ensure!(
                    prefix <= if ip.is_ipv4() { 32 } else { 128 },
                    "invalid network prefix"
                );
            }
            Ok(Bound::Text(text.clone()))
        }
        Kind::Mac(size) => {
            ensure!(value.quoted, "requires a quoted MAC address");
            let digits: String = text
                .chars()
                .filter(|c| !matches!(c, ':' | '-' | '.'))
                .collect();
            ensure!(
                digits.len() == size * 2 && digits.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid MAC address"
            );
            Ok(Bound::Text(
                digits
                    .as_bytes()
                    .chunks(2)
                    .map(|b| std::str::from_utf8(b).unwrap())
                    .collect::<Vec<_>>()
                    .join(":"),
            ))
        }
        Kind::Lsn => {
            ensure!(value.quoted, "requires a quoted WAL position");
            let (hi, lo) = text
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("invalid WAL position"))?;
            u32::from_str_radix(hi, 16)?;
            u32::from_str_radix(lo, 16)?;
            Ok(Bound::Text(text.clone()))
        }
        Kind::Char => {
            #[cfg(feature = "postgres")]
            {
                let byte = if value.quoted {
                    ensure!(
                        text.len() == 1,
                        "requires one quoted byte or an unquoted unsigned 8-bit integer"
                    );
                    text.as_bytes()[0]
                } else {
                    text.parse::<u8>()?
                };
                Ok(Bound::CChar(byte))
            }
            #[cfg(not(feature = "postgres"))]
            {
                bail!("CChar is PostgreSQL-only");
            }
        }
        Kind::Array(_) | Kind::Range(_) | Kind::Multirange(_) | Kind::Other => {
            ensure!(value.quoted, "requires a quoted PostgreSQL native literal");
            Ok(Bound::Text(text.clone()))
        }
        Kind::Text | Kind::Jsonb => Ok(Bound::Text(text.clone())),
    }
}
fn validate_decimal(s: &str) -> Result<()> {
    let s = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exp) = s
        .split_once(['e', 'E'])
        .map_or((s, None), |(a, b)| (a, Some(b)));
    let mut dot = false;
    let mut digits = 0;
    for b in mantissa.bytes() {
        if b == b'.' && !dot {
            dot = true;
        } else {
            ensure!(b.is_ascii_digit(), "invalid decimal");
            digits += 1;
        }
    }
    ensure!(digits > 0, "invalid decimal");
    if let Some(exp) = exp {
        let _: i32 = exp.parse()?;
    }
    Ok(())
}
fn validate_temporal(kind: &Kind, text: &str) -> Result<()> {
    #[cfg(feature = "chrono")]
    {
        match kind {
            Kind::Date => {
                chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")?;
            }
            Kind::Time => {
                chrono::NaiveTime::parse_from_str(text, "%H:%M:%S%.f")?;
            }
            Kind::Timestamp => {
                chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").or_else(
                    |_| chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"),
                )?;
            }
            Kind::Timestamptz => {
                chrono::DateTime::parse_from_rfc3339(text)?;
            }
            _ => unreachable!(),
        }
    }
    #[cfg(all(feature = "time", not(feature = "chrono")))]
    {
        use time::format_description::well_known::{Iso8601, Rfc3339};
        match kind {
            Kind::Date => {
                time::Date::parse(text, &Iso8601::DEFAULT)?;
            }
            Kind::Time => {
                time::Time::parse(text, &Iso8601::DEFAULT)?;
            }
            Kind::Timestamp => {
                time::PrimitiveDateTime::parse(text, &Iso8601::DEFAULT)?;
            }
            Kind::Timestamptz => {
                time::OffsetDateTime::parse(text, &Rfc3339)?;
            }
            _ => unreachable!(),
        }
    }
    #[cfg(not(any(feature = "chrono", feature = "time")))]
    {
        let _ = kind;
        ensure!(!text.is_empty(), "empty date/time literal");
    }
    Ok(())
}
// PostgreSQL composite text format: empty unquoted fields are NULL, doubled
// quotes/backslashes escape characters inside a quoted field.
fn record_fields(text: &str) -> Result<Vec<Option<String>>> {
    let body = text
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| anyhow::anyhow!("invalid record literal"))?;
    let mut fields = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut was_quoted = false;
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => value.push(
                chars
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("unfinished record escape"))?,
            ),
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                value.push('"');
            }
            '"' => {
                quoted = !quoted;
                was_quoted = true;
            }
            ',' if !quoted => {
                fields.push(if value.is_empty() && !was_quoted {
                    None
                } else {
                    Some(std::mem::take(&mut value))
                });
                was_quoted = false;
            }
            c => value.push(c),
        }
    }
    ensure!(!quoted, "unterminated record string");
    fields.push(if value.is_empty() && !was_quoted {
        None
    } else {
        Some(value)
    });
    Ok(fields)
}

#[cfg(feature = "mysql")]
fn mysql_decimal_type(text: &str) -> Result<String> {
    validate_decimal(text)?;
    let text = text.trim_start_matches(['-', '+']);
    let (mantissa, exp) = text
        .split_once(['e', 'E'])
        .map_or((text, 0), |(a, b)| (a, b.parse::<i32>().unwrap()));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let scale = (fraction.len() as i64 - i64::from(exp)).max(0);
    let integers = (whole.trim_start_matches('0').len() as i64 + i64::from(exp)).max(0);
    ensure!(
        scale <= 30 && integers + scale <= 65,
        "decimal exceeds MySQL DECIMAL precision/scale"
    );
    Ok(format!("DECIMAL(65,{scale})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_preserve_null_empty_strings_delimiters_and_escapes() {
        assert_eq!(
            record_fields(r#"(,"", "a,b","a""b","a\\b")"#).unwrap(),
            vec![
                None,
                Some("".into()),
                Some(" a,b".into()),
                Some("a\"b".into()),
                Some("a\\b".into())
            ]
        );
        assert!(record_fields("(\"unfinished)").is_err());
        assert!(record_fields("a,b").is_err());
    }
    #[test]
    fn decimals_validate_without_float_conversion() {
        for text in ["0", "-0.01", "1e+100", "12345678901234567890.123456789"] {
            validate_decimal(text).unwrap();
        }
        for text in [
            "",
            ".",
            "NaN",
            "inf",
            "1.2.3",
            "1e",
            "1e9999999999",
            "1; DROP TABLE x",
        ] {
            assert!(validate_decimal(text).is_err(), "{text}");
        }
    }
    #[cfg(any(feature = "chrono", feature = "time"))]
    #[test]
    fn temporal_formats_are_validated_with_either_integration() {
        for (kind, text) in [
            (Kind::Date, "2026-10-17"),
            (Kind::Time, "12:34:56.123456"),
            (Kind::Timestamp, "2026-10-17T12:34:56"),
            (Kind::Timestamptz, "2026-10-17T12:34:56+08:00"),
        ] {
            validate_temporal(&kind, text).unwrap();
        }
        assert!(validate_temporal(&Kind::Date, "2026-02-30").is_err());
    }
}

/// SQLite stores temporal values as text. Match Diesel's representation for
/// the model's temporal crate, including its fractional precision and UTC
/// normalization, so comparisons can keep using the column's index.
#[cfg(feature = "sqlite")]
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "enable the matching `chrono` or `time` feature on diesel-aip-160 for SQLite temporal model type `{Self}`"
)]
pub trait SqliteTemporal {
    fn canonical(text: &str) -> Result<String>;
}
#[cfg(feature = "sqlite")]
impl SqliteTemporal for String {
    fn canonical(text: &str) -> Result<String> {
        Ok(text.to_owned())
    }
}
#[cfg(all(feature = "sqlite", feature = "chrono"))]
mod sqlite_chrono {
    use super::*;
    impl SqliteTemporal for chrono::NaiveDate {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse_from_str(text, "%Y-%m-%d")?
                .format("%F")
                .to_string())
        }
    }
    impl SqliteTemporal for chrono::NaiveTime {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse_from_str(text, "%H:%M:%S%.f")?
                .format("%T%.f")
                .to_string())
        }
    }
    impl SqliteTemporal for chrono::NaiveDateTime {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| Self::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f"))
                .or_else(|_| chrono::DateTime::parse_from_rfc3339(text).map(|v| v.naive_utc()))?
                .format("%F %T%.f")
                .to_string())
        }
    }
    impl<Tz: chrono::TimeZone> SqliteTemporal for chrono::DateTime<Tz> {
        fn canonical(text: &str) -> Result<String> {
            Ok(chrono::DateTime::parse_from_rfc3339(text)?
                .with_timezone(&chrono::Utc)
                .format("%F %T%.f%:z")
                .to_string())
        }
    }
}
#[cfg(all(feature = "sqlite", feature = "time"))]
mod sqlite_time {
    use super::*;
    use time::format_description::parse_borrowed;
    use time::format_description::well_known::{Iso8601, Rfc3339};
    impl SqliteTemporal for time::Date {
        fn canonical(text: &str) -> Result<String> {
            Ok(Self::parse(text, &Iso8601::DEFAULT)?.to_string())
        }
    }
    fn format(
        date: bool,
        fraction: bool,
        offset: bool,
    ) -> Result<Vec<time::format_description::BorrowedFormatItem<'static>>> {
        let pattern = match (date, fraction, offset) {
            (false, false, _) => "[hour]:[minute]:[second]",
            (false, true, _) => "[hour]:[minute]:[second].[subsecond]",
            (true, false, false) => "[year]-[month]-[day] [hour]:[minute]:[second]",
            (true, true, false) => "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond]",
            (true, false, true) => {
                "[year]-[month]-[day] [hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
            }
            (true, true, true) => {
                "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond][offset_hour sign:mandatory]:[offset_minute]"
            }
        };
        Ok(parse_borrowed::<2>(pattern)?)
    }
    impl SqliteTemporal for time::Time {
        fn canonical(text: &str) -> Result<String> {
            let value = Self::parse(text, &Iso8601::DEFAULT)?;
            // Match Diesel 2.3's SQLite Time serializer, including its microsecond guard.
            Ok(value.format(&format(false, value.microsecond() != 0, false)?)?)
        }
    }
    impl SqliteTemporal for time::PrimitiveDateTime {
        fn canonical(text: &str) -> Result<String> {
            let value = Self::parse(&text.replace(' ', "T"), &Iso8601::DEFAULT).or_else(|_| {
                time::OffsetDateTime::parse(text, &Rfc3339).map(|value| {
                    let value = value.to_offset(time::UtcOffset::UTC);
                    time::PrimitiveDateTime::new(value.date(), value.time())
                })
            })?;
            Ok(value.format(&format(true, value.nanosecond() != 0, false)?)?)
        }
    }
    impl SqliteTemporal for time::OffsetDateTime {
        fn canonical(text: &str) -> Result<String> {
            let value = Self::parse(text, &Rfc3339)?.to_offset(time::UtcOffset::UTC);
            Ok(value.format(&format(true, value.nanosecond() != 0, true)?)?)
        }
    }
}
