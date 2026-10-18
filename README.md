# diesel-aip-160

Parse AIP-160 filter strings, validate them against model fields, and compile
them into typed, parameterized Diesel predicates.

**Status: beta (`0.1.0-beta.6`).** Both crates share this prerelease version.
The public API and filter behavior may change between beta releases. Pin the
exact version for reproducible integrations and review changes before upgrading.

This Rust workspace contains:

- [`diesel-aip-160`](diesel-aip-160): the parser, compiler, and Diesel helpers.
- [`diesel-aip-160-derive`](diesel-aip-160-derive): the `Aip160Filter`,
  `Aip160Jsonb`, and `Aip160PgEnum` derive macros, re-exported by the main crate.

## Installation

Enable one database backend:

```toml
[dependencies]
diesel-aip-160 = { version = "=0.1.0-beta.6", features = ["sqlite"] }
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

The derive reads each schema column's Diesel SQL type. All built-in column type
families are filterable without an adapter, including nullable variants, aliases,
PostgreSQL arrays/ranges/multiranges/records, MySQL unsigned numbers, and SQLite
timestamps and JSONB. Exclude fields with `#[aip160(skip)]`. Application-defined
PostgreSQL enum wrappers deriving `Aip160PgEnum` support filtering automatically.
Other custom SQL types, including composites, use adapters.

## Native types and literal formats

Enable optional Rust representation integrations as needed:

```toml
diesel-aip-160 = { version = "=0.1.0-beta.6", features = ["postgres", "chrono", "uuid", "numeric", "ipnet-address"] }
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

## Custom fields and PostgreSQL enums

For local wrappers around dependency enums, derive
[`Aip160PgEnum`](diesel-aip-160-derive/README.md#aip160pgenum) to generate
PostgreSQL `FromSql`/`ToSql` implementations using a label value type and two
fallible conversion functions. Your application owns the enum mapping and
validation. Models using these wrappers (or `Option<Wrapper>`) filter automatically
without field attributes. Literal values go through the wrapper's decoder and
are bound as the enum SQL type; ordering follows PostgreSQL declaration order.
Use a field adapter to override this behavior, for example for numeric ordering.

Select an application adapter explicitly for numeric ordering. Parsing a label
as a numeric `value_type` validates its code but does not change PostgreSQL's
comparison order:

```rust,ignore
#[derive(Aip160Filter)]
#[diesel(table_name = routes)]
struct Route {
    #[aip160(adapter = NumericRoute)]
    route_type: RouteType,
    #[aip160(adapter = ProtobufPickup)]
    continuous_pickup: Pickup,
    order: Option<i64>,
}
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

The complete, compiling [PostgreSQL enum example](diesel-aip-160/examples/postgres_enums.rs)
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

## Opt-in full-text search

Unconfigured models retain the existing escaped, case-insensitive substring
search. Field search (`title:diesel`) and global search (`diesel`) have separate
compiler hooks. The default global hook calls field search for every registered
searchable field and ORs the predicates. Thus it honors each field's search
configuration. A global adapter replaces this expansion entirely, even on a
model with no searchable fields.

Built-in full-text search currently supports **PostgreSQL only**. Enable the
`postgres` feature and opt individual `String` / `Option<String>` fields in:

```rust,ignore
#[derive(diesel_aip_160::Aip160Filter)]
#[diesel(table_name = articles)]
struct Article {
    id: i32,
    #[aip160(search(postgres, configuration = "english"))]
    title: Option<String>,
    body: Option<String>, // body:manual still uses substring matching.
    published: bool,
}
let predicate = Article::compile_filter(
    r#"title:"diesel engine" AND body:manual AND published=true AND id>=10"#
)?.unwrap();
let query = articles::table.filter(predicate).select(articles::id);
```

The complete compiling [PostgreSQL search example](diesel-aip-160/examples/postgres_search.rs)
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

```rust,ignore
struct CombinedDocument;
impl diesel_aip_160::diesel_helpers::SearchAdapter<articles::table> for CombinedDocument {
    fn search(value: &diesel_aip_160::parser::Value)
        -> diesel_aip_160::anyhow::Result<diesel_aip_160::diesel_helpers::Predicate<articles::table>>
    {
        diesel_aip_160::search::postgres::vector(articles::document, "english", value)
    }
}
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
maintenance triggers, ranking scores, or result ordering.

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
type. Supply field types, searchable text fields, constants, and logical
composition. Return `FieldType::Native` for built-in fields and implement
`native_compare` to interpret the original `parser::Comparator` and
`parser::Value`, including quoting, wildcard equality, containment, and NULL
semantics. Implement `ilike` for default substring searches. The default `field_search` and
`global_search` methods preserve compatibility; override them to receive the
original `parser::Value`, including its quoting flag. Application-defined
fields use `FieldType::Custom { nullable }` and `custom_compare`. See the
[changelog](CHANGELOG.md) for beta migration notes.

## Run examples and tests

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
[`rust-toolchain.toml`](rust-toolchain.toml).
