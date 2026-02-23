# Architecture

## Workspace

Karapace is a Cargo workspace of 9 crates.

```
karapace-schema     Manifest parsing, normalization, lock file, identity hashing
karapace-store      Content-addressable object store, layers, metadata, WAL, GC
karapace-runtime    Container backends, image cache, security policy, prerequisites
karapace-core       Engine: orchestrates the full environment lifecycle
karapace-cli        CLI binary (23 commands, clap)
karapace-dbus       D-Bus service (org.karapace.Manager1, zbus)
karapace-tui        Terminal UI (ratatui, crossterm)
karapace-remote     Remote store client: HTTP backend, registry, push/pull
karapace-server     Reference HTTP server for remote store (tiny_http)
```

## Dependency graph

```
karapace-cli ──┬──> karapace-core ──┬──> karapace-schema
               │                    ├──> karapace-store
               │                    ├──> karapace-runtime
               │                    └──> karapace-remote
               ├──> karapace-runtime
               └──> karapace-store

karapace-dbus ────> karapace-core
karapace-tui ─────> karapace-core
karapace-remote ──> karapace-store
karapace-server ──> karapace-remote, karapace-store
```

## Engine lifecycle

`karapace-core::Engine` is the central orchestrator. All state transitions go through it.

```
                  ┌─────────┐
   build()  ───> │ Defined  │
                  └────┬────┘
                       │ resolve → lock → build
                       v
                  ┌─────────┐
                  │  Built   │ <── rebuild()
                  └──┬──┬───┘
          enter() │  │  │ freeze()
                  v  │  v
           ┌─────────┐  ┌─────────┐
           │ Running  │  │ Frozen  │
           └─────────┘  └────┬────┘
                              │ archive()
                              v
                        ┌──────────┐
                        │ Archived │
                        └──────────┘
```

State transitions are validated in `karapace-core/src/lifecycle.rs`. Invalid transitions return `CoreError`.

### Build pipeline

`Engine::build(manifest_path)` executes:

1. Parse manifest (`karapace-schema::parse_manifest_file`)
2. Normalize (`ManifestV1::normalize`) — sort packages, deduplicate, lowercase backend
3. Select runtime backend (`karapace-runtime::select_backend`)
4. Resolve — backend downloads base image, computes content digest, queries package manager for exact versions → `ResolutionResult`
5. Create lock file (`LockFile::from_resolved`) with pinned versions and content digest
6. Compute identity (`LockFile::compute_identity`) → `env_id` (blake3)
7. Store manifest as object, create layers, write metadata
8. Backend builds the environment filesystem
9. Write lock file to disk

### Identity computation

Defined in `karapace-schema/src/lock.rs::LockFile::compute_identity()`.

Input fed to blake3 in order:
- `base_digest:<content_hash>`
- `pkg:<name>@<version>` for each resolved package (sorted)
- `app:<name>` for each app (sorted)
- `hw:gpu` / `hw:audio` if enabled
- `mount:<label>:<host>:<container>` for each mount (sorted)
- `backend:<name>`
- `net:isolated` if enabled
- `cpu:<value>` / `mem:<value>` if set

Output: 64-character hex blake3 digest. First 12 characters = `short_id`.

## Container runtime

`karapace-runtime/src/backend.rs` defines `RuntimeBackend` trait:

```rust
pub trait RuntimeBackend: Send + Sync {
    fn name(&self) -> &str;
    fn available(&self) -> bool;
    fn resolve(&self, spec: &RuntimeSpec) -> Result<ResolutionResult, RuntimeError>;
    fn build(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError>;
    fn enter(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError>;
    fn exec(&self, spec: &RuntimeSpec, command: &[String]) -> Result<Output, RuntimeError>;
    fn destroy(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError>;
    fn status(&self, env_id: &str) -> Result<RuntimeStatus, RuntimeError>;
}
```

Three backends (`karapace-runtime/src/backend.rs::select_backend`):

| Backend | Implementation | Use |
|---------|---------------|-----|
| `namespace` | `unshare` + `fuse-overlayfs` + `chroot` | Default. Unprivileged. |
| `oci` | `crun` / `runc` / `youki` | OCI-compatible runtimes. |
| `mock` | Deterministic stubs | Testing only. |

## Image cache

`karapace-runtime/src/image.rs::ImageCache` stores downloaded base images under `<store_root>/images/<cache_key>/rootfs/`.

Images are fetched from `images.linuxcontainers.org`. The content digest is a blake3 hash of the rootfs directory tree (`compute_image_digest`). Package manager is auto-detected from rootfs contents (`detect_package_manager`).

## Content-addressable store

All persistent data lives under `<store_root>/store/`. See [storage-format.md](storage-format.md) for the full layout.

- **Objects**: keyed by blake3 hash of content. Written atomically (tempfile + rename). Verified on every read.
- **Layers**: JSON manifests describing tar archives. Kinds: `Base`, `Dependency`, `Policy`, `Snapshot`.
- **Metadata**: JSON per environment. Includes state, layers, ref count, checksum.

## Snapshot mechanism

`Engine::commit(env_id)`:
1. Pack the overlay upper directory into a deterministic tar (`pack_layer`)
2. Store tar as object
3. Create a `Snapshot` layer manifest with composite hash: `blake3("snapshot:{env_id}:{base_layer}:{tar_hash}")`

`Engine::restore(env_id, snapshot_hash)`:
1. Retrieve snapshot layer and its tar object
2. Unpack to `store/staging/restore-{env_id}`
3. Atomic rename-swap with the environment's upper directory

Deterministic packing: entries sorted, timestamps zeroed, owner `0:0`, permissions preserved. Symlinks preserved. Extended attributes, device nodes, hardlinks, ACLs, SELinux labels are dropped.

## Garbage collection

`Engine::gc(store_lock, dry_run)` in `karapace-core/src/engine.rs`. Caller must hold `StoreLock`.

Protected from collection:
- Environments with state `Running` or `Archived`
- Layers referenced by any live environment
- Snapshot layers whose parent is a live base layer
- Objects referenced by any live layer or live metadata `manifest_hash`

Everything else is orphaned and removed. GC supports `SIGINT`/`SIGTERM` cancellation.

## Write-ahead log

`karapace-store/src/wal.rs`. JSON entries in `store/wal/`.

Operations tracked: `Build`, `Rebuild`, `Commit`, `Restore`, `Destroy`, `Gc`.

Each entry records rollback steps (`RemoveDir`, `RemoveFile`). On `Engine::new()`, incomplete WAL entries are replayed in reverse order, then deleted. Corrupt entries are silently removed.

## Concurrency

`karapace-core/src/concurrency.rs::StoreLock` uses `flock(2)` on `store/.lock`. All mutating CLI commands and D-Bus methods acquire this lock.

## Signal handling

`karapace-core/src/concurrency.rs::install_signal_handler()` registers `SIGINT`/`SIGTERM` via `ctrlc` crate. Sets an atomic flag checked by GC and long-running operations.

## Unsafe code

Five `unsafe` blocks in the codebase:

| Location | Call | Purpose |
|----------|------|---------|
| `karapace-core/src/engine.rs:455` | `libc::kill(SIGTERM)` | Stop running environment |
| `karapace-core/src/engine.rs:475` | `libc::kill(SIGKILL)` | Force-kill after timeout |
| `karapace-runtime/src/sandbox.rs:46` | `libc::getuid()` | Get current UID for namespace setup |
| `karapace-runtime/src/sandbox.rs:53` | `libc::getgid()` | Get current GID for namespace setup |
| `karapace-runtime/src/terminal.rs:41` | `libc::isatty()` | Detect terminal for interactive mode |
