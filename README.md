# Karapace

[![CI](https://github.com/marcoallegretti/karapace/actions/workflows/ci.yml/badge.svg)](https://github.com/marcoallegretti/karapace/actions/workflows/ci.yml)
[![License: EUPL-1.2](https://img.shields.io/badge/License-EUPL_1.2-blue.svg)](LICENSE)

A deterministic container environment engine for immutable Linux systems.

Karapace creates isolated, reproducible development environments using Linux namespaces and overlay filesystems — no root, no daemon, no Docker. Environments are content-addressed artifacts derived from declarative TOML manifests, with full host integration for GPU, audio, Wayland, and desktop applications.

## What Karapace Is (and Isn't)

Karapace is **the identity and certainty layer** for reproducible environments. It is not a general-purpose container runtime.

| Need | Tool |
|---|---|
| Full system container lifecycle, advanced networking, snapshots | Incus, LXD, Podman |
| Deterministic, content-addressed, reproducible environments | **Karapace** |
| Quick disposable containers with zero config | Distrobox |
| Determinism + simplicity for casual use | **Karapace** `quick` command |

Karapace is **complementary** — it can sit on top of or alongside any container runtime. Users needing full container features use a runtime; users needing reproducibility and traceability use Karapace. The `quick` command bridges the gap for users who want both simplicity and determinism.

## Features

- **Real container isolation** — Linux user namespaces (`unshare`), `fuse-overlayfs`, `chroot`
- **No root required** — runs entirely as an unprivileged user
- **No daemon** — direct CLI, no background service needed
- **Multi-distro images** — openSUSE, Ubuntu, Debian, Fedora, Arch from LXC image servers
- **Package installation** — `zypper`, `apt`, `dnf`, `pacman` inside the container
- **Host integration** — home directory, Wayland, PipeWire, D-Bus, GPU (`/dev/dri`), audio (`/dev/snd`), SSH agent, fonts, themes
- **Desktop app export** — export GUI apps as `.desktop` files on the host
- **OCI runtime support** — optional `crun`/`runc`/`youki` backend
- **Content-addressable store** — deterministic hashing, deduplication, integrity verification
- **Overlay drift control** — diff, freeze, commit, export writable layer changes
- **OSC 777 terminal markers** — container-aware terminal integration (Konsole, etc.)

## Crate Layout

| Crate | Purpose |
|---|---|
| `karapace-schema` | Manifest v1 parsing, normalization, identity hashing, lock file |
| `karapace-store` | Content-addressable store, layers, metadata, GC, integrity |
| `karapace-runtime` | Container runtime: image download, sandbox, host integration, app export |
| `karapace-core` | Build engine, lifecycle state machine, drift control, concurrency |
| `karapace-cli` | Full CLI interface (23 commands) |
| `karapace-dbus` | Socket-activated D-Bus desktop integration (**optional**, feature-gated) |
| `karapace-remote` | Remote content-addressable store, push/pull, registry |
| `karapace-server` | Reference remote server implementing protocol v1 (tiny_http) |
| `karapace-tui` | Terminal UI for environment management (ratatui) |

## CLI Commands (23)

```
karapace build [manifest]                   # Build environment from manifest
karapace rebuild [manifest]                 # Destroy + rebuild
karapace enter <env_id> [-- cmd...]         # Enter environment (or run a command)
karapace exec <env_id> -- <cmd...>          # Execute command inside environment
karapace destroy <env_id>                   # Destroy environment
karapace stop <env_id>                      # Stop a running environment
karapace freeze <env_id>                    # Freeze environment
karapace archive <env_id>                   # Archive environment (preserve, prevent entry)
karapace list                               # List all environments
karapace inspect <env_id>                   # Show environment metadata
karapace diff <env_id>                      # Show overlay drift
karapace snapshots <env_id>                 # List snapshots for an environment
karapace commit <env_id>                    # Commit overlay drift as snapshot
karapace restore <env_id> <snapshot>        # Restore overlay from snapshot
karapace gc [--dry-run]                     # Garbage collect store
karapace verify-store                       # Check store integrity
karapace push <env_id> [--tag name@tag]     # Push environment to remote store
karapace pull <reference> [--remote url]    # Pull environment from remote store
karapace rename <env_id> <name>             # Rename environment
karapace doctor                             # Run diagnostic checks on system and store
karapace migrate                            # Check store version and migration guidance
karapace completions <shell>                # Generate shell completions
karapace man-pages [dir]                    # Generate man pages
```

All commands support `--json` for structured output, `--store <path>` for custom store location, `--verbose` / `-v` for debug logging, and `--trace` for trace-level output.

Set `KARAPACE_LOG=debug` (or `info`, `warn`, `error`, `trace`) for fine-grained log control.

## Quick Start

```bash
# Build (core CLI only — D-Bus service is opt-in)
cargo build --release

# Build with D-Bus desktop integration
cargo build --release --workspace
```

```bash
# Write a manifest
cat > karapace.toml << 'EOF'
manifest_version = 1

[base]
image = "rolling"    # openSUSE Tumbleweed (or "ubuntu/24.04", "fedora/41", "arch", etc.)

[system]
packages = ["git", "curl", "vim"]

[hardware]
gpu = true
audio = true

[runtime]
backend = "namespace"
EOF

karapace build
karapace enter <env_id>

# Run a command inside without interactive shell
karapace exec <env_id> -- git --version

# Snapshot and restore
karapace commit <env_id>
karapace snapshots <env_id>
karapace restore <env_id> <snapshot_hash>

# Push/pull to remote
karapace push <env_id> --tag my-env@latest
karapace pull my-env@latest

# List environments
karapace list
```

## Example Manifests

Ready-to-use manifests in `examples/`:

| File | Description |
|---|---|
| `examples/minimal.toml` | Bare openSUSE system, no extras |
| `examples/dev.toml` | Developer tools (git, vim, tmux, gcc, clang) |
| `examples/gui-dev.toml` | GUI development with GPU + audio passthrough |
| `examples/ubuntu-dev.toml` | Ubuntu-based with Node.js, Python, build-essential |
| `examples/rust-dev.toml` | Rust development environment |

## Shell Completions

```bash
# Bash
karapace completions bash > /etc/bash_completion.d/karapace

# Zsh
karapace completions zsh > /usr/share/zsh/site-functions/_karapace

# Fish
karapace completions fish > ~/.config/fish/completions/karapace.fish
```

## Man Pages

```bash
karapace man-pages /usr/share/man/man1
```

## Installation

### From Source (recommended)

```bash
git clone https://github.com/marcoallegretti/karapace.git
cd karapace
cargo build --release
sudo install -Dm755 target/release/karapace /usr/local/bin/karapace
```

### With D-Bus Service

```bash
cargo build --release --workspace
sudo install -Dm755 target/release/karapace /usr/local/bin/karapace
sudo install -Dm755 target/release/karapace-dbus /usr/local/bin/karapace-dbus
sudo install -Dm644 data/dbus/org.karapace.Manager1.service /usr/share/dbus-1/services/
sudo install -Dm644 data/systemd/karapace-dbus.service /usr/lib/systemd/user/
```

### Via Cargo

```bash
cargo install --git https://github.com/marcoallegretti/karapace.git karapace-cli
```

## Prerequisites

- Linux with user namespace support (`CONFIG_USER_NS=y`)
- `fuse-overlayfs` (for overlay filesystem)
- `curl` (for image downloads)
- Optional: `crun`/`runc`/`youki` (for OCI backend)

Karapace checks for missing prerequisites at startup and provides distro-specific install instructions.

## Documentation

- **[Getting Started Guide](docs/getting-started.md)** — installation, first use, common workflows
- [Architecture Overview](docs/architecture.md)
- [Manifest v1 Specification](docs/manifest-spec.md)
- [Lock File v2 Specification](docs/lock-spec.md)
- [Store Format v2 Specification](docs/store-spec.md)
- [Hash Contract](docs/hash-contract.md)
- [Security Model](docs/security-model.md)
- [CLI Stability Contract](docs/cli-stability.md)
- [Remote Protocol v1 (Draft)](docs/protocol-v1.md)
- [Layer Limitations (Phase 1)](docs/layer-limitations-v1.md)
- [Public API Reference](docs/api-reference.md)
- [Versioning Policy](docs/versioning-policy.md)
- [Verification & Supply Chain](docs/verification.md)
- [E2E Testing](docs/e2e-testing.md)

## Verification

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release --workspace
```
