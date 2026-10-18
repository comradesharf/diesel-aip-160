//! Typed Diesel predicates and application field adapters.

use anyhow::Result;
#[cfg(feature = "mysql")]
pub use diesel::mysql::Mysql as Backend;
#[cfg(feature = "postgres")]
pub use diesel::pg::Pg as Backend;
use diesel::sql_types::Bool;
#[cfg(feature = "sqlite")]
pub use diesel::sqlite::Sqlite as Backend;
use diesel::{BoxableExpression, Column};

use crate::compiler::ComparisonOperator;

/// A nonaggregate boolean expression on the selected backend and table.
/// SQL NULL can still propagate; boxing does not install a NULL fallback.
pub type Predicate<Table> = Box<dyn BoxableExpression<Table, Backend, SqlType = Bool>>;

/// Application-owned field adapter selected with `#[aip160(adapter = MyAdapter)]`.
/// Validate accepted literals and map them before constructing typed Diesel expressions.
/// Bind all values; any SQL fragments must be trusted application constants.
/// Unquoted `null` and `:*` on nullable fields are dispatched as null comparisons.
/// Implementations must preserve SQL NULL semantics (avoid COALESCE).
pub trait FieldAdapter<C: Column> {
    /// Opt-in ranked comparison; existing implementations need no changes.
    fn compare_query(
        column: C,
        operator: ComparisonOperator,
        value: &crate::parser::Value,
    ) -> Result<crate::ranking::CompiledQueryFilter<C::Table>> {
        Self::compare(column, operator, value).map(crate::compiler::QueryFilter::predicate)
    }

    /// Validate the original literal and construct a bound, typed comparison.
    fn compare(
        column: C,
        operator: ComparisonOperator,
        value: &crate::parser::Value,
    ) -> Result<Predicate<C::Table>>;
}

/// Application-owned search over a model's table, independent of `FieldAdapter`.
/// Selected with field `#[aip160(search_adapter = MySearch)]` or model
/// `#[aip160(global_search_adapter = MySearch)]`. Field search adapters receive `:`
/// literals (except unquoted `*`/`null`); global adapters receive every bare term.
/// Terms explicitly distinguish exact, phrase, and prefix searches. Choose and document query
/// semantics, bind user input, and use only trusted SQL structure/identifiers.
/// Preserve SQL NULL semantics, including under NOT (avoid COALESCE unless the
/// application's search document deliberately defines missing text as empty).
/// The returned predicate may target any registered column or expression on Table.
pub trait SearchAdapter<Table> {
    /// Opt in only when both search methods implement prefix semantics.
    /// The compiler rejects prefix terms before invoking unsupported adapters.
    const SUPPORTS_PREFIX: bool = false;

    /// Opt-in ranked search; adapters define the meaning and NULL policy of metrics.
    fn search_query(
        value: &crate::search::SearchTerm,
    ) -> Result<crate::ranking::CompiledQueryFilter<Table>> {
        Self::search(value).map(crate::compiler::QueryFilter::predicate)
    }

    /// Build the predicate for literal text and its explicit search kind.
    /// Prefix terms are delivered only when `SUPPORTS_PREFIX` is true.
    fn search(value: &crate::search::SearchTerm) -> Result<Predicate<Table>>;
}
