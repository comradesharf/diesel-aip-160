# diesel-aip-160-derive

Derive macros for compiling AIP-160 filters against Diesel models.

**Status: beta (`0.1.0-beta.5`).** Prefer the macros re-exported by
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

The macro dispatches from the schema column's SQL type rather than the Rust
field name. All Diesel built-in column families, aliases, and nullable variants
support filtering. Exclude fields with `#[aip160(skip)]`. PostgreSQL JSONB
traversal uses `diesel_aip_160::JsonbPath`; `serde_json::Value` supports arbitrary
keys automatically. SQLite JSONB supports root comparisons. See the
[workspace README](../README.md#native-types-and-literal-formats) for formats.

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

## `Aip160PgEnum`

Enable `postgres` on the runtime crate to generate PostgreSQL enum storage
conversions for a local wrapper around an enum from another crate:

```rust
use diesel::deserialize::FromSqlRow;
use diesel::expression::AsExpression;
use diesel_aip_160::Aip160PgEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, AsExpression, FromSqlRow, Hash, Aip160PgEnum)]
#[diesel(sql_type = RouteTypeEnum)]
#[aip160(
    value_type = i16,
    from_value = route_type_from_code,
    to_value = route_type_to_code,
)]
pub struct GtfsRouteType(pub RouteType);
```

Here `RouteTypeEnum` is your Diesel SQL type and `RouteType` comes from your
dependency. Move your existing domain mappings into two functions:

| Function | Input | Return type |
| --- | --- | --- |
| `route_type_from_code` | `i16` | `diesel::deserialize::Result<RouteType>` |
| `route_type_to_code` | `&RouteType` | `diesel::deserialize::Result<i16>` |

The derive generates `FromSql<RouteTypeEnum, Pg>` and
`ToSql<RouteTypeEnum, Pg>`, including UTF-8 decoding, `i16` parsing, label
formatting, and error propagation. Remove your manual implementations.
`value_type` can also be `String` for nonnumeric labels; it must implement
`FromStr` and `Display`, with parse errors convertible to Diesel's boxed error.
Both conversion functions return `Result`; their errors must convert to
`Box<dyn std::error::Error + Send + Sync>`. The serializer borrows the wrapped
value, so the inner type need not be `Copy` or `Clone`.

If the dependency already provides compatible fallible conversions, use
paths such as `RouteType::try_from` directly. The derive cannot infer the
mapping or validation rules of an external enum. A local wrapper keeps the
Diesel implementations compatible with Rust's orphan rules.

`Aip160Filter` also picks up these conversions automatically. Use the wrapper
as a model field without an adapter attribute:

```rust
#[derive(diesel_aip_160::Aip160Filter)]
#[diesel(table_name = routes)]
struct Route {
    id: i32,
    route_type: GtfsRouteType,
    optional_route_type: Option<GtfsRouteType>,
}
```

Each schema column must use `RouteTypeEnum` (or `Nullable<RouteTypeEnum>`).
Type aliases and qualified wrapper paths work too. Filter values are parsed
as `value_type`, passed through `from_value`, and bound using the wrapper's
`ToSql` implementation. Both quoted and unquoted labels are accepted.
Comparisons support `=`, `!=`, `<`, `<=`, `>`, `>=`, and `:` (equality), plus
`:*` for presence and `=null`/`!=null` on nullable columns. Invalid labels
fail through the same decoder used for database values. Enum fields are
excluded from global text search.

Ordering follows **PostgreSQL enum declaration order**, even for numeric labels.
Use `#[aip160(adapter = MyAdapter)]` on a field when it needs different literal
mapping or numeric ordering; explicit adapters override automatic filtering.
See the [compiling model and codec tests](../diesel-aip-160/tests/pg_enum.rs),
including the opt-in PostgreSQL execution test:

```sh
AIP160_TEST_DATABASE_URL=postgres://... cargo test -p diesel-aip-160 \
  --features postgres-integration --test pg_enum -- --include-ignored
```

## Development

```sh
cargo test -p diesel-aip-160-derive
```


`Aip160Filter` supports `i32`, `i64`, and their `Option` variants in addition
to text fields. Use `#[aip160(adapter = path::Adapter)]` for custom fields such
that are not covered by `Aip160PgEnum`; implement the runtime's
`FieldAdapter<Column>` trait.
Adapters validate/map original literals and return typed predicates with bound
values, choosing numeric or native enum ordering explicitly. `skip` and
`adapter` cannot be combined. JSONB types still opt in through `JsonbPath` or
`Aip160Jsonb`. See the runtime's `examples/postgres_enums.rs` for a compiling
example and live-test instructions.
