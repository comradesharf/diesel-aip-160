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

`#[derive(Aip160Filter)]` reads `#[diesel(table_name = ...)]`. `String` and
`Option<String>` fields support text comparisons and global search. `i32`
fields support integer comparisons. Use `#[aip160(skip)]` on unsupported fields.
The generated `Model::compile_filter(&str)` returns an optional boxed Diesel
predicate; an empty filter returns `None`.

Postgres additionally supports JSONB columns. Derive `Aip160Jsonb` on a JSONB
struct to map Rust field names and `#[serde(rename = "...")]` attributes to
stored keys. Dotted filters use bound `#>` paths; `:` uses JSONB containment
and `:*` checks path presence. JSONB is unavailable with SQLite and MySQL.

Filters support `=`, `!=`, `<`, `<=`, `>`, `>=`, `:`, `AND`, `OR`, `NOT`,
parentheses, and bare text search. Text equality supports `*` wildcards;
`text:value` performs substring search. Nullable fields accept unquoted
`null` with `=` and `!=`, while `field:*` checks presence. Integer values
must be unquoted signed 32-bit decimals. Values are bound as parameters.

Run `cargo test --workspace` for parser and compiler tests. Run
`cargo test -p diesel-aip-160 --features sqlite`, then repeat with `mysql`
and `postgres`, for backend predicate tests. The Postgres suite also covers
JSONB.
