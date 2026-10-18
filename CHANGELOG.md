# Changelog

Both workspace crates share the versions below. These beta releases may change
public APIs and filter behavior; pin the exact version when upgrading.

## Unreleased

### Added

- Opt-in `compile_query_filter()` compiles predicates and named, typed ranking
  signals in one traversal using the existing derived field registry.
- `RankingSignal`, typed expression recovery, and explicit single/priority
  ordering policies; adapters can opt in with `search_query` / `compare_query`.
- PostgreSQL `ts_rank_cd` relevance reuses matching configuration and targets,
  binds query values, scores NULL as zero, and uses strongest positive-match
  relevance to avoid duplicate boosts. NOT suppresses signals by polarity;
  unmatched OR branch signals remain available.
- Full-text and generic distance ordering examples, migration documentation,
  and SQL/SQLite/PostgreSQL execution coverage. Existing `compile_filter()`
  and adapter implementations remain compatible.

### Changed

- The runtime crate README now contains the complete guide and SQL reference,
  and is included directly as the docs.rs landing page. Guide examples run as
  feature-aware doctests; the workspace README is a navigation/development page.
- PostgreSQL matching and ranking share one validated search specification;
  deduplication uses typed identities instead of formatted strings. Search
  policy and SQL live in `search::postgres`, separate from generic ranking.
- Ranking signals use Diesel's standard nonaggregate expression bounds, including
  bound constants. `sql_type` is now an immutable `sql_type()` accessor. Common
  backend, predicate, adapter, and ranking types are re-exported at the crate root.
- Finalized text composites no longer carry a single leaf's deduplication identity.
  Reusing a compiled score through an adapter retains it as an independent signal;
  applications must disambiguate repeated names as with other adapter metrics.

## 0.1.0-beta.6 — 2026-10-18

### Added

- Default `field_search` and `global_search` compiler hooks receive the original
  parser value and preserve existing substring defaults. Global search expands
  through field hooks unless explicitly overridden.
- Independent `SearchAdapter<Table>` extension point for typed, parameterized
  field and global predicates on all backends.
- Explicit derive options `search(postgres, configuration = "english")`,
  optional `vector = table::document`, field `search_adapter`, and model
  `global_search_adapter`, with conflict/type/backend diagnostics.
- PostgreSQL full-text helpers for text expressions and stored `TsVector`
  targets. Quoted literals use phrase queries; unquoted literals use plain
  queries. NULL propagates under NOT. Other backend built-ins are explicitly
  unsupported; comparison, presence, and nontext containment behavior remains.
- Compiler, derive, SQL-binding, adapter, and database execution tests, plus a
  complete PostgreSQL search example and database setup documentation.

### Migration from beta.5

- Update both crates together to `0.1.0-beta.6` before using the new search
  attributes; generated search code requires the matching runtime hooks.
- Existing models and custom compilers retain substring search by default.
  `field_search` and `global_search` have default implementations, so existing
  compiler implementations do not need new methods.
- PostgreSQL full-text search is opt-in. Create the matching search expression
  index or stored vector in your application, and use the same explicit text
  search configuration for indexing and queries. Quoted search literals use
  phrase semantics instead of substring semantics on configured fields.
- MySQL and SQLite applications can use custom search adapters; built-in
  full-text search attributes support PostgreSQL only.

## 0.1.0-beta.5 — 2026-10-18

### Added

- Automatic PostgreSQL enum filtering for wrappers deriving `Aip160PgEnum`,
  including nullable fields, aliases, and qualified wrapper paths. Labels use
  the wrapper's fallible decoder and are bound as the enum SQL type.
- Enum comparison and database execution coverage, including PostgreSQL
  declaration ordering, nullable values, invalid labels, and adapter overrides.

### Fixed

- Unrelated Diesel attribute options no longer hide `table_name`, and unrelated
  Serde options no longer hide JSONB field renames or `rename_all` validation.
- Duplicate `table_name` options now produce a derive error.

### Migration from beta.4

- Update both crates together to `0.1.0-beta.5`; generated enum filtering code
  requires the matching runtime helpers.
- Remove enum field adapters when the wrapper's label conversion and PostgreSQL
  declaration ordering match the intended filter behavior. Both quoted and
  unquoted labels are accepted; nullable fields support `=null`, `!=null`, and
  `:*`. Enum fields are excluded from global text search.
- Keep explicit adapters for alternate literal mappings or numeric ordering.
  Numeric enum labels still sort in PostgreSQL declaration order by default.

## 0.1.0-beta.4 — 2026-10-17

### Added

- Schema SQL type dispatch for Diesel's built-in column families, aliases, and
  nullable variants across PostgreSQL, MySQL, and SQLite. This includes booleans,
  signed/unsigned integers, floats, decimals, binary, temporal values, JSON, and
  PostgreSQL UUID, network, array, range, multirange, and record types.
- Optional `chrono`, `time`, `uuid`, `numeric`, `network-address`, and
  `ipnet-address` integrations, plus the `native-types` feature bundle.
- `Aip160PgEnum` derive for PostgreSQL enum `FromSql`/`ToSql` implementations on
  local wrappers, using application-provided fallible label conversions.
- Native database execution tests, Docker Compose test infrastructure, and
  `sqlite-integration` (bundled SQLite) and `mysql-integration` features.
- Documentation of native literal formats, SQL equivalents, precision limits,
  containment, and backend-specific behavior.

### Changed

- `Aip160Filter` selects built-in handlers from the schema column's SQL type
  instead of classifying the model's Rust field type.
- **Breaking:** `Aip160FilterCompiler` now uses
  `native_compare(field, parser::Comparator, &parser::Value)` for built-in
  comparisons. `FieldType::Native` replaces the former scalar and JSONB variants;
  `FilterLiteral` and the separate comparison, LIKE, and JSONB trait methods
  have been removed. `custom_compare` and `ilike` remain available.
- Native handlers validate literal spelling, quoting, numeric bounds, NULL,
  presence, and supported operators. SQLite temporal values use Diesel's storage
  format and precision when the matching `chrono` or `time` feature is enabled.

### Migration from beta.3

- Update both crates together to `0.1.0-beta.4`; the runtime requires the matching
  derive version or a compatible newer prerelease.
- Custom compiler implementations must return `FieldType::Native` for built-in
  fields and move literal validation and comparison behavior into
  `native_compare`. It receives the original literal text and quoting, and the
  parser comparator preserves `:`. Implement `ilike` for global search.
- Models using `Aip160Filter` gain SQL type dispatch automatically. Keep
  `#[aip160(adapter = ...)]` for application-defined SQL types and enums, or use
  `#[aip160(skip)]` to exclude fields. `Aip160PgEnum` generates storage codecs;
  filter literal mappings and ordering still require an adapter.
- Enable the matching representation feature for SQLite temporal filtering.
  Review the README's literal-format table before adopting new native types.
- Select one backend at a time; backend features remain mutually exclusive.

## 0.1.0-beta.3 — 2026-10-17

- Added explicit `#[aip160(adapter = ...)]` custom field adapters for
  application-defined SQL types, including PostgreSQL enums.
- Added signed 64-bit and nullable integer comparisons, with checked literal
  conversion and NULL/presence handling.
- Added PostgreSQL enum examples with application-owned numeric label ordering
  and protobuf mappings, plus opt-in database execution coverage.
- Added `postgres-integration` to enable native PostgreSQL connections while
  keeping `postgres` usable for SQL generation without libpq.

## 0.1.0-beta.2 — 2026-10-15

- Changed text `:` and global substring search to case-insensitive matching via
  `LOWER(column) LIKE LOWER(pattern)`, preserving SQL NULL semantics and escaping
  LIKE metacharacters.
- **Breaking:** custom `Aip160FilterCompiler` implementations must supply
  `ilike`. Wildcard equality continues to use case-sensitive LIKE behavior
  subject to database collation.

## 0.1.0-beta.1 — 2026-10-06

- Initial beta parser and generic compiler for AIP-160 filters, typed and
  parameterized Diesel predicates, and SQLite, MySQL, and PostgreSQL examples.
- Added `Aip160Filter` and `Aip160Jsonb` derives, PostgreSQL JSONB paths, and
  Serde field rename mappings.
- Declared Rust 1.99 and the MIT license.
