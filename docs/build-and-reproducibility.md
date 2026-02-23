# Build and Reproducibility

## Deterministic identity

The environment identity (`env_id`) is a blake3 hash computed from the fully resolved lock file — not from the unresolved manifest. Two identical lock files on any machine produce the same `env_id`.

The identity is computed in `karapace-schema/src/lock.rs::LockFile::compute_identity()`. See [architecture.md](architecture.md) for the full list of hash inputs.

## Lock file

`karapace build` writes `karapace.lock` next to the manifest. This file pins:

- Base image content digest (blake3 of rootfs)
- Exact package versions (queried from the package manager inside the image)
- All manifest-declared settings (hardware, mounts, backend, resource limits)

The lock file should be committed to version control.

## Reproducibility constraints

Given the same lock file and the same base image content, builds produce the same `env_id`. The overlay filesystem content depends on the package manager's behavior, which Karapace does not control.

Karapace guarantees identity reproducibility (same inputs → same `env_id`). It does not guarantee bit-for-bit filesystem reproducibility across different package repository states.

## CI release builds

Configured in `.github/workflows/ci.yml` and `.github/workflows/release.yml`.

### Environment variables

| Variable | Value | Purpose |
|----------|-------|---------|
| `CARGO_INCREMENTAL` | `0` | Disable incremental compilation |
| `SOURCE_DATE_EPOCH` | `0` | Deterministic timestamps in build artifacts |

### RUSTFLAGS

```
-D warnings
--remap-path-prefix /home/runner/work=src
--remap-path-prefix /home/runner/.cargo/registry/src=crate
--remap-path-prefix /home/runner/.rustup=rustup
```

Path remapping eliminates runner-specific filesystem paths from the binary.

### Cargo profile (release)

Defined in workspace `Cargo.toml`:

```toml
[profile.release]
strip = true
lto = "thin"
```

### Build procedure

1. `cargo clean` — eliminates stale intermediate artifacts
2. `cargo build --release --target <target> -p karapace-cli -p karapace-dbus`

Both glibc (`x86_64-unknown-linux-gnu`) and musl (`x86_64-unknown-linux-musl`) targets are built. Musl binaries are fully statically linked.

### Reproducibility verification in CI

CI runs two types of reproducibility checks:

- **Same-run:** two sequential builds on the same runner, output hashes compared
- **Cross-run:** builds on `ubuntu-latest` and `ubuntu-22.04`, hashes compared (warning-only — different system libraries may cause differences in glibc builds)

Musl builds are expected to be runner-independent.

**Constraint:** build invocations must use identical `-p` flags. Building `-p karapace-cli` alone may produce a different binary than `-p karapace-cli -p karapace-dbus` due to codegen unit ordering.

## Local development builds

`.cargo/config.toml` configures path remapping for the project maintainer. Other developers should update the paths or set `RUSTFLAGS` directly.

Local builds are for development. CI builds are the authoritative release artifacts.

## Pinned toolchain

CI pins Rust toolchain version via `RUST_TOOLCHAIN` environment variable (currently `1.93`). This is set in the workflow `env` block and used with `dtolnay/rust-toolchain@stable`.
