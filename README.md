# diesel-aip-160

Parse AIP-160 filter strings, validate them against model fields, and compile
them into typed, parameterized Diesel predicates.

**Status: beta (`0.1.0-beta.1`).** Both crates share this prerelease version.
The public API and filter behavior may change between beta releases. Pin the
exact version for reproducible integrations and review changes before upgrading.

This Rust workspace contains:

- [`diesel-aip-160`](diesel-aip-160): the parser, compiler, and Diesel helpers.
- [`diesel-aip-160-derive`](diesel-aip-160-derive): the `Aip160Filter` and
  `Aip160Jsonb` derive macros, re-exported by the main crate.

## Installation

Enable one database backend:

```toml
[dependencies]
diesel-aip-160 = { version = "=0.1.0-beta.1", features = ["sqlite"] }
diesel = { version = "2", default-features = false, features = ["sqlite"] }
```

Available features are `sqlite`, `mysql`, and `postgres`. No backend is enabled
by default, and enabling multiple backends produces a compile error. The parser
and generic compiler also work without a backend feature.

The `mysql` and `postgres` features enable SQL generation without linking native
database client libraries. To connect using Diesel, also enable `mysql` or
`postgres` on your application's `diesel` dependency.

## Example: compile a filter into a query

```rust
use diesel::{prelude::*, sqlite::Sqlite};
use diesel_aip_160::Aip160Filter;

diesel::table! {
    records (id) {
        id -> Integer,
        name -> Text,
        note -> Nullable<Text>,
    }
}

#[derive(Aip160Filter)]
#[diesel(table_name = records)]
struct Record {
    id: i32,
    name: String,
    note: Option<String>,
}

fn main() -> diesel_aip_160::anyhow::Result<()> {
    let filter = "name=Alice AND note:*";
    let mut query = records::table.into_boxed::<Sqlite>();

    if let Some(predicate) = Record::compile_filter(filter)? {
        query = query.filter(predicate);
    }

    // Inspect the query without opening a database connection.
    println!("{}", diesel::debug_query::<Sqlite, _>(&query));
    Ok(())
}
```

The filter matches records whose name is `Alice` and whose note is not NULL.
Values are bound as SQL parameters. An empty or whitespace-only filter returns
`Ok(None)`, so the query can omit `.filter(...)`. Invalid syntax, unknown fields,
and incompatible values return an error.

The derive supports `String`, `Option<String>`, and `i32` fields. Exclude fields
that should not be filterable with `#[aip160(skip)]`. PostgreSQL also supports
JSONB fields, described below.

## Filter syntax

These examples use the `Record` model above:

| Filter | Meaning |
| --- | --- |
| `name=Alice` | Exact text equality |
| `name!=Alice` | Text inequality |
| `id>=10` | Integer comparison |
| `name="Alice Smith"` | A quoted value containing spaces |
| `name="Ali*"` | Text equality with a `*` wildcard |
| `name:lic` | Text substring search |
| `note=null` | Nullable text is NULL |
| `note!=null` or `note:*` | Nullable text is not NULL |
| `name:*` or `id:*` | Always true for a nonnullable scalar field |
| `Alice` | Substring search across all filterable text fields |
| `name=(Alice OR Bob)` | Apply the comparison to either literal |
| `name=Alice AND id>=10` | Combine predicates with AND |
| `NOT (name=Alice OR name=Bob)` | Negate a group |

Supported comparison operators are `=`, `!=`, `<`, `<=`, `>`, `>=`, and `:`.
Integer `:` behaves as equality. Integers must be unquoted signed 32-bit decimal
values; quoted numbers, fractions, and out-of-range values are rejected.

Logical operators use uppercase `AND`, `OR`, and `NOT`; adjacent expressions
also imply AND. A leading `-` can negate an expression. **OR binds more tightly
than AND**: `name=Alice AND id=1 OR id=2` means
`name=Alice AND (id=1 OR id=2)`. Use parentheses to make grouping explicit.

For text `=` and `!=`, `*` is a wildcard even inside quotes. For text `:` and
global search, `*` is literal except for the unquoted presence check `field:*`.
SQL LIKE characters `%`, `_`, and backslash are escaped in search patterns.
Unquoted `null` has NULL semantics only for nullable text; `note="null"`
matches the literal string instead. SQL NULL behavior still applies when
negating predicates over nullable columns.

This is AIP-160 filter syntax: CEL operators such as `==` and `&&`, function
calls, and field-to-field comparisons are unsupported. For example,
`name=note` compares the name to the literal text `note`.

## PostgreSQL JSONB

Enable `postgres` and use `Aip160Jsonb` to map fields in a JSONB value to stored
keys. For a Diesel table named `records` with `id -> Integer` and
`payload -> Jsonb`:

```rust
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, Aip160Jsonb};

#[derive(Aip160Jsonb)]
struct Payload {
    #[serde(rename = "displayName")]
    display_name: String,
    details: diesel_aip_160::serde_json::Value,
}

#[derive(Aip160Filter)]
#[diesel(table_name = records)]
struct Record {
    id: i32,
    payload: Payload,
}
```

Example filters:

```text
payload.display_name=Alice
payload.details.enabled=true
payload.details:*
payload.details:'{"enabled":true}'
```

Dotted paths use PostgreSQL `#>` extraction. `:` performs JSONB containment
(`@>`), while `:*` checks that the path exists, including a stored JSON null.
Paths and values are bound as parameters. The first path segment uses the Rust
field name, so `payload.display_name` maps to the stored key `displayName`.
Subsequent segments address stored JSON keys directly.

`serde_json::Value` can also be used directly as a JSONB field when arbitrary
keys are needed. `Aip160Jsonb` supports field-level `#[serde(rename = "...")]`;
struct-level `rename_all` is rejected. JSONB filtering is available only with
PostgreSQL. See the [JSONB tests](diesel-aip-160/tests/derive.rs) for a complete
schema and query examples.

## Compiler API

[`compiler.rs`](diesel-aip-160/src/compiler.rs) exposes
`compile(source, &compiler) -> anyhow::Result<Option<C::Predicate>>` and the
`Aip160FilterCompiler` trait. `Aip160Filter` generates a compiler named after
the model, such as `RecordFilterCompiler`. In the SQLite example, the direct
equivalent of `Record::compile_filter(filter)` is:

```rust
let predicate = diesel_aip_160::compiler::compile(
    "name=Alice AND id>=10",
    &RecordFilterCompiler,
)?;
```

For a custom backend, implement `Aip160FilterCompiler` with your own predicate
type. Supply field types and searchable text fields, typed comparisons, LIKE
operations, constants, and logical composition. The compiler parses and
validates the filter before dispatching these operations. JSONB methods have
default implementations that return unsupported-operation errors.

## Run examples and tests

The included examples print SQL and do not require a running database:

```sh
cargo run -p diesel-aip-160 --example sqlite --features sqlite
cargo run -p diesel-aip-160 --example mysql --features mysql
cargo run -p diesel-aip-160 --example postgres --features postgres
```

Run tests separately for each backend because backend features are mutually
exclusive:

```sh
cargo test --workspace
cargo test -p diesel-aip-160 --features sqlite
cargo test -p diesel-aip-160 --features mysql
cargo test -p diesel-aip-160 --features postgres
```

The repository pins its Rust toolchain in
[`rust-toolchain.toml`](rust-toolchain.toml).
