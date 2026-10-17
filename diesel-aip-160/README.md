# diesel-aip-160

**Status: beta (`0.1.0-beta.1`).** The public API and filter behavior may change
between beta releases. Pin the exact version and review changes before upgrading.

AIP-160 filter parser and Diesel predicate derive. The parser and compiler work
without a database driver. Enable exactly one backend feature to use
`Aip160Filter`:

```toml
[dependencies]
diesel-aip-160 = { version = "=0.1.0-beta.1", features = ["sqlite"] }
```

The available features are `sqlite`, `mysql`, and `postgres`. None is enabled
by default. The SQLite feature enables Diesel's SQLite driver. The MySQL and
Postgres features enable Diesel's backend SQL generation without linking their
native client libraries. If your application connects through Diesel, enable
`diesel/mysql` or `diesel/postgres` in its own dependencies.

## Examples

Each example builds a typed predicate and prints parameterized SQL without a
running database:

```sh
cargo run -p diesel-aip-160 --example sqlite --features sqlite
cargo run -p diesel-aip-160 --example mysql --features mysql
cargo run -p diesel-aip-160 --example postgres --features postgres
```

`#[derive(Aip160Filter)]` reads `#[diesel(table_name = ...)]` and dispatches
from each column's SQL type. All Diesel built-in column families and nullable
variants are supported without adapters. Text fields support global search.
Use `#[aip160(skip)]` to exclude fields. The generated
`Model::compile_filter(&str)` returns an optional boxed predicate; empty input
returns `None`.

Optional `chrono`, `time`, `uuid`, `numeric`, `ipnet-address`, and
`network-address` features enable Diesel representation integrations.
`native-types` enables all except the alternative `network-address`.
See the [workspace README](../README.md#native-types-and-literal-formats) for
literal formats, operators, backend differences, and validation boundaries.
Run `../scripts/test-native-types.sh` from this crate to execute the full
PostgreSQL/MySQL/SQLite suite through Docker Compose.

Postgres additionally supports JSONB columns. Derive `Aip160Jsonb` on a JSONB
struct to map Rust field names and `#[serde(rename = "...")]` attributes to
stored keys. Dotted filters use bound `#>` paths; `:` uses JSONB containment
and `:*` checks path presence. JSONB is unavailable with SQLite and MySQL.

Filters support `=`, `!=`, `<`, `<=`, `>`, `>=`, `:`, `AND`, `OR`, `NOT`,
parentheses, and bare text search. Text equality supports `*` wildcards;
`text:value` and bare text perform case-insensitive substring search using
`LOWER(column) LIKE LOWER(pattern)`. Unicode and locale behavior follow the
database; SQLite's built-in `LOWER` converts ASCII only. Equality and wildcard
equality retain their existing operator and collation behavior. `*` is literal
in substring searches except for the unquoted presence check. Nullable fields
accept unquoted `null` with `=` and `!=`, while `field:*` checks presence.
Integer values must be unquoted signed decimals within the field's 32/64-bit
range. BIGINT accepts `4294967295` exactly. Values are bound as
parameters.

Run `cargo test --workspace` for parser and compiler tests. Run
`cargo test -p diesel-aip-160 --features sqlite`, then repeat with `mysql`
and `postgres`, for backend predicate tests. The Postgres suite also covers
JSONB.

For PostgreSQL storage conversion of a local wrapper around a dependency enum,
derive [`Aip160PgEnum`](../diesel-aip-160-derive/README.md#aip160pgenum) with a
label value type and two fallible conversion functions. It generates
`FromSql`/`ToSql`; your application supplies the enum mapping and validation.

Custom enum fields opt in with `#[aip160(adapter = MyAdapter)]`. Implement
`diesel_helpers::FieldAdapter<Column>` to validate/map original `parser::Value`
literals and return typed, bound Diesel predicates. Ordering is explicit: a
numeric enum adapter can cast a trusted column expression to BIGINT; a native
enum comparison instead follows PostgreSQL declaration order. No GTFS mapping
is built into the library. See the compiling `examples/postgres_enums.rs` for
route codes, protobuf pickup/drop-off mappings, nullable BIGINT, and adapter NULL
handling. Run its SQL-generation tests with `--features postgres --all-targets`.
The optional `postgres-integration` feature enables an ignored live PostgreSQL
test (native libpq and `AIP160_TEST_DATABASE_URL` required); run with
`--example postgres_enums postgres_executes -- --ignored`.

Adapters own trusted expressions and must bind all input values, never interpolate
filter literals or field names into SQL. Unknown custom types need an adapter,
`Aip160Jsonb`/`JsonbPath` for JSONB, or `#[aip160(skip)]`.

The compiler dispatches every built-in Diesel SQL type through `FieldType::Native`
and application adapters through `FieldType::Custom { nullable }`. Native handlers
receive `parser::Comparator` and the original `parser::Value`, and own scalar
validation, text patterns, JSON traversal, and backend SQL construction.
Manual `Aip160FilterCompiler` implementations must implement `native_compare`;
the former `compare`/`FilterLiteral`, scalar `FieldType` variants, `like`, and
JSON-specific compiler hooks have been removed. The obsolete scalar/JSON
helpers in `diesel_helpers` have also been removed; use native filtering or
construct typed Diesel expressions in a `FieldAdapter`.
