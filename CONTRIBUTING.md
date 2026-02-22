# Contributing to Karapace

## Development Setup

```bash
# Clone and build
git clone https://github.com/marcoallegretti/karapace.git
cd karapace
cargo build

# Run tests
cargo test --workspace

# Full verification (must pass before submitting)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
```

## Project Structure

```
crates/
  karapace-schema/    # Manifest parsing, normalization, lock file, identity hashing
  karapace-store/     # Content-addressable store, metadata, layers, GC, integrity
  karapace-runtime/   # Container runtime: images, sandbox, host integration, security
  karapace-core/      # Build engine, lifecycle state machine, drift control, concurrency
  karapace-remote/    # Remote store client, push/pull, registry
  karapace-server/    # Reference remote server (tiny_http)
  karapace-tui/       # Terminal UI (ratatui)
  karapace-cli/       # CLI interface (23 commands)
  karapace-dbus/      # D-Bus desktop integration (optional)
docs/                 # Public documentation and specifications
examples/             # Ready-to-use manifest examples
data/                 # systemd and D-Bus service files
```

## Architecture Principles

Before implementing any feature, verify it aligns with these principles:

1. **Determinism first.** Same manifest + lock = identical environment, always.
2. **No hidden mutable state.** All state changes are explicit and tracked.
3. **No silent drift.** Overlay changes are visible via `diff` and must be committed explicitly.
4. **No privilege escalation.** Everything runs as the unprivileged user.
5. **Convenience must not break reproducibility.** If there's a conflict, determinism wins.

See [Architecture Overview](docs/architecture.md) for the full design.

## Code Standards

- **Zero warnings**: `cargo clippy --workspace --all-targets -- -D warnings` must pass.
- **Formatted**: `cargo fmt --all --check` must pass.
- **No `unwrap()` in production code** (test code is fine).
- **No `TODO`/`FIXME`/`HACK`** in committed code.
- **All values interpolated into shell scripts must use `shell_quote()`.**
- **All mutating operations must hold a `StoreLock`.**
- **All file writes must be atomic** (use `NamedTempFile` + `persist()`).

## Testing

- All new features must include tests.
- Run the full test suite: `cargo test --workspace`
- Integration tests go in `crates/karapace-core/tests/`.
- Unit tests go in the relevant module as `#[cfg(test)] mod tests`.

## Submitting Changes

1. Fork the repository.
2. Create a feature branch from `main`.
3. Make your changes, ensuring all verification checks pass.
4. Submit a pull request with a clear description.

## Building the D-Bus Service

The D-Bus service is not compiled by default:

```bash
# Core CLI only (default)
cargo build --release

# Include D-Bus service
cargo build --release --workspace
```

## Generating Shell Completions

```bash
karapace completions bash > /etc/bash_completion.d/karapace
karapace completions zsh > /usr/share/zsh/site-functions/_karapace
karapace completions fish > ~/.config/fish/completions/karapace.fish
```

## License

By contributing, you agree that your contributions will be licensed under the European Union Public Licence v1.2 (EUPL-1.2).
