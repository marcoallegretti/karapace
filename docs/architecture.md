# Karapace Architecture

## Overview

Karapace is a deterministic container environment engine organized as a Cargo workspace of 9 crates. Each crate has a single responsibility and clean dependency boundaries.

## Crate Dependency Graph

```
karapace-cli ─────┬──▶ karapace-core ──┬──▶ karapace-schema
                  │                    ├──▶ karapace-store
                  │                    ├──▶ karapace-runtime
                  │                    └──▶ karapace-remote
                  ├──▶ karapace-remote ──▶ karapace-store
                  └──▶ karapace-tui ────▶ karapace-core

karapace-dbus ────────▶ karapace-core
karapace-server ──────▶ karapace-store (standalone HTTP server)
```

## Crate Responsibilities

### `karapace-schema`
Manifest v1 parsing (TOML), normalization, canonical JSON serialization, environment identity hashing, lock file v2 (resolved packages, integrity/intent verification), and built-in presets.

### `karapace-store`
Content-addressable object store (blake3), layer manifests, environment metadata (with naming, ref-counting, state machine), garbage collection with signal cancellation, and store integrity verification. All writes are atomic via `NamedTempFile` + `persist`.

### `karapace-runtime`
Container runtime abstraction (`RuntimeBackend` trait) with three backends:
- **Namespace** — `unshare` + `fuse-overlayfs` + `chroot` (unprivileged)
- **OCI** — `crun`/`runc`/`youki`
- **Mock** — deterministic test backend

Also handles image downloading, sandbox scripting, host integration (Wayland, GPU, audio, D-Bus), security policy enforcement, and desktop app export.

### `karapace-core`
The `Engine` struct orchestrates the full lifecycle: init → resolve → lock → build → enter/exec → freeze → archive → destroy. Caches `MetadataStore`, `ObjectStore`, and `LayerStore` as fields. Handles drift detection (diff/commit/export via overlay upper_dir scanning) and garbage collection delegation.

### `karapace-cli`
23 CLI commands, each in its own file under `commands/`. Shared helpers in `commands/mod.rs` (spinners, colored output, environment resolution, JSON formatting). `main.rs` is a thin dispatcher. Exit codes: 0 (success), 1 (failure), 2 (manifest error), 3 (store error).

### `karapace-dbus`
Socket-activated D-Bus service (`org.karapace.Manager1`) with 11 methods. Typed serde response structs. Desktop notifications via `notify-rust`. 30-second idle timeout for socket activation. Hardened systemd unit file.

### `karapace-remote`
Remote content-addressable store with `RemoteBackend` trait, HTTP backend (ureq), push/pull transfer with blake3 integrity verification on pull, and a JSON registry for name@tag references.

### `karapace-tui`
Interactive terminal UI (ratatui + crossterm) with list/detail/help views, vim-style keybindings, search/filter, sort cycling, freeze/archive/rename actions, and confirmation dialogs.

## Key Design Decisions

1. **Content-addressed identity** — `env_id` is computed from the *resolved* lock file (pinned versions + base image content digest), not from unresolved manifest data.

2. **Atomic operations** — All store writes use `NamedTempFile` + `persist` for crash safety. Rebuild builds the new environment before destroying the old one.

3. **No `unwrap()` in production** — All error paths are handled with proper error types (`StoreError`, `CoreError`, `RemoteError`, `RuntimeError`).

4. **Store locking** — `StoreLock` file lock on all mutating operations (CLI + D-Bus). GC respects active/archived environments.

5. **Layered security** — Mount whitelist, device policy, env var allow/deny, resource limits. No privilege escalation.

## Data Flow

```
Manifest (TOML)
    │
    ▼
NormalizedManifest (canonical JSON)
    │
    ▼ resolve (RuntimeBackend)
ResolutionResult (base_image_digest + resolved_packages)
    │
    ▼
LockFile v2 (pinned, verifiable)
    │
    ▼ compute_identity()
EnvIdentity (env_id = blake3 of canonical lock)
    │
    ▼ build (store objects + layers + metadata)
Built Environment (overlay filesystem)
```

### `karapace-server`
Reference remote server implementing protocol v1 over HTTP (tiny_http). Provides blob storage, registry, and list endpoints. Used for testing push/pull workflows.

## Test Coverage

417 tests across all crates. 24 ignored tests require privileged operations (real `unshare`, `fuse-overlayfs`, ENOSPC simulation, namespace access).
