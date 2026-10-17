//! Typed Diesel predicates and application field adapters.

use anyhow::Result;
#[cfg(feature = "mysql")]
use diesel::mysql::Mysql as Backend;
#[cfg(feature = "postgres")]
use diesel::pg::Pg as Backend;
use diesel::sql_types::Bool;
#[cfg(feature = "sqlite")]
use diesel::sqlite::Sqlite as Backend;
use diesel::{BoxableExpression, Column};

use crate::compiler::ComparisonOperator;

pub type Predicate<Table> = Box<dyn BoxableExpression<Table, Backend, SqlType = Bool>>;

/// Application-owned field adapter selected with `#[aip160(adapter = MyAdapter)]`.
/// Validate accepted literals and map them before constructing typed Diesel expressions.
/// Bind all values; any SQL fragments must be trusted application constants.
/// Unquoted `null` and `:*` on nullable fields are dispatched as null comparisons.
/// Implementations must preserve SQL NULL semantics (avoid COALESCE).
pub trait FieldAdapter<C: Column> {
    fn compare(
        column: C,
        operator: ComparisonOperator,
        value: &crate::parser::Value,
    ) -> Result<Predicate<C::Table>>;
}
