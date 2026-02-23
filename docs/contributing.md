# Contributing

## Setup

```bash
git clone https://github.com/marcoallegretti/karapace.git
cd karapace
cargo build
cargo test --workspace
```

## Verification

All of these must pass before submitting changes:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
```

## Project layout

```
crates/
  karapace-schema/    Manifest parsing, normalization, lock file, identity
  karapace-store/     Content-addressable store, metadata, layers, WAL, GC
  karapace-runtime/   Container backends, images, sandbox, security policy
  karapace-core/      Engine: lifecycle orchestration, drift, concurrency
  karapace-cli/       CLI binary (23 commands)
  karapace-dbus/      D-Bus service (optional, not in default-members)
  karapace-tui/       Terminal UI (optional, not in default-members)
  karapace-remote/    Remote store client, push/pull, registry
  karapace-server/    Reference HTTP server for remote store
docs/                 Public documentation
docu_dev/             Internal development notes (not shipped)
data/                 systemd and D-Bus service files
```

`default-members` in `Cargo.toml`: schema, store, runtime, core, cli, server. The D-Bus service and TUI are opt-in.

## Code standards

- `cargo clippy -- -D warnings` — zero warnings.
- `cargo fmt` — enforced.
- No `unwrap()` in production `src/` code. Tests are fine.
- All values interpolated into shell commands must use `shell_quote()`.
- All mutating operations must hold a `StoreLock`.
- All file writes must be atomic (`NamedTempFile` + `persist()`).

## Testing

- Unit tests: `#[cfg(test)] mod tests` in the relevant module.
- Integration tests: `crates/karapace-core/tests/`.
- E2E tests: `crates/karapace-core/tests/e2e.rs` — `#[ignore]`, require user namespaces and `fuse-overlayfs`.

```bash
# Unit + integration tests
cargo test --workspace

# E2E tests (requires Linux with user namespaces)
cargo test --test e2e -- --ignored --test-threads=1
```

## CI

Three workflows in `.github/workflows/`:

| Workflow | File | What it checks |
|----------|------|----------------|
| CI | `ci.yml` | fmt, clippy, tests (Ubuntu + Fedora), E2E, ENOSPC, release builds, reproducibility, smoke tests, lockfile, cargo-deny |
| Release | `release.yml` | Builds release binaries, signs with cosign, generates SBOM and provenance |
| Supply Chain | `supply-chain-test.yml` | Tamper detection, signature verification, adversarial injection tests |

The `ci-contract` job in `ci.yml` verifies that all required jobs are present. See `CI_CONTRACT.md`.

## Building the D-Bus service

```bash
# CLI only (default)
cargo build --release

# CLI + D-Bus service
cargo build --release --workspace
```

## Shell completions

```bash
karapace completions bash > ~/.local/share/bash-completion/completions/karapace
karapace completions zsh > ~/.local/share/zsh/site-functions/_karapace
karapace completions fish > ~/.config/fish/completions/karapace.fish
```

## License

Contributions are licensed under EUPL-1.2.
