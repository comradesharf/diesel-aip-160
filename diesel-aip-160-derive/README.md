# diesel-aip-160-derive

Derive macros for compiling AIP-160 filters against Diesel models.

**Status: beta (`0.1.0-beta.1`).** Prefer the macros re-exported by
[`diesel-aip-160`](../diesel-aip-160), which provides the parser, compiler,
and runtime helpers used by generated code.

## `Aip160Filter`

Enable exactly one backend (`sqlite`, `mysql`, or `postgres`) on
`diesel-aip-160`, then derive on a struct with named fields:

```rust
use diesel_aip_160::Aip160Filter;

#[derive(Aip160Filter)]
#[diesel(table_name = records)]
struct Record {
    name: String,
    count: i32,
    optional_name: Option<String>,
    #[aip160(skip)]
    internal_note: String,
}
```

The consuming crate must define the matching Diesel `records` table. The
macro registers the `diesel` helper attribute, so an additional Diesel derive
is not required. It generates `RecordFilterCompiler` and
`Record::compile_filter(&str)`, returning an `anyhow::Result` containing an
optional boxed Diesel predicate. An empty filter returns `None`.

`String`, `Option<String>`, and `i32` fields support filtering. Exclude
unsupported fields with `#[aip160(skip)]`. PostgreSQL also supports JSONB
fields through `diesel_aip_160::JsonbPath`; other backends reject those fields.

## `Aip160Jsonb`

Derive on a named-field struct to map Rust field names to stored JSONB keys:

```rust
use diesel_aip_160::Aip160Jsonb;

#[derive(Aip160Jsonb)]
struct Details {
    status: String,
    #[serde(rename = "displayName")]
    display_name: String,
}
```

The macro implements `diesel_aip_160::JsonbPath`. Fields without a rename use
the Rust field name; unknown fields return `None`. Struct-level
`#[serde(rename_all = "...")]` is rejected. See the
[workspace README](../README.md) for complete schema and query examples.

## Development

```sh
cargo test -p diesel-aip-160-derive
```
