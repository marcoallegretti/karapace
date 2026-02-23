# Changelog

All notable changes to Karapace will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Breaking Changes

- **Store format v2** — `STORE_FORMAT_VERSION` bumped to 2. New `staging/` and `wal/` directories. Version 1 stores require rebuild.
- **CLI pruned to 23 commands** — removed legacy commands: `init`, `preset`, `list-presets`, `export-app`, `unexport-app`, `quick`, `validate`, `verify-lock`, `export`, `list-images`, `remove-image`, `remote-list`, `tui`.
- **Content-addressed layers** — `LayerStore::put()` now returns the blake3 content hash used as filename. Callers must use the returned hash for references.
- **`Engine::gc()` requires `&StoreLock`** — compile-time enforcement that callers hold the store lock before garbage collection.
- **MetadataStore checksum** — `EnvMetadata` now includes an optional `checksum` field (blake3). Written on every `put()`, verified on every `get()`. Backward-compatible via `serde(default)`.

### Added

- **WAL crash safety** — Fixed race windows in `build()` and `restore()` (rollback registered before side-effects). Added WAL protection to `destroy()`, `commit()` (layer manifest rollback), and `gc()` (WAL marker).
- **Integrity hardening** — `LayerStore::get()` verifies blake3 hash on every read. `MetadataStore` embeds and verifies blake3 checksum. `verify_store_integrity()` expanded to check objects, layers, and metadata.
- **GC safety** — `Engine::gc()` now requires `&StoreLock` parameter (type-enforced). Snapshot layers whose parent is a live base layer are preserved during GC.
- **Remote protocol headers** — `X-Karapace-Protocol: 1` header sent on all HTTP backend requests (PUT, GET, HEAD). `PROTOCOL_VERSION` constant exported from `karapace-remote`.
- **unwrap() audit** — 0 `unwrap()` in production code. `Mutex::lock().unwrap()` calls in `MockBackend` replaced with proper `RuntimeError` propagation.
- **Failure mode tests** — WAL write failure, build on read-only WAL dir, stop() with SIGTERM/non-existent PID, permission denied on object read, read-only metadata dir, concurrent GC lock contention, layer/metadata corruption detection.
- **Coverage expansion** — `verify_store_integrity()` now checks objects + layers + metadata. `IntegrityReport` expanded with `layers_checked/passed` and `metadata_checked/passed` fields.
- **Real tar layers** — `pack_layer()`/`unpack_layer()` in karapace-store: deterministic tar creation (sorted entries, zero timestamps, owner 0:0) for regular files, directories, and symlinks. Content-addressed via blake3.
- **Snapshot system** — `Engine::commit()` captures overlay upper as a tar snapshot; `Engine::restore()` atomically unpacks a snapshot via staging directory swap; `Engine::list_snapshots()` lists snapshots for an environment.
- **CLI: `snapshots` and `restore`** — new commands for snapshot management.
- **Write-ahead log (WAL)** — `store/wal/{op_id}.json` tracks in-flight operations with rollback steps. `Engine::new()` auto-recovers on startup. Integrated into `build()`, `commit()`, `restore()`.
- **Newtype wrappers threaded through all structs** — `EnvId`, `ShortId`, `ObjectHash`, `LayerHash` now used in `EnvMetadata` across all 8 crates. Transparent serde for backward compatibility.
- **Engine::push/pull** — transfer logic moved from `karapace-remote` to `Engine` methods. `karapace-remote` is now pure I/O.
- **CoreError::Remote** — new error variant for remote operation failures.
- **CLI stability contract** — `docs/cli-stability.md` defines CLI stability expectations.
- **Remote protocol spec** — `docs/protocol-v1.md` (v1-draft) documents blob store routes, push/pull protocol, registry format.
- **Layer limitations doc** — `docs/layer-limitations.md` documents current limits (no xattrs, device nodes, hardlinks).

### Changed

- **CLI monolith decomposition** — split `main.rs` into ~30 command modules under `commands/`, thin dispatcher in `main.rs`.
- **Error type cleanup** — added `StoreError::InvalidName` and `StoreError::NameConflict` variants; removed `Io(Error::other)` hacks.
- **D-Bus serialization cleanup** — replaced hand-rolled JSON with typed `serde` response structs.
- **Engine store caching** — `MetadataStore`, `ObjectStore`, and `LayerStore` cached as fields on `Engine`.
- **Remote integrity verification** — `pull_env` verifies blake3 hash of each downloaded object and layer.
- **Store spec updated** — `docs/store-spec.md` reflects v2 format with WAL, staging, tar_hash, name field.
- **README updated** — reflects 23 commands, snapshot workflow, remote push/pull examples.

## [0.1.0] — 2026-02-20

### Added

- **Deterministic environment engine** — content-addressed, hash-based environment identity from resolved lock files.
- **Manifest v1** — declarative TOML manifest with strict schema validation, deterministic normalization, and canonical serialization.
- **Lock file v2** — resolved packages with pinned versions, base image content digest (not tag), dual verification (integrity + manifest intent).
- **Content-addressable store** — blake3 hashing, atomic writes (NamedTempFile + persist), integrity verification on read, reference counting, garbage collection with signal cancellation.
- **CLI commands** — `build`, `rebuild`, `enter`, `exec`, `destroy`, `stop`, `freeze`, `archive`, `list`, `inspect`, `diff`, `snapshots`, `commit`, `restore`, `gc`, `verify-store`, `push`, `pull`, `rename`, `completions`, `man-pages`, `doctor`, `migrate`.
- **Example manifests** — `examples/minimal.toml`, `examples/dev.toml`, `examples/gui-dev.toml`, `examples/ubuntu-dev.toml`, `examples/rust-dev.toml` for common use cases.
- **Multi-distro image support** — openSUSE Tumbleweed/Leap, Ubuntu (20.04–24.10), Debian (Bookworm/Trixie/Sid), Fedora (40–42), Arch Linux, custom URLs.
- **Runtime backends** — user namespace (`unshare` + `fuse-overlayfs` + `chroot`), OCI (`crun`/`runc`/`youki`), mock (for testing).
- **Host integration** — Wayland, X11, PipeWire, PulseAudio, D-Bus session bus, GPU (`/dev/dri`), audio (`/dev/snd`), SSH agent, fonts, themes, cursor themes, GTK/icon themes.
- **Desktop app export** — export GUI applications from environments as `.desktop` files on the host.
- **Overlay drift control** — diff, freeze, commit, export writable layer changes.
- **D-Bus desktop integration** — socket-activated `org.karapace.Manager1` service (feature-gated, opt-in).
- **Security model** — mount whitelist, device policy, environment variable allow/deny lists, resource limits, no privilege escalation.
- **Structured logging** — `log` + `env_logger` with `KARAPACE_LOG` env var and `--verbose`/`-v` CLI flag.
- **Concurrency safety** — `StoreLock` file locking on all mutating CLI and D-Bus operations, GC protects active/archived environments.
- **Automated tests** — unit tests, integration tests, crash injection tests, concurrent build safety, GC safety, reproducibility.
- **Shell completions** — `karapace completions bash|zsh|fish|elvish|powershell` for tab completion.
- **Man page generation** — `karapace man-pages <dir>` generates man pages for all commands.
- **Prerequisite detection** — early check for `unshare`, `fuse-overlayfs`, `curl` with distro-aware install instructions.
- **CI pipeline** — GitHub Actions workflow: format, clippy, test, release build with artifact upload.

### Security

- Shell injection prevention via POSIX single-quote escaping (`shell_quote`) on all sandbox script interpolation.
- Environment variable key validation (`[a-zA-Z0-9_]` only).
- Image download integrity — blake3 digest stored on download, `verify_image()` detects corruption.
- Destroy guard — cannot destroy a running environment (must stop first).
- Atomic rebuild — new environment built before old one is destroyed (no data loss on failure).
- PID cast safety — `i32::try_from()` instead of `as i32` for `libc::kill()`.
- Zero `unwrap()` in production code — all error paths handled gracefully.
- Input validation in `quick` command — image and package names validated against TOML injection.
- `Cargo.lock` committed for reproducible builds.

### Documentation

- Manifest v0.1 specification (`docs/manifest-spec.md`)
- Lock file v2 specification (`docs/lock-spec.md`)
- Store format specification (`docs/store-spec.md`)
- Hash contract (`docs/hash-contract.md`)
- Security model with threat model and attack surface (`docs/security-model.md`)
- Public API reference (`docs/api-reference.md`)
- Versioning policy (`docs/versioning-policy.md`)
- `CONTRIBUTING.md` — development workflow, architecture principles, code standards.
- `LICENSE` — European Union Public Licence v1.2 (EUPL-1.2).
