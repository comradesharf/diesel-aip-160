# diesel-aip-160-derive

Derive macros for compiling AIP-160 filters against Diesel models. The Cargo package is named `diesel-aip-160-derive`
and is configured for publishing. The generated code uses the `aip160` crate and a Diesel table module supplied by the
consuming crate. This package does not provide the parser or Diesel helpers itself.

## Derives

### `Aip160Filter`

Add the derive to a struct with named fields and specify its Diesel table. The example also derives Diesel's
`Selectable`, which registers the `#[diesel(...)]` helper attribute:

```rust
use diesel_aip_160_derive::Aip160Filter;

#[derive(diesel::Selectable, Aip160Filter)]
#[diesel(table_name = records)]
struct Record {
    name: String,
    count: i32,
    optional_name: Option<String>,
    details: Details,
    #[aip160(skip)]
    internal_note: String,
}

let predicate = Record::compile_filter("name = 'example'")?;
```

The derive creates `RecordFilterCompiler` and `Record::compile_filter(&str)`. The latter returns
`anyhow::Result<Option<Predicate<records::table>>>` through the `aip160` crate. The consuming crate must have the
matching Diesel schema and `aip160` dependency available.

Field handling:

| Rust field type                    | Generated filter support                         |
|------------------------------------|--------------------------------------------------|
| `String`                           | String comparisons and `LIKE`                    |
| `Option<String>`                   | String comparisons, null comparisons, and `LIKE` |
| `i32` or `Option<i32>`             | Signed 32-bit integer comparisons                |
| Other types, including `Option<T>` | JSONB path lookup through `aip160::json_path`    |

`#[aip160(skip)]` excludes a field. The derive requires `#[diesel(table_name = ...)]` and named struct fields. For JSONB
fields, the corresponding type must meet the requirements of `aip160::json_path`.

### `Aip160Jsonb`

Use this derive on a named-field struct stored inside a JSONB column:

```rust
use diesel_aip_160_derive::Aip160Jsonb;

#[derive(Aip160Jsonb)]
struct Details {
    status: String,
    #[serde(rename = "displayName")]
    display_name: String,
}
```

It implements `aip160::JsonbPath`, mapping Rust field names to stored JSON keys. A field without
`#[serde(rename = "...")]` keeps its Rust name; an unknown field returns `None`. Struct-level
`#[serde(rename_all = "...")]` is unsupported and produces a compile error.

## Implementation checklist

- [x] Generate filter comparisons for `String`, `Option<String>`, and `i32` fields.
- [x] Generate string `LIKE` filters and JSONB path operations.
- [x] Exclude fields marked `#[aip160(skip)]`.
- [x] Map JSONB field names, including simple `#[serde(rename = "...")]` values.
- [x] Reject missing Diesel table names and unsupported `serde(rename_all)` attributes.
- [x] Test generated tokens and selected input errors with unit tests.
- [ ] Add null comparison handling for `Option<i32>` fields, or reject that field type at derive time if the runtime
  helper cannot represent it.
- [ ] Preserve generic parameters and `where` clauses in both generated implementations.
- [ ] Omit fields with Serde `skip` or `skip_serializing` from JSONB key mappings.
- [ ] Use the serialization name from `#[serde(rename(serialize = "...", deserialize = "..."))]` for JSONB key mappings.
- [ ] Reject `#[serde(flatten)]` with a clear derive error until flattened paths can be represented.
- [ ] Require an explicit `#[aip160(jsonb)]` marker for JSONB columns and reject other unsupported field types instead
  of assuming they are JSONB.
- [ ] Add macro regression tests for each supported mapping and derive error above.

## Development

From the workspace root:

```sh
cargo test -p diesel-aip-160-derive
```
