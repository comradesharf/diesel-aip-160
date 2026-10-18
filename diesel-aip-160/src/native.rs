//! Schema-driven native filtering. SQL names originate only from sealed Diesel
//! type implementations; literals always travel as bind parameters.
use crate::compiler::{FieldType, substring_pattern};
use crate::diesel_helpers::Predicate;
use crate::parser::{Comparator, Value};

use crate::diesel_helpers::Backend;
#[cfg(not(feature = "postgres"))]
use anyhow::bail;
use anyhow::{Result, ensure};
use diesel::expression::{
    AppearsOnTable, Expression, SelectableExpression, ValidGrouping, is_aggregate::No,
};
use diesel::query_builder::{AstPass, QueryFragment, QueryId};
use diesel::sql_types as st;
use diesel::{Column, QueryResult};

mod sql_type;
#[cfg(feature = "sqlite")]
mod sqlite_temporal;
mod value;

pub use sql_type::{Kind, NativeSqlType, NativeType, Type};
#[cfg(feature = "sqlite")]
pub use sqlite_temporal::SqliteTemporal;
#[cfg(feature = "mysql")]
use value::mysql_decimal_type;
use value::parse;

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
