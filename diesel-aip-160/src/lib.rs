#![doc = include_str!("../README.md")]

pub mod compiler;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub mod diesel_helpers;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub mod native;
pub mod parser;
#[cfg(feature = "postgres")]
#[doc(hidden)]
pub mod pg_enum;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub mod ranking;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub mod search;

pub use compiler::QueryFilter;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub use diesel_helpers::{Backend, FieldAdapter, Predicate, SearchAdapter};
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub use ranking::{CompiledQueryFilter, Direction, Ordering, RankingExpression, RankingSignal};

pub use anyhow;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub use diesel;
pub use diesel_aip_160_derive::{Aip160Filter, Aip160Jsonb, Aip160PgEnum};
pub use serde_json;

/// Map a filter's first JSONB path segment to its stored JSON key.
///
/// Derived structs accept their registered Rust field names. `serde_json::Value`
/// accepts arbitrary keys. Later segments use stored JSON keys directly.
#[diagnostic::on_unimplemented(
    message = "unsupported Aip160Filter field type `{Self}`",
    label = "use #[aip160(adapter = MyAdapter)] for custom fields, #[aip160(skip)] to exclude it, or derive Aip160Jsonb for a JSONB field"
)]
pub trait JsonbPath {
    /// Return the stored key, or `None` if this field is not registered.
    fn stored_key(field: &str) -> Option<&str>;
}

impl JsonbPath for serde_json::Value {
    fn stored_key(field: &str) -> Option<&str> {
        Some(field)
    }
}

/// Resolve a field name relative to an explicitly registered JSONB column.
/// The root column returns an empty path; unknown roots or mapped fields return
/// `None`. The derive supplies the trusted column name.
pub fn json_path<T: JsonbPath>(field: &str, column: &str) -> Option<Vec<String>> {
    if field == column {
        return Some(Vec::new());
    }
    let path = field.strip_prefix(column)?.strip_prefix('.')?;
    let mut parts = path.split('.');
    let first = T::stored_key(parts.next()?)?;
    Some(
        std::iter::once(first)
            .chain(parts)
            .map(str::to_owned)
            .collect(),
    )
}
#[cfg(any(
    all(feature = "sqlite", feature = "mysql"),
    all(feature = "sqlite", feature = "postgres"),
    all(feature = "mysql", feature = "postgres"),
))]
compile_error!("enable exactly one of the `sqlite`, `mysql`, or `postgres` features");
