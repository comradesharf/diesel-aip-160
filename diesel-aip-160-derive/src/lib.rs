//! Derive macros for AIP-160 filtering with Diesel.
//!
//! Prefer the macros re-exported by `diesel_aip_160`, which also provides the
//! parser, compiler, and runtime helpers required by generated code.
//! [`Aip160Filter`] requires a named-field struct with
//! `#[diesel(table_name = ...)]` and one database backend enabled on the runtime
//! crate. [`Aip160Jsonb`] maps Rust field names to stored JSONB keys.

use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod attributes;
mod filter;
mod jsonb;
mod pg_enum;

/// Generate PostgreSQL enum codecs for a local newtype around a dependency type.
///
/// Requires `#[diesel(sql_type = SqlType)]` and
/// `#[aip160(value_type = i16, from_value = decode, to_value = encode)]`.
/// `decode` takes the parsed value and returns `Result<Inner, E>`; `encode`
/// takes `&Inner` and returns `Result<ValueType, E>`. Errors must convert to
/// `Box<dyn std::error::Error + Send + Sync>`. Values use `FromStr`/`Display`
/// to read/write PostgreSQL enum labels. Enable the runtime's `postgres` feature.
/// Derive Diesel's `AsExpression` and `FromSqlRow` separately. `Aip160Filter`
/// automatically reuses these conversions for wrapper fields (including `Option`)
/// without a field adapter. Ordering follows PostgreSQL enum declaration order.
#[proc_macro_derive(Aip160PgEnum, attributes(aip160, diesel))]
pub fn derive_aip160_pg_enum(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    pg_enum::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Generate a typed filter compiler and a `compile_filter` method for a Diesel model.
///
/// Use `#[diesel(table_name = ...)]` to specify the table and `#[aip160(skip)]`
/// to exclude fields from filtering.
#[proc_macro_derive(Aip160Filter, attributes(aip160, diesel))]
pub fn derive_aip160_filter(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    filter::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Map Rust field names to their serialized JSON keys for filter traversal.
#[proc_macro_derive(Aip160Jsonb, attributes(serde))]
pub fn derive_aip160_jsonb(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    jsonb::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[cfg(test)]
mod tests;
