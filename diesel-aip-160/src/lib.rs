//! Parse AIP-160 filters and compile typed, parameterized Diesel predicates.
//!
//! The [`parser`] and [`compiler`] work without a database backend. Enable one
//! of `sqlite`, `mysql`, or `postgres` to use [`Aip160Filter`] on Diesel models.
//! Backend features are mutually exclusive. PostgreSQL additionally supports
//! JSONB filtering with [`Aip160Jsonb`].
//!
//! ```
//! use diesel_aip_160::parser::parse_filter;
//!
//! let expression = parse_filter("name=Alice AND id>=10")?;
//! assert!(expression.is_some());
//! assert!(parse_filter("   ")?.is_none());
//! # Ok::<(), diesel_aip_160::anyhow::Error>(())
//! ```

pub mod compiler;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub mod diesel_helpers;
pub mod parser;

pub use anyhow;
#[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
pub use diesel;
pub use diesel_aip_160_derive::{Aip160Filter, Aip160Jsonb};
pub use serde_json;

/// Split a JSONB field name into a column-relative path. The caller supplies
/// only column names explicitly registered by the derive macro.
pub trait JsonbPath {
    fn stored_key(field: &str) -> Option<&str>;
}

impl JsonbPath for serde_json::Value {
    fn stored_key(field: &str) -> Option<&str> {
        Some(field)
    }
}

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
