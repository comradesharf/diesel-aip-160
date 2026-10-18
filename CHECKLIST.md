# Remaining implementation checklist

The current supported behavior and SQL reference live in
[diesel-aip-160/README.md](diesel-aip-160/README.md). These are remaining gaps,
not promises of support. Native SQL type dispatch, nullable integers, enum
codecs, database execution tests, and generic ranking are already implemented.

## Derive correctness and diagnostics

- [ ] Preserve generic parameters and `where` clauses in derives; add fixtures
  for lifetime, type, and const parameters.
- [ ] Resolve the runtime crate when its Cargo dependency is renamed. Expansion
  currently refers to `::diesel_aip_160` directly.
- [ ] Handle raw identifiers and explicit column renaming consistently across
  field registration and JSONB key mapping.
- [ ] Add compile-pass/fail fixtures for unsupported types, malformed attributes,
  generic models, dependency aliases, and modules without a Diesel prelude.
  Existing unit tests inspect generated tokens; integration tests and doctests
  compile representative models.

## JSONB key mapping

- [ ] Omit Serde `skip`/`skip_serializing` fields from `Aip160Jsonb` mappings.
- [ ] Support serialization-specific `serde(rename(serialize = "..."))`.
- [ ] Reject unsupported `flatten` and mapping attributes explicitly rather
  than silently accepting a mapping the derive cannot represent.
- [ ] Decide whether to implement `rename_all`; it is currently rejected.

## Validation and release readiness

- [ ] Add parser fuzz/property tests beyond existing malformed-input, Unicode,
  byte/token/nesting-limit, and Boolean-precedence regression tests.
- [ ] Automate the separate no-backend/PostgreSQL/MySQL/SQLite test, Clippy,
  formatting, doctest, and strict-rustdoc checks in CI.
- [ ] Complete item-level documentation for remaining AST variants and native
  helper types. The runtime README is included as the full rustdoc landing page.
- [x] Add the license text corresponding to the workspace's selected MIT license.
- [ ] Evaluate an older minimum compiler only after testing all dependencies
  and backend combinations; the workspace currently declares Rust 1.99.
- [ ] Verify package archives before release, publishing the derive crate first
  so the runtime's versioned derive dependency can resolve. See RELEASE.md.

## Optional extensions

- [ ] Decide whether to implement AIP-160 function calls; compilation currently
  rejects them even though the parser represents them.
- [ ] Consider production fuzzy, spatial, vector, or time-proximity adapters
  using the existing generic ranking hooks, without changing predicate semantics.
- [ ] Consider weighted/normalized ranking only with an explicit policy design;
  unrelated metrics currently remain separate.
