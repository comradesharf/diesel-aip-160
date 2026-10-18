# diesel-aip-160 workspace

Parse AIP-160 filter strings into typed, parameterized Diesel predicates and
optional ranking signals. Supports PostgreSQL, MySQL, and SQLite.

The project is in beta; breaking changes between beta versions are expected.

- [Complete usage guide and SQL reference](diesel-aip-160/README.md)
- [Online API documentation](https://docs.rs/diesel-aip-160)
- [Derive crate](diesel-aip-160-derive)
- [Changelog](CHANGELOG.md) and [release process](RELEASE.md)

The runtime crate's README is the single source for its crate landing page,
including installation, syntax, native types, JSONB, enum codecs, search,
ranking, custom adapters, and equivalent SQL examples. Rustdoc includes it
directly, and its Rust examples run as doctests for the relevant features.

From this workspace, run backend configurations separately:

```sh
cargo test --workspace
cargo test --workspace --all-targets --features postgres,native-types
cargo test --workspace --all-targets --features mysql,native-types
cargo test --workspace --all-targets --features sqlite-integration,native-types
cargo test --workspace --doc --features postgres,native-types
cargo doc -p diesel-aip-160 --no-deps --features postgres,native-types
```

Use `scripts/test-native-types.sh` for the Docker Compose integration suite.
The repository pins its compiler and formatting tools in `rust-toolchain.toml`.
