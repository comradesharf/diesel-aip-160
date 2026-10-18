# Releasing 0.1.0-beta.8

Release notes and migration guidance are in [CHANGELOG.md](CHANGELOG.md).
Both crates use the workspace version; publish the derive crate first so the
runtime crate's versioned dependency can resolve on crates.io.

## Validate

Use the pinned Rust 1.99.0 toolchain. Run backend configurations separately
because enabling multiple backends is unsupported.

```sh
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets
cargo test --workspace --locked
cargo test --workspace --locked --all-targets --features sqlite-integration,native-types
cargo test --workspace --locked --all-targets --features mysql,native-types
cargo test --workspace --locked --all-targets --features postgres,native-types
cargo test --workspace --locked --doc --features postgres,native-types
cargo test --workspace --locked --doc --features mysql,native-types
cargo test --workspace --locked --doc --features sqlite-integration,native-types
for backend in sqlite-integration mysql postgres; do
  cargo clippy --workspace --locked --all-targets --features "$backend,native-types"
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --no-deps --features "$backend,native-types"
done
./scripts/test-native-types.sh
```

The final command requires Docker and executes the native database tests
using the Rust 1.99.1 container toolchain.
Review any ignored tests separately; ordinary Cargo tests do not execute them.

## Package and publish

Commit the release preparation before running these commands. Cargo verifies
the packaged derive crate during the dry run. The runtime dry run must follow
publication and registry availability of the matching derive version.

```sh
cargo publish -p diesel-aip-160-derive --locked --dry-run
cargo publish -p diesel-aip-160-derive --locked
cargo publish -p diesel-aip-160 --locked --dry-run
cargo publish -p diesel-aip-160 --locked
```

After both versions are published, tag the release commit and push the tag:

```sh
git tag -a v0.1.0-beta.8 -m 'Release 0.1.0-beta.8'
git push origin v0.1.0-beta.8
```

Use the beta.8 changelog section as the release description. Package dry runs,
publication, and tagging are separate steps; preparing these files does not
publish a release.

## Preparation checks (2026-10-18)

- Both workspace crates, the runtime's derive dependency, lockfile, and
  installation examples use `0.1.0-beta.8`. The changelog includes prefix-search
  release notes, breaking API changes, and migration guidance from beta.7.
- Formatting, locked workspace tests, and Clippy passed offline with the pinned
  Rust 1.99.0 toolchain.
- All-target tests, doctests, Clippy, and strict rustdoc (`-D warnings`) passed
  separately for `sqlite-integration`, `mysql`, and `postgres` with `native-types`.
- Docker PostgreSQL, MySQL, and SQLite execution suites passed with Rust 1.99.1,
  including ignored tests and PostgreSQL prefix-search execution, autocomplete,
  ranking, and GIN index eligibility. Compose ran under isolated project
  `aip160-beta8-release` with host database ports disabled; its containers and
  volumes were removed after validation.
- `cargo package -p diesel-aip-160-derive --locked --offline --allow-dirty`
  built and verified the beta.8 archive. Both package file lists include the MIT
  license; the runtime list includes search/ranking source, examples, and the
  prefix-search regression tests.
- Runtime packaging was attempted offline and cannot resolve
  `diesel-aip-160-derive ^0.1.0-beta.8` from the local crates.io index. Repeat
  its dry run after publishing the matching derive crate and waiting for registry
  availability. No publication or release tag has been performed.
