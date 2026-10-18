# Releasing 0.1.0-beta.6

Release notes and migration guidance are in [CHANGELOG.md](CHANGELOG.md).
Both crates use the workspace version; publish the derive crate first so the
runtime crate's versioned dependency can resolve on crates.io.

## Validate

Use the pinned Rust 1.99.0 toolchain. Run backend configurations separately
because enabling multiple backends is unsupported.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo test -p diesel-aip-160 --locked --all-targets --features sqlite,native-types
cargo test -p diesel-aip-160 --locked --all-targets --features mysql,native-types
cargo test -p diesel-aip-160 --locked --all-targets --features postgres,native-types
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
git tag -a v0.1.0-beta.6 -m 'Release 0.1.0-beta.6'
git push origin v0.1.0-beta.6
```

Use the beta.6 changelog section as the release description. Package dry runs,
publication, and tagging are separate steps; preparing these files does not
publish a release.

## Preparation checks (2026-10-18)

- Formatting and workspace tests passed with locked dependencies offline using
  the pinned Rust 1.99.0 toolchain.
- All-target tests passed separately for SQLite, MySQL, and PostgreSQL with
  `native-types`, including search adapter and PostgreSQL full-text SQL tests.
- The Docker PostgreSQL and MySQL suites passed with Rust 1.99.1, including
  ignored enum and full-text database execution tests. The normal script hit
  an existing host-port allocation; the same Compose test services were run
  under project `aip160-beta6-release` with host database ports disabled.
- `cargo package -p diesel-aip-160-derive --locked --offline --allow-dirty`
  built and verified the beta.6 derive archive successfully.
- The runtime package file list includes the search module, example, and tests.
  Runtime archive verification was attempted offline but could not resolve
  `diesel-aip-160-derive ^0.1.0-beta.6` from the local crates.io index. Repeat
  its dry run after publishing the derive crate.
- The bundled SQLite Docker suite passed, including native database execution,
  temporal precision, and custom search adapter execution tests.
