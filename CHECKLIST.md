# Remaining implementation checklist

This checklist records gaps in the current beta implementation. Unchecked items
are pending work, not promises of support. Update the relevant documentation and
add regression coverage when completing an item.

## Derive correctness and diagnostics

- [ ] Handle nullable integers consistently: `Option<i32>` currently generates
  integer comparisons, but `FieldType::Int32` does not carry nullability and the
  derive does not dispatch null comparisons. Add nullable integer metadata,
  null/presence handling, and backend tests, or reject this field type explicitly
  until it is supported.
- [ ] Preserve generic parameters and `where` clauses in both derives. Ensure
  generated compiler types carry any required parameters and bounds; test
  lifetime, type, and const parameters.
- [ ] Require an explicit `#[aip160(jsonb)]` marker or another validated mapping
  for JSONB fields. The filter derive currently treats every non-string,
  non-integer field as JSONB. Produce clear diagnostics for unsupported types
  and JSONB use with a non-PostgreSQL backend.
- [ ] Resolve the runtime crate when its Cargo dependency is renamed. Generated
  code currently hardcodes `::diesel_aip_160`.
- [ ] Remove generated code's reliance on caller imports for Diesel extension
  methods such as `like`, `and`, and `into_sql`. Test derives in a module without
  `use diesel::prelude::*`.
- [ ] Handle raw identifiers when mapping filter field names and JSONB keys;
  verify a field such as `r#type` uses the intended external name.
- [ ] Fix the missing closing backticks in generated `field_type` and `like`
  error messages.

## JSONB key mapping

- [ ] Omit fields marked `#[serde(skip)]` or `#[serde(skip_serializing)]` from
  `Aip160Jsonb` mappings.
- [ ] Read the serialization name from
  `#[serde(rename(serialize = "...", deserialize = "..."))]`.
- [ ] Reject `#[serde(flatten)]` with a clear error until flattened paths can be
  represented correctly.
- [ ] Validate unsupported Serde mapping attributes rather than silently
  accepting attributes that change stored keys. Keep the existing explicit
  rejection of struct-level `rename_all` unless support is implemented.

## Regression and integration coverage

- [ ] Add compile-pass and compile-fail fixtures for derives, covering unsupported
  field types, malformed attributes, tuple structs, enums, generic models,
  dependency aliases, and missing caller imports. Existing macro unit tests
  inspect generated tokens; fixtures should compile the generated code and
  verify useful diagnostics.
- [ ] Add regression cases for nullable integers and each new JSONB key mapping.
- [ ] Add database execution tests for SQLite and opt-in MySQL/PostgreSQL tests.
  Existing backend tests inspect generated SQL; verify actual result rows for
  NULL semantics, wildcard escaping, negation, and PostgreSQL JSONB operations.
- [ ] Add parser fuzz/property tests for malformed input, Unicode, escaping, and
  size/token/nesting limits, checking for panics and bounded resource use.
- [ ] Add CI checks for formatting, Clippy with warnings denied, tests, and
  rustdoc with warnings denied. Run no-backend, SQLite, MySQL, and PostgreSQL
  configurations separately because backend features are mutually exclusive.

## Documentation and release readiness

- [ ] Choose the project license, add its license text, and inherit the license
  declaration in both package manifests. The license choice is still pending.
- [ ] Document remaining public AST types, variants, constants, compiler trait
  methods, and Diesel/JSONB helpers, including errors and runnable examples.
- [ ] Document the supported field and attribute matrix, including the nullable
  integer limitation until resolved.
- [ ] Verify whether older Rust versions can be supported. The manifests currently
  declare Rust 1.99, matching the tested toolchain; lowering that requirement
  needs dependency and backend checks on the proposed minimum version.
- [ ] Verify actual package archives before release, publishing the derive crate
  before the runtime crate so its versioned dependency can resolve. Package
  listing has been checked; archive compilation has not.
- [ ] Add a changelog covering beta API changes and migration guidance.

## Optional language extensions

These extend the currently documented scope and require an API/behavior decision
before implementation.

- [ ] Decide whether to implement AIP-160 function calls. The parser represents
  functions, while the compiler currently rejects them.
- [ ] Decide whether to support additional scalar types such as booleans,
  64-bit integers, floating-point values, timestamps, UUIDs, and enums; define
  literal conversion, nullability, and backend behavior for each chosen type.
