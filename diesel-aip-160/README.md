# diesel-aip-160

Parse AIP-160 filter strings, validate them against model fields, and compile
them into typed, parameterized Diesel predicates.

**Status: beta (`0.1.0-beta.7`).** Both crates share this prerelease version.
The public API and filter behavior may change between beta releases. Pin the
exact version for reproducible integrations and review changes before upgrading.

The main crate re-exports `Aip160Filter`, `Aip160Jsonb`, and `Aip160PgEnum`;
applications do not need a separate derive dependency. This guide is also the
crate's docs.rs landing page and includes the filter-to-SQL reference.

- [Installation](#installation) and [first query](#example-compile-a-filter-into-a-query)
- [Native types](#native-types-and-literal-formats) and [filter syntax](#filter-syntax)
- [Equivalent SQL](#equivalent-sql-by-data-type) and [JSONB](#postgresql-jsonb)
- [Custom fields and enum codecs](#custom-fields-and-postgresql-enums)
- [Full-text search](#opt-in-full-text-search) and [ranking](#ranking-signals-and-ordering)
- [Compiler extension points](#compiler-api) and [limits](#validation-and-limits)

## Installation

Enable one database backend:

```toml
[dependencies]
diesel-aip-160 = { version = "=0.1.0-beta.7", features = ["sqlite"] }
diesel = { version = "2", default-features = false, features = ["sqlite"] }
```

Available features are `sqlite`, `mysql`, and `postgres`. No backend is enabled
by default, and enabling multiple backends produces a compile error. The parser
and generic compiler also work without a backend feature.

The `mysql` and `postgres` features enable SQL generation without linking native
database client libraries. To connect using Diesel, also enable `mysql` or
`postgres` on your application's `diesel` dependency.

`Backend`, `Predicate`, `FieldAdapter`, `SearchAdapter`, `QueryFilter`, and the
ranking types are re-exported at the crate root for application code. `Backend`
is the Diesel backend selected at compile time.

| Feature | Purpose |
| --- | --- |
| `postgres`, `mysql`, `sqlite` | Mutually exclusive backend selection |
| `postgres-integration`, `mysql-integration` | Backend plus native connection driver for execution tests |
| `sqlite-integration` | SQLite backend with bundled SQLite for execution tests |
| `chrono`, `time`, `uuid`, `numeric` | Optional Rust value integrations |
| `ipnet-address`, `network-address` | Alternative network address representations |
| `native-types` | Convenience group: chrono, time, UUID, numeric, and ipnet |

## Example: compile a filter into a query

```rust
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# mod example {

# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, Backend};

diesel::table! {
    records (id) { id -> Integer, name -> Text, note -> Nullable<Text> }
}
#[derive(Aip160Filter)]
#[diesel(table_name = records)]
struct Record { id: i32, name: String, note: Option<String> }

let mut query = records::table.into_boxed::<Backend>();
if let Some(predicate) = Record::compile_filter("name=Alice AND note:*")? {
    query = query.filter(predicate);
}
println!("{}", diesel::debug_query::<Backend, _>(&query));
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# example::run()?;
# Ok(())
# }
```

Equivalent PostgreSQL SQL for this first query (other backends use their own
placeholder syntax):

```sql
SELECT records.id, records.name, records.note
FROM records
WHERE records.name = $1 AND records.note IS NOT NULL;
-- binds: ["Alice"]
```

The filter matches records whose name is `Alice` and whose note is not NULL.
Values are bound as SQL parameters. An empty or whitespace-only filter returns
`Ok(None)`, so the query can omit `.filter(...)`. Invalid syntax, unknown fields,
and incompatible values return an error.

The derive reads each schema column's Diesel SQL type. All built-in column type
families are filterable without an adapter, including nullable variants, aliases,
PostgreSQL arrays/ranges/multiranges/records, MySQL unsigned numbers, and SQLite
timestamps and JSONB. Exclude fields with `#[aip160(skip)]`. Application-defined
PostgreSQL enum wrappers deriving `Aip160PgEnum` support filtering automatically.
Other custom SQL types, including composites, use adapters.

## Native types and literal formats

Enable optional Rust representation integrations as needed:

```toml
diesel-aip-160 = { version = "=0.1.0-beta.7", features = ["postgres", "chrono", "uuid", "numeric", "ipnet-address"] }
```

`chrono`, `time`, `uuid`, `numeric` (BigDecimal), `ipnet-address`, and
`network-address` (ipnetwork) forward to Diesel's corresponding integrations.
`native-types` enables all of these except the alternative `network-address`.
No representation dependency is enabled by default. Schema type dispatch also
works when your application already enables a representation through Diesel.
For SQLite temporal models, enable the matching `chrono` or `time` feature
on this crate so literals match Diesel's storage format and UTC conversion.
SQLite temporal comparisons preserve the precision of Diesel's serializer
without conversion through SQLite date functions.
Temporal validation uses `chrono` or `time` when enabled; otherwise the database
validates temporal literals.

| SQL family | Filter example | Behavior |
| --- | --- | --- |
| Boolean | `enabled=true` | Unquoted `true` / `false`; native ordering |
| Signed/unsigned integers | `code=65535` | Unquoted, checked against the SQL type's bit width |
| Float/double | `score>=1.25e-3` | Unquoted finite values within the target type's range |
| Numeric/decimal | `amount=12345678901234567890.123456789` | Exact decimal parsing; no float conversion on PostgreSQL/MySQL |
| Binary | `bytes="0xdeadbeef"` | Quoted even-length hexadecimal, including empty `0x` |
| Date/time/timestamp | `created_at>="2026-10-17T12:34:56"` | Quoted ISO values; timezone-aware types use RFC 3339 offsets |
| UUID/network/MAC/WAL position | `network="192.168.1.0/24"` | Quoted canonical strings |
| PostgreSQL interval/money | `duration="1 day 2 hours"` | Quoted native strings; database validates format |
| PostgreSQL array | `tags:"{rust,diesel}"` | Quoted PostgreSQL array literal; `:` means contains |
| PostgreSQL range | `window:"[1,10)"` | Quoted native range literal; `:` means contains |
| PostgreSQL multirange | `windows:"{[1,10),[20,30)}"` | Quoted native multirange literal; `:` means contains |
| PostgreSQL record | `pair='(7,"seven")'` | Typed row comparison; empty unquoted record fields represent NULL |
| JSON | `payload='{"enabled":true}'` | Validated JSON; equality, inequality, and backend-supported containment |
| PostgreSQL `Citext` | `name=alice` | Database case-insensitive comparisons and text search |
| PostgreSQL internal `CChar` | `flag="a"` | One quoted byte or an unquoted unsigned 8-bit integer |

Comparisons use database-native ordering. Unsupported operators return an error.
PostgreSQL `Json` comparisons convert to JSONB; MySQL JSON containment uses
`JSON_CONTAINS`. SQLite JSON/JSONB equality compares `json(...)` text, so object
key order remains significant; SQLite JSON containment is rejected. PostgreSQL
JSONB retains dotted paths and structured Rust key mappings described below.

Unquoted `null` checks SQL NULL for nullable native columns; `:*` checks presence.
For PostgreSQL JSONB, dotted `=null` compares JSON null, while dotted `:*` checks
path existence. Root nullable JSONB `=null` checks SQL NULL. Nonnullable JSONB
retains its existing `=null` JSON-null comparison.

PostgreSQL complex literals are bound as text and cast to trusted built-in SQL
types. Their complete format validation happens when executing the query.
Record elements are individually bound and typed. MySQL decimals use a cast
with a scale derived from the literal (maximum 65 digits and 30 fractional
places); values outside those native limits are rejected. SQLite numeric
precision follows SQLite's floating-point storage behavior.

## Filter syntax

These examples use the `Record` model above:

| Filter | Meaning |
| --- | --- |
| `name=Alice` | Exact text equality |
| `name!=Alice` | Text inequality |
| `id>=10` | Integer comparison |
| `name="Alice Smith"` | A quoted value containing spaces |
| `name="Ali*"` | Text equality with a `*` wildcard |
| `name:lic` | Case-insensitive text substring search |
| `note=null` | Nullable text is NULL |
| `note!=null` or `note:*` | Nullable text is not NULL |
| `name:*` or `id:*` | Always true for a nonnullable scalar field |
| `Alice` | Case-insensitive substring search across all filterable text fields |
| `name=(Alice OR Bob)` | Apply the comparison to either literal |
| `name=Alice AND id>=10` | Combine predicates with AND |
| `NOT (name=Alice OR name=Bob)` | Negate a group |

Supported comparison operators are `=`, `!=`, `<`, `<=`, `>`, `>=`, and `:`.
For signed 32-bit and 64-bit integers, `:` behaves as equality. Integers must
be unquoted decimal values within the SQL column's signed or unsigned range;
quoted numbers, fractions, negative unsigned values, and overflow are rejected.
`i64` accepts the full uint32 range, including
`4294967295`, without precision loss. Nullable integers support `=null`,
`!=null`, and `:*`; ordering against NULL is rejected. Other comparisons and
logical negation retain SQL three-valued NULL semantics.

Logical operators use uppercase `AND`, `OR`, and `NOT`; adjacent expressions
also imply AND. A leading `-` can negate an expression. **OR binds more tightly
than AND**: `name=Alice AND id=1 OR id=2` means
`name=Alice AND (id=1 OR id=2)`. Use parentheses to make grouping explicit.

For text `=` and `!=`, `*` is a wildcard even inside quotes. For text `:` and
global search, `*` is literal except for the unquoted presence check `field:*`.
SQL LIKE characters `%`, `_`, and backslash are escaped in search patterns.

By default, text `:` and global search use `LOWER(column) LIKE LOWER(pattern)` on SQLite,
MySQL, and PostgreSQL. For example, `name:ali` matches `Alice`, `ALICE`, and
`Malik`. Case conversion follows the database's Unicode, locale, and collation
rules; SQLite's built-in `LOWER` handles ASCII case conversion only. This does
not guarantee accent-insensitive matching. Substring searches may need
backend-specific indexing for large tables.

Text comparisons (`=`, `!=`, and ordering) and `*` wildcard equality retain
their existing SQL operator and collation behavior; they are not normalized
with `LOWER`. NULL and presence checks and JSONB containment are unchanged.

Unquoted `null` has NULL semantics for nullable scalar fields; `note="null"`
matches the literal string instead. SQL NULL behavior still applies when
negating predicates over nullable columns.

This is AIP-160 filter syntax: CEL operators such as `==` and `&&`, function
calls, and field-to-field comparisons are unsupported. For example,
`name=note` compares the name to the literal text `note`.

### Choose syntax by data type

The column's **Diesel SQL type** determines which literals and operators are
accepted. The field names below are illustrative; replace them with fields in
your model. Types such as UUID, arrays, and ranges require a backend that
supports them. See [native types and literal formats](#native-types-and-literal-formats)
for optional representation features and backend limitations.

| Data type | Example filters | How it works |
| --- | --- | --- |
| Text (`Text`, `VarChar`) | `name=Alice`, `name="Alice Smith"`, `name:ali`, `name="Ali*"` | `=` compares text; `:` searches for a case-insensitive substring; `*` in `=` / `!=` is a wildcard. Quote spaces or special characters. |
| Boolean (`Bool`) | `enabled=true`, `enabled!=false` | Use lowercase, unquoted `true` or `false`; `"true"` and `1` are rejected. |
| Signed integer (`SmallInt`, `Integer`, `BigInt`) | `age>=18`, `balance<0`, `id=(1 OR 2)` | Use unquoted whole numbers within the SQL type's range. Ordering is numeric. |
| MySQL unsigned integer | `code=65535`, `code>=100` | Use unquoted nonnegative whole numbers within the unsigned column's range. |
| Floating point (`Float`, `Double`) | `score>=1.25`, `ratio<1e-3` | Use unquoted finite numbers; scientific notation is accepted. |
| Decimal (`Numeric`) | `amount>=19.95` | Use an unquoted decimal. PostgreSQL and MySQL preserve decimal precision; SQLite follows floating-point storage behavior. |
| Date (`Date`) | `day="2026-10-17"` | Quote an ISO date. |
| Time (`Time`) | `clock>="12:34:56"` | Quote an ISO time; fractional seconds are supported. |
| Timestamp (`Timestamp`) | `created_at>="2026-10-17T12:34:56"` | Quote an ISO timestamp without a timezone. |
| Timestamp with timezone | `created_at>="2026-10-17T12:34:56Z"`, `created_at<"2026-10-18T00:00:00+08:00"` | Quote an RFC 3339 timestamp with `Z` or an explicit offset. |
| Binary (`Binary`) | `bytes="0xdeadbeef"` | Quote hexadecimal bytes with a `0x` prefix and an even number of hex digits. |
| UUID (`Uuid`) | `key="550e8400-e29b-41d4-a716-446655440000"` | Quote the UUID string. |
| Network (`Inet`, `Cidr`) | `network="192.168.1.0/24"` | Quote the address, optionally with a prefix length. |
| PostgreSQL array | `tags="{rust,diesel}"`, `tags:"{rust}"` | `=` compares the array; `:` checks that it contains the supplied array. Use a quoted PostgreSQL array literal. |
| PostgreSQL range / multirange | `window:"[1,10)"`, `windows:"{[1,10)}"` | `:` checks containment of the supplied range or multirange. Quote the native literal. |
| JSON (`Json`) | `payload='{"enabled":true}'`, `payload:'{"enabled":true}'` | Quote valid JSON. `=` / `!=` compare values; `:` checks containment on PostgreSQL and MySQL. SQLite rejects containment. |
| PostgreSQL JSONB (`Jsonb`) | `payload.enabled=true`, `payload.count>=10`, `payload:'{"enabled":true}'` | Dotted paths compare JSON values; `:` checks JSONB containment. Unquoted booleans/numbers stay JSON booleans/numbers; quoted scalar values become JSON strings. |
| Nullable column (`Nullable<T>`, usually `Option<T>`) | `note=null`, `note!=null`, `note:*` | Unquoted `null` checks SQL NULL; `!=null` and `:*` check for a non-NULL value. Ordinary values follow the underlying type's rules. |
| Custom SQL type / enum | `route_type=3`, `route_type>=100` | `Aip160PgEnum` wrappers reuse their conversions and PostgreSQL enum ordering; other custom fields use an explicit adapter. See [custom fields and PostgreSQL enums](#custom-fields-and-postgresql-enums). |

Use `=` / `!=` for equality and `<`, `<=`, `>`, `>=` for ordering where the
type and backend support it. `:` is type-specific: text substring search,
signed 32/64-bit integer equality, or container containment. It is not a general
alias for equality on booleans, floats, decimals, or other scalar types.
The unquoted presence check `field:*` is handled separately.

For example, a model with text `name`, boolean `enabled`, integer `age`,
decimal `amount`, timestamp `created_at`, and nullable text `note` can combine
typed predicates in one filter:

```text
name:ali AND enabled=true AND age>=18 AND amount<100.00 AND created_at>="2026-10-17T00:00:00" AND note:*
```

Quoting changes literal interpretation: `age=18` is valid but `age="18"` is
rejected; `note=null` checks SQL NULL while `note="null"` matches text.
For JSONB, `payload.count=10` compares a JSON number while
`payload.count="10"` compares a JSON string. A dotted JSONB `=null` compares
JSON null, and dotted `:*` checks path existence, including JSON null.

### Equivalent SQL by data type

Each SQL fragment below can be used after `WHERE`. Semicolon-separated
fragments correspond to separate filter examples, in the same order as the
examples above. Values are shown inline for readability; the library binds
values and JSON paths as parameters. Identifier quoting, table qualifiers,
parentheses, and some casts are omitted. PostgreSQL syntax is used unless a
backend is named explicitly.

| Data type | Equivalent SQL predicates |
| --- | --- |
| Text | `name = 'Alice'`; `name = 'Alice Smith'`; `LOWER(name) LIKE LOWER('%ali%') ESCAPE E'\\'`; `name LIKE 'Ali%' ESCAPE E'\\'` |
| Boolean | `enabled = TRUE`; `enabled != FALSE` |
| Signed integer | `age >= 18`; `balance < 0`; `(id = 1 OR id = 2)` |
| MySQL unsigned integer | `code = 65535`; `code >= 100` |
| Floating point | `score >= 1.25`; `ratio < 1e-3` |
| Decimal | PostgreSQL: `amount >= CAST('19.95' AS numeric)`; MySQL: `amount >= CAST('19.95' AS DECIMAL(65,2))`; SQLite: `amount >= 19.95` |
| Date | `day = CAST('2026-10-17' AS date)` |
| Time | `clock >= CAST('12:34:56' AS time)` |
| Timestamp | `created_at >= CAST('2026-10-17T12:34:56' AS timestamp)` |
| Timestamp with timezone | `created_at >= CAST('2026-10-17T12:34:56Z' AS timestamptz)`; `created_at < CAST('2026-10-18T00:00:00+08:00' AS timestamptz)` |
| Binary | PostgreSQL: `bytes = decode('deadbeef', 'hex')`; MySQL / SQLite: `bytes = X'deadbeef'`. The library binds decoded bytes directly. |
| UUID | `key = CAST('550e8400-e29b-41d4-a716-446655440000' AS uuid)` |
| Network | For `Inet`: `network = CAST('192.168.1.0/24' AS inet)`; for `Cidr`, use `cidr` instead. |
| PostgreSQL array | For `Array<Text>`: `tags = CAST('{rust,diesel}' AS text[])`; `tags @> CAST('{rust}' AS text[])` |
| PostgreSQL range / multirange | For integer ranges: `"window" @> CAST('[1,10)' AS int4range)`; `windows @> CAST('{[1,10)}' AS int4multirange)` |
| JSON (PostgreSQL) | `CAST(payload AS jsonb) = CAST('{"enabled":true}' AS jsonb)`; `CAST(payload AS jsonb) @> CAST('{"enabled":true}' AS jsonb)` |
| JSON (MySQL) | `payload = CAST('{"enabled":true}' AS JSON)`; `JSON_CONTAINS(payload, CAST('{"enabled":true}' AS JSON))` |
| JSON (SQLite) | `json(payload) = json('{"enabled":true}')`; containment is rejected. |
| PostgreSQL JSONB | `(payload #> ARRAY['enabled']) = 'true'::jsonb`; `(payload #> ARRAY['count']) >= '10'::jsonb`; `(payload #> ARRAY[]::text[]) @> '{"enabled":true}'::jsonb` |
| Nullable column | `note IS NULL`; `note IS NOT NULL`; `note IS NOT NULL` |
| Custom SQL type / enum | With the example `NumericRoute` adapter: `CAST(CAST(route_type AS TEXT) AS BIGINT) = 3`; `CAST(CAST(route_type AS TEXT) AS BIGINT) >= 100`. Other adapters define their own SQL. |

For MySQL temporal columns, the generated value casts use `DATE`, `TIME(6)`,
and `DATETIME(6)`. SQLite binds temporal strings normalized to Diesel's storage
format when the matching `chrono` or `time` feature is enabled. SQLite and MySQL
bind booleans as `1` / `0`. Text search on those backends uses the same
`LOWER(...) LIKE LOWER(...)` expression with a bound backslash escape character
instead of PostgreSQL's `E'\\'` notation.

Additional PostgreSQL native types from the literal-format table translate as
follows:

| Filter | Equivalent SQL predicate |
| --- | --- |
| `duration="1 day 2 hours"` (`Interval`) | `duration = CAST('1 day 2 hours' AS interval)` |
| `price="$12.34"` (`Money`) | `price = CAST('$12.34' AS money)` |
| `mac="08:00:2b:01:02:03"` (`MacAddr`) | `mac = CAST('08:00:2b:01:02:03' AS macaddr)` |
| `position="0/16B6C50"` (`PgLsn`) | `position = CAST('0/16B6C50' AS pg_lsn)` |
| `pair='(7,"seven")'` (`Record<(Integer, Text)>`) | `pair = ROW(7, 'seven')` |
| `name=alice` (`Citext`) | `name = CAST('alice' AS citext)`; the database's `citext` operators provide case-insensitive equality. |
| `flag="a"` (`CChar`) | `flag = CAST('a' AS "char")`; the library binds the single byte using Diesel's `CChar` type. |

JSONB quoting and presence checks have distinct SQL meanings:

| Filter | Equivalent SQL predicate |
| --- | --- |
| `payload.count=10` | `(payload #> ARRAY['count']) = '10'::jsonb` |
| `payload.count="10"` | `(payload #> ARRAY['count']) = '"10"'::jsonb` |
| `payload.count=null` | `(payload #> ARRAY['count']) = 'null'::jsonb` |
| `payload.count:*` | `(payload #> ARRAY['count']) IS NOT NULL` |
| `payload=null` (nullable JSONB column) | `payload IS NULL` |

The combined filter above is equivalent to this PostgreSQL query predicate:

```sql
WHERE LOWER(name) LIKE LOWER('%ali%') ESCAPE E'\\'
  AND enabled = TRUE
  AND age >= 18
  AND amount < CAST('100.00' AS numeric)
  AND created_at >= CAST('2026-10-17T00:00:00' AS timestamp)
  AND note IS NOT NULL
```

## PostgreSQL JSONB

Enable `postgres` and use `Aip160Jsonb` to map fields in a JSONB value to stored
keys. For a Diesel table named `records` with `id -> Integer` and
`payload -> Jsonb`:

```rust
# #[cfg(feature = "postgres")]
# mod example {

# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, Aip160Jsonb, Backend};
diesel::table! { records (id) { id -> Integer, payload -> Jsonb } }

#[derive(Aip160Jsonb)]
struct Payload {
    #[serde(rename = "displayName")]
    display_name: String,
    details: diesel_aip_160::serde_json::Value,
}
#[derive(Aip160Filter)]
#[diesel(table_name = records)]
struct Record { id: i32, payload: Payload }

let predicate = Record::compile_filter("payload.display_name=Alice")?.unwrap();
let query = records::table.filter(predicate);
println!("{}", diesel::debug_query::<Backend, _>(&query));
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(feature = "postgres")]
# example::run()?;
# Ok(())
# }
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
struct-level `rename_all` is rejected. Dotted JSONB filtering requires PostgreSQL; SQLite JSONB supports root
comparisons, as described in the native-type reference. See the [JSONB tests](https://docs.rs/crate/diesel-aip-160/latest/source/tests/derive.rs) for a complete
schema and query examples.

## Custom fields and PostgreSQL enums

For local wrappers around dependency enums, derive
[`Aip160PgEnum`](#enum-codec-reference) to generate
PostgreSQL `FromSql`/`ToSql` implementations using a label value type and two
fallible conversion functions. Your application owns the enum mapping and
validation. Models using these wrappers (or `Option<Wrapper>`) filter automatically
without field attributes. Literal values go through the wrapper's decoder and
are bound as the enum SQL type; ordering follows PostgreSQL declaration order.
Use a field adapter to override this behavior, for example for numeric ordering.

Parsing an enum label as a numeric `value_type` validates its code but does
not change PostgreSQL's comparison order. For custom validation, supply a field
adapter. This standalone example uses an integer SQL column and an
application-specific Rust field type:

```rust
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# mod example {

# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, FieldAdapter, Predicate};
use diesel_aip_160::{compiler::ComparisonOperator, parser::Value};
diesel::table! { routes (id) { id -> Integer, route_type -> Integer } }
struct RouteCode;
struct ValidatedRoute;
impl FieldAdapter<routes::route_type> for ValidatedRoute {
    fn compare(column: routes::route_type, op: ComparisonOperator, value: &Value)
        -> diesel_aip_160::anyhow::Result<Predicate<routes::table>>
    {
        diesel_aip_160::anyhow::ensure!(!value.quoted, "expected an unquoted route code");
        let code: i32 = value.text.parse()?;
        diesel_aip_160::anyhow::ensure!([3, 100, 1702].contains(&code), "unknown route code");
        match op {
            ComparisonOperator::Equal => Ok(Box::new(column.eq(code))),
            ComparisonOperator::LessThan => Ok(Box::new(column.lt(code))),
            _ => diesel_aip_160::anyhow::bail!("unsupported route comparison"),
        }
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = routes)]
struct Route {
    #[aip160(adapter = ValidatedRoute)]
    route_type: RouteCode,
}
let predicate = Route::compile_filter("route_type<100")?.unwrap();
let query = routes::table.filter(predicate);
# let _ = query;
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# example::run()?;
# Ok(())
# }
```

Implement `diesel_helpers::FieldAdapter<Column>` for the adapter. Its
`compare(column, ComparisonOperator, &parser::Value)` returns
`anyhow::Result<Predicate<Column::Table>>`. The column is the statically
registered Diesel column; `Value` preserves literal text and quoting. Validate
and map the literal before building typed Diesel expressions. Adapters can
reject unsupported operators. `:` maps to equality for custom fields; text
wildcard and substring handling never applies. Custom fields without a search adapter are excluded from
global text search, and unknown fields or traversals are rejected.

For nullable adapter fields, the compiler allows unquoted `null` only with
`=`/`!=`, and sends `:*` as `!=null`. The adapter must emit `IS NULL`/`IS NOT
NULL` for these requests. Nonnullable `:*` is constant true. Quoted `"null"`
remains an ordinary adapter literal. Compare nullable values with Diesel's
`assume_not_null()` expression wrapper when needed; it changes Rust typing
without changing SQL NULL semantics.

For actual PostgreSQL enum columns, the complete, compiling [PostgreSQL enum example](https://docs.rs/crate/diesel-aip-160/latest/source/examples/postgres_enums.rs)
shows `Aip160PgEnum` codecs and automatic filtering of stored GTFS labels in
`NativeRoute`. Its `Route` model overrides filtering with numeric route ordering
for labels `3`, `100`, `1702`, nullable BIGINT `order`, and application mappings
from protobuf pickup/drop-off literals `1`–`4` to stored
GTFS labels `0`–`3`. The numeric adapter explicitly casts enum → text → BIGINT
using a fixed application SQL expression, so `3 < 100 < 1702` regardless of
PostgreSQL enum declaration order. The pickup adapter binds a value of the
custom PostgreSQL SQL type and deliberately rejects ordering. For declaration
ordering instead, an adapter can use typed enum `.lt()`/`.gt()` expressions.

With the test declaration `('1702', '3', '100')`, the two models differ:

| Filter | `NativeRoute` (stored labels) | `Route` (application adapters) |
| --- | --- | --- |
| `route_type<100` | Matches labels `1702` and `3` | Matches only label `3` |
| `route_type='3'` | Accepts the quoted label | Rejects quoted numeric codes |
| `continuous_pickup=1` | Matches stored label `1` | Matches stored label `0` |
| `continuous_pickup>1` | Uses PostgreSQL enum order | Rejects ordering |
| `continuous_drop_off=null` | Checks `IS NULL` | Checks `IS NULL` |
| `continuous_drop_off:*` | Checks `IS NOT NULL` | Checks `IS NOT NULL` |

Native enum filtering supports all six comparison operators and `:` as equality,
accepting quoted or unquoted labels through the codec. The example's adapters
require unquoted codes: `NumericRoute` accepts only `3`, `100`, and `1702`, and
`ProtobufPickup` accepts only `1` through `4`. Quoted `null` fails validation
in both models. Ordinary comparisons retain SQL NULL semantics under `NOT`;
for example, `NOT continuous_drop_off=1` does not select NULL rows.

All values must be bound parameters. Never concatenate user literals, labels,
paths, or field names into SQL. Adapter SQL expressions are trusted application
code, and the application owns cast validity for its entire stored enum domain.
For example, a numeric adapter is inappropriate for enums with nonnumeric
stored labels. The library contains no GTFS mappings or enum ordering policy.

Existing JSONB structs continue to opt in with `Aip160Jsonb`/`JsonbPath` (and
`serde_json::Value` remains supported). Unknown custom SQL types require an
adapter or `skip`; built-in SQL types dispatch directly from the schema.

Run the enum SQL-generation tests without a database:

```sh
cargo test -p diesel-aip-160 --features postgres --example postgres_enums
cargo run -p diesel-aip-160 --features postgres --example postgres_enums
```

An optional ignored integration test executes Diesel queries against PostgreSQL
with deliberately reordered enum declarations. The Rust schema declarations do
not create database objects. The test creates session-local types
and a temporary table only. It requires native libpq and an explicitly selected
test database:

```sh
AIP160_TEST_DATABASE_URL=postgres://user:password@localhost/test_db \
  cargo test -p diesel-aip-160 --features postgres-integration \
  --example postgres_enums postgres_executes -- --ignored
```

`postgres` continues to enable SQL generation without native libraries;
`postgres-integration` additionally enables Diesel's PostgreSQL connection driver.

### Enum codec reference

For a local single-field tuple wrapper, derive `Aip160PgEnum` together with
Diesel's `AsExpression` and `FromSqlRow`. The required attributes are:

```text
#[derive(Debug, diesel::expression::AsExpression, diesel::deserialize::FromSqlRow,
         diesel_aip_160::Aip160PgEnum)]
#[diesel(sql_type = RouteTypeEnum)]
#[aip160(value_type = i16, from_value = route_type_from_code, to_value = route_type_to_code)]
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

The derive accepts wrappers without `Copy` or `Clone` and serializes by borrowing
the inner value. Nullable wrappers, aliases, and qualified wrapper paths are
supported by `Aip160Filter`. It does not generate the SQL enum declaration or
choose label mappings. The complete source example linked above includes both
codec generation and custom numeric-ordering adapters.

## Opt-in full-text search

Unconfigured models retain the existing escaped, case-insensitive substring
search. Field search (`title:diesel`) and global search (`diesel`) have separate
compiler hooks. The default global hook calls field search for every registered
searchable field and ORs the predicates. Thus it honors each field's search
configuration. A global adapter replaces this expansion entirely, even on a
model with no searchable fields.

Built-in full-text search currently supports **PostgreSQL only**. Enable the
`postgres` feature and opt individual `String` / `Option<String>` fields in:

```rust
# #[cfg(feature = "postgres")]
# mod example {

# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, Backend};
diesel::table! { articles (id) { id -> Integer, title -> Nullable<Text>, body -> Nullable<Text>, published -> Bool } }
#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
struct Article {
    id: i32,
    #[aip160(search(postgres, configuration = "english"))]
    title: Option<String>,
    body: Option<String>,
    published: bool,
}
let predicate = Article::compile_filter(
    r#"title:"diesel engine" AND body:manual AND published=true AND id>=10"#
)?.unwrap();
let query = articles::table.filter(predicate).select(articles::id);
println!("{}", diesel::debug_query::<Backend, _>(&query));
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(feature = "postgres")]
# example::run()?;
# Ok(())
# }
```

The complete compiling [PostgreSQL search example](https://docs.rs/crate/diesel-aip-160/latest/source/examples/postgres_search.rs)
adds a global adapter over a combined document and prints bound SQL:

```sh
cargo run -p diesel-aip-160 --example postgres_search --features postgres
```

The built-in predicate targets `to_tsvector('english'::regconfig, articles.title)`.
For an expression index, the application must create the **same expression and
configuration**. It does not search a separate vector automatically. Example
application-owned SQL (adapt names/types to your schema):

```sql
CREATE TABLE articles (
    id integer PRIMARY KEY, title text, body text, published boolean NOT NULL,
    document tsvector GENERATED ALWAYS AS (
        to_tsvector('english'::regconfig,
            coalesce(title, '') || ' ' || coalesce(body, ''))
    ) STORED
);
CREATE INDEX articles_title_fts ON articles USING GIN
    (to_tsvector('english'::regconfig, title));
CREATE INDEX articles_document_fts ON articles USING GIN (document);
```

To explicitly search an existing vector for a field, use
`#[aip160(search(postgres, configuration = "english", vector = articles::document))]`.
Declare that schema column as `search::postgres::TsVector` or
`Nullable<TsVector>`. The field itself remains a `String`/`Option<String>` with
ordinary comparisons and presence checks against its original column. The
`vector` target must belong to the model's table. The query configuration must
match the document's construction configuration; the compiler cannot verify
database setup. Static configuration names accept ASCII letters, digits,
underscores, and dots (for schema qualification).

PostgreSQL uses `plainto_tsquery` for unquoted literals and `phraseto_tsquery` for
quoted literals. A quoted multiword value means an ordered lexeme phrase,
including stop-word distances, rather than an exact byte substring. Stemming,
stop words, and tokenization follow the explicit configuration. Empty or
stop-word-only queries match no rows. User punctuation is processed as plain
text, without interpreting tsquery operators, prefixes, or wildcards.

The filter grammar is unchanged: `title:"diesel engine"` is one phrase search;
`title:diesel engine` is a title term AND a separate global term. Both quote
styles supported by the parser set the phrase flag. AND, OR, NOT, grouping,
composite field arguments, implicit AND, and AIP-160's OR-before-AND precedence
are unchanged. Unconfigured fields still treat quoted multiword searches as
substrings.

Only search changes. Equality, inequality, ordering, wildcard equality, NULL
checks, `field:*`, and nontext containment retain their behavior. Unquoted
`field:null` stays on the comparison path (and remains invalid for nullable
text); quoted `field:"null"` searches text. Required text retains its previous
literal `field:null` behavior. Global `null` and `*` remain search terms.
NULL search targets produce SQL NULL, also under NOT, so neither a search nor
its negation selects a NULL target. A global OR may still match another field.
Applications may deliberately build combined documents with `coalesce`, as
above, giving that document empty text for missing source columns.

### Custom search adapters

`diesel_helpers::SearchAdapter<Table>` is independent of `FieldAdapter<Column>`.
It receives `&parser::Value` with original text and quoting and returns
`anyhow::Result<Predicate<Table>>`, a typed, boxed Diesel boolean expression.
Use `#[aip160(search_adapter = MySearch)]` on a field, or
`#[aip160(global_search_adapter = CombinedDocument)]` on a model:

```rust
# #[cfg(feature = "postgres")]
# mod example {

# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
use diesel_aip_160::{Aip160Filter, Predicate, SearchAdapter, CompiledQueryFilter};
use diesel_aip_160::{parser::Value, search::postgres};
diesel::table! {
    use diesel::sql_types::*;
    use diesel_aip_160::search::postgres::TsVector;
    articles (id) { id -> Integer, document -> Nullable<TsVector> }
}
struct CombinedDocument;
impl SearchAdapter<articles::table> for CombinedDocument {
    fn search(value: &Value) -> diesel_aip_160::anyhow::Result<Predicate<articles::table>> {
        postgres::vector(articles::document, "english", value)
    }
    fn search_query(value: &Value) -> diesel_aip_160::anyhow::Result<CompiledQueryFilter<articles::table>> {
        postgres::vector_query(articles::document, "english", value)
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = articles)]
#[aip160(global_search_adapter = CombinedDocument)]
struct Article { id: i32 }
let query = Article::compile_query_filter("diesel")?.unwrap();
assert_eq!(query.signals.len(), 1);
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(feature = "postgres")]
# example::run()?;
# Ok(())
# }
```

Adapters can choose other registered targets or application-defined expressions
and query semantics. They must bind input and preserve SQL NULL behavior;
identifiers and SQL structure must come from trusted application code. Nullable
Diesel expressions can use `assume_not_null()` for the boxed Bool metadata; this
does not alter SQL NULL propagation. Search never passes through
`ComparisonOperator`, which continues to map `:` to equality for custom field
comparisons without a search adapter. Search adapters opt their field into
default global search, including fields also using `adapter = MyFieldAdapter`.
Unquoted `:*` and `:null` bypass field search adapters and retain comparison
validation. Built-in search cannot be combined with `adapter` or
`search_adapter`; `skip` cannot be combined with any adapter/search option.
Other field types require custom search adapters.

Custom adapters work on PostgreSQL, MySQL, and SQLite. Built-in `search(mysql, ...)`
and `search(sqlite, ...)` are rejected, as is a PostgreSQL search configuration
compiled with a different backend; explicit full-text requests never silently
fall back to substring matching. MySQL requires a suitable FULLTEXT index and
MATCH column list. SQLite FTS5 requires an FTS virtual table and MATCH target,
usually related to source rows through rowid. Implement those application-specific
targets in an adapter; binding query text does not neutralize the backend's own
query language, so adapters must define and validate their query semantics.

Official backend requirements:
[PostgreSQL query semantics](https://www.postgresql.org/docs/current/textsearch-controls.html),
[PostgreSQL tables and indexes](https://www.postgresql.org/docs/current/textsearch-tables.html),
[MySQL FULLTEXT/MATCH](https://dev.mysql.com/doc/refman/8.4/en/fulltext-natural-language.html),
and [SQLite FTS5](https://www.sqlite.org/fts5.html).
The compiler creates no migrations, indexes, or search documents and manages no
maintenance triggers. Ranking signals are opt-in; result ordering remains explicit.

## Ranking signals and ordering

`compile_filter()` remains unchanged. `compile_query_filter()` is opt-in and
returns `Option<ranking::CompiledQueryFilter<Table>>`: a `predicate` plus
`signals`. Empty input returns `None`. Parsing and validation use the same
compiler traversal and derived field registry. A predicate decides which rows
match; a signal describes a metric; an ordering policy selects metrics and their
priority. No ordering is applied automatically, including for filters that emit
signals. Filters without signals retain their existing predicates.

A search annotation such as
`#[aip160(search(postgres, configuration = "english"))]` emits a signal named
`relevance` with SQL type `Float`, preferred direction `Descending`, and operation
`postgres.full_text`. It uses `ts_rank_cd` with exactly the matching target,
configuration, and query constructor (`phraseto_tsquery` for quoted literals,
`plainto_tsquery` for unquoted literals). `vector = articles::document` also
ranks that stored vector. All user values are bound. Scores use
`COALESCE(..., 0.0::real)` so missing text scores zero rather than sorting first;
predicates still preserve SQL NULL, including under NOT. Default substring
search, equality, presence, and JSON containment emit no built-in signals.

Migrate a full-text query from:

```rust
# #[cfg(feature = "postgres")]
# mod example {
# use diesel::prelude::*;
# use diesel_aip_160::{Aip160Filter, Backend};
# diesel::table! { articles (id) { id -> Integer, title -> Nullable<Text>, body -> Nullable<Text>, published -> Bool } }
# #[derive(Aip160Filter)]
# #[diesel(table_name = articles)]
# struct Article {
#     id: i32,
#     #[aip160(search(postgres, configuration = "english"))]
#     title: Option<String>,
#     body: Option<String>,
#     published: bool,
# }
# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
let predicate = Article::compile_filter("title:diesel")?.unwrap();
let query = articles::table.filter(predicate);
# let _ = query;
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(feature = "postgres")]
# example::run()?;
# Ok(())
# }
```

to explicit relevance ordering:

```rust
# #[cfg(feature = "postgres")]
# mod example {
# use diesel::prelude::*;
# use diesel_aip_160::{Aip160Filter, Backend};
# diesel::table! { articles (id) { id -> Integer, title -> Nullable<Text>, body -> Nullable<Text>, published -> Bool } }
# #[derive(Aip160Filter)]
# #[diesel(table_name = articles)]
# struct Article {
#     id: i32,
#     #[aip160(search(postgres, configuration = "english"))]
#     title: Option<String>,
#     body: Option<String>,
#     published: bool,
# }
# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
let mut compiled = Article::compile_query_filter("title:diesel")?.unwrap();
let order = compiled.ordering(&["relevance"])?;
let query = articles::table
    .filter(compiled.predicate)
    .order(order)
    .then_order_by(articles::id) // Application-owned stable tie-breaker.
    .limit(20)
    .offset(40);
# let _ = query;
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(feature = "postgres")]
# example::run()?;
# Ok(())
# }
```

Signals expose `name`, `direction`, and `operation`; `sql_type()` reports the
immutable Diesel SQL type metadata. Applications
can inspect or consume the public `signals` vector and build their own ordering:

```rust
# #[cfg(feature = "postgres")]
# mod example {
# use diesel::prelude::*;
# use diesel_aip_160::{Aip160Filter, Backend};
# diesel::table! { articles (id) { id -> Integer, title -> Nullable<Text>, body -> Nullable<Text>, published -> Bool } }
# #[derive(Aip160Filter)]
# #[diesel(table_name = articles)]
# struct Article {
#     id: i32,
#     #[aip160(search(postgres, configuration = "english"))]
#     title: Option<String>,
#     body: Option<String>,
#     published: bool,
# }
# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
let mut compiled = Article::compile_query_filter("title:diesel")?.unwrap();
let relevance = compiled.take_signal("relevance")?
    .into_expression::<diesel::sql_types::Float>()?;
let query = articles::table.filter(compiled.predicate)
    .order(relevance.desc()).then_order_by(articles::id).limit(20);
# let _ = query;
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(feature = "postgres")]
# example::run()?;
# Ok(())
# }
```

The SQL type must match exactly; `into_expression` returns an error for a
mismatch. No SQL casts or numeric conversions are added. `ordering(&["name"])`
selects one metric; `ordering(&["distance", "relevance"])` orders by ascending
distance first, then descending relevance. Each uses the signal's preferred
direction. Empty policies, repeated policy names, missing names, and ambiguous
signal names are errors; invalid policies do not consume signals. Signals with
the same name from custom operations remain independent and must be resolved
by the application (for example, select a contribution from `signals` or rename
it before building the policy). A custom adapter that reuses an already compiled
composite score returns that score as an independent metric; it is not treated
as a single raw full-text leaf or silently combined with another compiler's
`relevance` signal.

### Ranking SQL

For `title:diesel`, selecting `relevance` and appending an ID tie-breaker
produces this PostgreSQL query shape. Query text is bound separately where the
predicate and ranking expression appear; it is never inserted into SQL:

```sql
SELECT articles.id
FROM articles
WHERE to_tsvector('english'::regconfig, articles.title)
      @@ plainto_tsquery('english'::regconfig, $1)
ORDER BY COALESCE(ts_rank_cd(
    to_tsvector('english'::regconfig, articles.title),
    plainto_tsquery('english'::regconfig, $2)
), 0.0::real) DESC, articles.id ASC
LIMIT $3 OFFSET $4;
-- binds: ["diesel", "diesel", 20, 40]
```

Quoted `title:"diesel engine"` uses `phraseto_tsquery` in both expressions.
Stored-vector search uses `articles.document` in both places instead of
`to_tsvector(...)`. Multiple distinct positive searches produce
`GREATEST(COALESCE(ts_rank_cd(...), 0.0::real), ...)`; repeated identical searches
are removed before SQL generation. This is a strongest-match score with no
term/field coverage bonus.

The distance/label example below has this portable ordering shape:

```sql
SELECT places.id, places.label
FROM places
WHERE places.id >= $1
ORDER BY abs(places.id - $2) ASC, places.label DESC, places.id ASC
LIMIT $3 OFFSET $4;
-- binds: [0, 7, 20, 40]
```

PostgreSQL uses numbered placeholders; MySQL and SQLite use `?`. The generic
ordering layer adds directions and priorities; metric SQL comes from the typed
adapter expression. No user-defined SQL renderer is required.

### Boolean policy

- AND and OR concatenate leaf contributions. They do not infer a common scale,
  combine unrelated metrics, or gate signals by the truth of a branch.
- Signals from **unmatched OR branches contribute**, as do signals inside a
  failed AND branch when another OR branch admits the row. This avoids imposing
  text-specific zero/fallback rules on distance, similarity, or other adapters.
  A custom metric may itself include a typed conditional expression if needed.
- NOT toggles signal polarity; odd negation drops contributions and double
  negation restores them. Parenthesized restrictions such as
  `title:(diesel OR NOT engine)` follow the same rule. Predicate truth semantics
  and SQL three-valued logic are unchanged.
- The built-in text policy alone takes the maximum positive full-text score as one
  `relevance` signal. It deduplicates identical target/configuration/literal/
  quoting contributions, including global/field overlap and fields sharing a
  stored vector. Distinct contributions use `GREATEST`, so equivalent quoted,
  case-folded, or stemmed queries cannot inflate the score even when their
  literal spellings differ. This strongest-match policy also avoids rewarding
  repeated matches across fields; it does not add term/field coverage bonuses.
  Negative terms never boost relevance.
- Default global search expands the registered searchable fields and collects
  their signals, retaining each field's configuration. An explicit global
  adapter replaces that expansion and owns its scoring policy.

### Custom metrics

Existing `SearchAdapter::search` and `FieldAdapter::compare` implementations
continue to work unchanged and emit no signals. Override `search_query` or
`compare_query` to return `compiler::QueryFilter { predicate, signals }`.
`RankingSignal::new` accepts a typed nonaggregate Diesel expression on the
model's table. Different signals can use different SQL types, including custom
SQL types; no `QueryFragment` implementation is required from applications.
Adapters must bind values and define their metric's NULL handling and meaning
on rows that do not satisfy that leaf.

For example, a one-dimensional distance adapter can use ordinary integer
expressions, with no spatial extension:

```rust
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# mod example {

# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
use diesel::prelude::*;
use diesel_aip_160::{Aip160Filter, CompiledQueryFilter, Direction, Predicate,
    QueryFilter, RankingSignal, SearchAdapter, parser::Value};
diesel::table! { places (id) { id -> Integer, label -> Text } }
diesel::define_sql_function! {
    fn abs(value: diesel::sql_types::Integer) -> diesel::sql_types::Integer;
}
struct Distance;
impl SearchAdapter<places::table> for Distance {
    fn search(_: &Value) -> diesel_aip_160::anyhow::Result<Predicate<places::table>> {
        Ok(Box::new(places::id.ge(0)))
    }
    fn search_query(value: &Value) -> diesel_aip_160::anyhow::Result<CompiledQueryFilter<places::table>> {
        let center: i32 = value.text.parse()?;
        Ok(QueryFilter {
            predicate: Self::search(value)?,
            signals: vec![
                RankingSignal::new("distance", abs(places::id - center),
                    Direction::Ascending, "demo.distance"),
                RankingSignal::new("label", places::label,
                    Direction::Descending, "demo.label"),
            ],
        })
    }
}
#[derive(Aip160Filter)]
#[diesel(table_name = places)]
#[aip160(global_search_adapter = Distance)]
struct Place { id: i32 }
let mut compiled = Place::compile_query_filter("7")?.unwrap();
let order = compiled.ordering(&["distance", "label"])?;
let query = places::table.filter(compiled.predicate)
    .order(order).then_order_by(places::id).limit(20).offset(40);
# let _ = query;
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# example::run()?;
# Ok(())
# }
```

A caller selects `ordering(&["distance"])`, or prioritizes several independent
metrics, and appends its own ID tie-breaker before limit/offset. See
[the generic adapter tests](https://docs.rs/crate/diesel-aip-160/latest/source/tests/ranking_adapters.rs) for a
compiling distance/priority example and SQLite execution, and
[the PostgreSQL search example](https://docs.rs/crate/diesel-aip-160/latest/source/examples/postgres_search.rs) for
full-text migration and a ranked global stored-vector adapter.

Fuzzy matching, geographic distance, vector similarity, and time proximity can
use these adapter hooks; production implementations are not supplied. Weighted
combinations, normalization, automatic explicit-user-order reconciliation,
pagination policy, and stable tie-breakers are outside this API. PostgreSQL
built-ins require the `postgres` feature; custom metrics work on each backend.
For non-Diesel compilers, `compiler::QueryFilterCompiler` provides the same
opt-in hooks with an application-chosen signal type and finalization policy;
`compiler::compile_query` reuses `Aip160FilterCompiler` dispatch.

## Compiler API

Parsing is also available without a backend or Diesel dependency:

```rust
use diesel_aip_160::parser::parse_filter;
let expression = parse_filter("name=Alice AND id>=10")?;
assert!(expression.is_some());
assert!(parse_filter("   ")?.is_none());
# Ok::<(), diesel_aip_160::anyhow::Error>(())
```

Parsing preserves literal spelling and quoting; compilation interprets values
against registered fields and their SQL types.

[`compiler.rs`](https://docs.rs/crate/diesel-aip-160/latest/source/src/compiler.rs) exposes
`compile(source, &compiler) -> anyhow::Result<Option<C::Predicate>>` and the
`Aip160FilterCompiler` trait. `Aip160Filter` generates a compiler named after
the model, such as `RecordFilterCompiler`. In the SQLite example, the direct
equivalent of `Record::compile_filter(filter)` is:

```rust
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# mod example {
# use diesel::prelude::*;
# use diesel_aip_160::{Aip160Filter, Backend};
# diesel::table! { records (id) { id -> Integer, name -> Text, note -> Nullable<Text> } }
# #[derive(Aip160Filter)]
# #[diesel(table_name = records)]
# struct Record { id: i32, name: String, note: Option<String> }
# pub fn run() -> diesel_aip_160::anyhow::Result<()> {
let predicate = diesel_aip_160::compiler::compile(
    "name=Alice AND id>=10", &RecordFilterCompiler,
)?;
assert!(predicate.is_some());
# Ok(())
# }
# }
# fn main() -> diesel_aip_160::anyhow::Result<()> {
# #[cfg(any(feature = "sqlite", feature = "mysql", feature = "postgres"))]
# example::run()?;
# Ok(())
# }
```

For a custom backend, implement `Aip160FilterCompiler` with your own predicate
type. Supply field types, searchable text fields, constants, and logical
composition. Return `FieldType::Native` for built-in fields and implement
`native_compare` to interpret the original `parser::Comparator` and
`parser::Value`, including quoting, wildcard equality, containment, and NULL
semantics. Implement `ilike` for default substring searches. The default `field_search` and
`global_search` methods preserve compatibility; override them to receive the
original `parser::Value`, including its quoting flag. Application-defined
fields use `FieldType::Custom { nullable }` and `custom_compare`. See the
[changelog](https://github.com/comradesharf/diesel-aip-160/blob/main/CHANGELOG.md) for beta migration notes.

## Validation and limits

The parser accepts at most 16,384 input bytes, 512 tokens, and 32 levels of
nesting. Errors are returned as `anyhow::Error`; syntax diagnostics include byte
offsets. Empty input is valid and returns `None`. Field names must be registered
by the derive; values, JSON paths, search terms, and adapter literals are bound.
Field names from a filter never become free-form SQL identifiers.

Compilation validates syntax, registered fields, operator compatibility, NULL
rules, and supported literal formats. PostgreSQL native complex literal formats
and configuration existence can require database validation at execution time.
A SQL error is therefore still possible after successful compilation.
`table!` declarations, derives, and compilation create no database objects.

Known boundaries:

- Use one backend per build. There is no cross-backend runtime connection switch.
- AIP-160 functions, CEL operators, and field-to-field comparisons are rejected.
- Derive models require named fields and currently do not support generics.
  Fields map to their Rust names; custom SQL column renaming is not inferred.
- `Aip160Jsonb` supports direct field `serde(rename = "...")`, not `rename_all`,
  `flatten`, or serialization-direction-specific renames. It maps the first path
  segment only. Do not rely on Serde skipped fields being automatically excluded.
- Derive expansion currently expects the dependency to be named `diesel_aip_160`;
  Cargo dependency aliases are not resolved.
- The SQL types are selected from the Diesel schema. Application adapters own
  domain validation, casts, NULL scoring, and operation-specific query languages.
- Built-in full-text relevance is PostgreSQL-only. Spatial/vector/fuzzy/time
  metrics require custom adapters; weighting and normalization are deferred.

## Run examples and tests

Run these development commands from a checkout of the workspace repository.

For local database development, start PostgreSQL and MySQL with Docker Compose:

```sh
docker compose up -d --wait
```

Both databases use database name, username, and password `aip160`. They are
available only on localhost with these connection URLs:

- PostgreSQL: `postgres://aip160:aip160@localhost:5432/aip160`
- MySQL: `mysql://aip160:aip160@localhost:3306/aip160`

These credentials are for local development. Data persists in named Docker
volumes across `docker compose down`. To delete the database data as well, run
`docker compose down -v`.

Use the PostgreSQL URL as `AIP160_TEST_DATABASE_URL` for the optional integration
test above.

The included examples print SQL and do not require a running database:

```sh
cargo run -p diesel-aip-160 --example sqlite --features sqlite
cargo run -p diesel-aip-160 --example mysql --features mysql
cargo run -p diesel-aip-160 --example postgres --features postgres
```

Run the complete native-type suite through the existing Docker Compose databases:

```sh
./scripts/test-native-types.sh
```

The script builds `Dockerfile.test`, starts healthy databases, and runs all tests
separately for PostgreSQL, MySQL, and SQLite using `compose.test.yaml`. The tests
execute bound Diesel queries against real databases, checking every native type
family, nullable values, precision boundaries, and containment. SQLite runs
inside the test container using an in-memory database and bundled SQLite
(`sqlite-integration`), including JSONB support (SQLite 3.45 or newer). Test containers are
removed after each run; database and build-cache volumes persist.

Run tests separately for each backend because backend features are mutually
exclusive:

```sh
cargo test --workspace
cargo test -p diesel-aip-160 --features sqlite
cargo test -p diesel-aip-160 --features mysql
cargo test -p diesel-aip-160 --features postgres
```

The repository pins its Rust toolchain in
[`rust-toolchain.toml`](https://github.com/comradesharf/diesel-aip-160/blob/main/rust-toolchain.toml).
