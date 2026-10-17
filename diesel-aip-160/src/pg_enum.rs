//! Filtering support used by `Aip160PgEnum` generated code.

use anyhow::{Result, ensure};
use diesel::expression::{AppearsOnTable, SelectableExpression, ValidGrouping, is_aggregate::No};
use diesel::pg::Pg;
use diesel::query_builder::{AstPass, QueryFragment, QueryId};
use diesel::serialize::ToSql;
use diesel::sql_types::{Bool, HasSqlType, Nullable, SingleValue, SqlType, is_nullable::NotNull};
use diesel::{Column, Expression, QueryResult};
use std::marker::PhantomData;

use crate::compiler::FieldType;
use crate::diesel_helpers::Predicate;
use crate::parser::{Comparator, Value};

/// Ensures the schema column uses the wrapper's SQL type, optionally nullable.
pub trait EnumSqlType<S>: SqlType {
    const NULLABLE: bool;
}
impl<S: SqlType<IsNull = NotNull>> EnumSqlType<S> for S {
    const NULLABLE: bool = false;
}
impl<S: SqlType<IsNull = NotNull>> EnumSqlType<S> for Nullable<S> {
    const NULLABLE: bool = true;
}

pub fn field_type<C, S>(_: C, field: &str, name: &str) -> Option<FieldType>
where
    C: Column,
    C::SqlType: EnumSqlType<S>,
{
    (field == name).then_some(FieldType::Native)
}

struct EnumPredicate<C, T, S> {
    column: C,
    operator: &'static str,
    value: Option<T>,
    sql_type: PhantomData<fn() -> S>,
}
impl<C, T, S> Expression for EnumPredicate<C, T, S> {
    type SqlType = Bool;
}
impl<C: Column, T, S> AppearsOnTable<C::Table> for EnumPredicate<C, T, S> {}
impl<C: Column, T, S> SelectableExpression<C::Table> for EnumPredicate<C, T, S> {}
impl<C, T, S> ValidGrouping<()> for EnumPredicate<C, T, S> {
    type IsAggregate = No;
}
impl<C, T, S> QueryId for EnumPredicate<C, T, S> {
    type QueryId = ();
    const HAS_STATIC_QUERY_ID: bool = false;
}
impl<C, T, S> QueryFragment<Pg> for EnumPredicate<C, T, S>
where
    C: QueryFragment<Pg>,
    T: ToSql<S, Pg>,
    Pg: HasSqlType<S>,
{
    fn walk_ast<'b>(&'b self, mut out: AstPass<'_, 'b, Pg>) -> QueryResult<()> {
        out.unsafe_to_cache_prepared();
        out.push_sql("(");
        self.column.walk_ast(out.reborrow())?;
        out.push_sql(self.operator);
        if let Some(value) = &self.value {
            out.push_bind_param::<S, _>(value)?;
        }
        out.push_sql(")");
        Ok(())
    }
}

pub fn compare<C, T, S>(
    column: C,
    field: &str,
    name: &str,
    operator: Comparator,
    value: &Value,
    parse: impl FnOnce(&str) -> diesel::deserialize::Result<T>,
) -> Result<Predicate<C::Table>>
where
    C: Column + QueryFragment<Pg> + Send + 'static,
    C::SqlType: EnumSqlType<S>,
    S: SqlType<IsNull = NotNull> + SingleValue + 'static,
    T: ToSql<S, Pg> + Send + 'static,
    Pg: HasSqlType<S>,
{
    ensure!(field == name, "unknown filter field `{field}`");
    let present = operator == Comparator::Has && !value.quoted && value.text == "*";
    let null = !value.quoted && value.text == "null";
    let (operator, value) = if present || null {
        if !present {
            ensure!(
                <C::SqlType as EnumSqlType<S>>::NULLABLE,
                "field `{field}` is not nullable"
            );
            ensure!(
                matches!(operator, Comparator::Equal | Comparator::NotEqual),
                "null supports only = and != comparisons"
            );
        }
        (
            if present || operator == Comparator::NotEqual {
                " IS NOT NULL"
            } else {
                " IS NULL"
            },
            None,
        )
    } else {
        let mapped = parse(&value.text).map_err(anyhow::Error::from_boxed)?;
        (
            match operator {
                Comparator::Equal | Comparator::Has => " = ",
                Comparator::NotEqual => " != ",
                Comparator::LessThan => " < ",
                Comparator::LessThanOrEqual => " <= ",
                Comparator::GreaterThan => " > ",
                Comparator::GreaterThanOrEqual => " >= ",
            },
            Some(mapped),
        )
    };
    Ok(Box::new(EnumPredicate::<C, T, S> {
        column,
        operator,
        value,
        sql_type: PhantomData,
    }))
}
