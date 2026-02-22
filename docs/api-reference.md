# Karapace Public API Reference

## CLI Commands

### Environment Lifecycle

#### `karapace init [manifest]`
Initialize an environment from a manifest without building. Creates metadata and a preliminary lock file.
- **Default manifest**: `karapace.toml`

#### `karapace build [manifest]`
Build an environment. Resolves dependencies, computes canonical identity, creates store objects/layers, and writes the lock file.
- **Default manifest**: `karapace.toml`

#### `karapace enter <env_id> [-- cmd...]`
Enter a built environment interactively, or run a command if `-- cmd` is provided. Transitions state to Running, then back to Built on exit.
- Accepts full env_id or short_id prefix.

#### `karapace exec <env_id> -- <cmd...>`
Execute a command inside a built environment (non-interactive). Prints stdout/stderr.

#### `karapace rebuild [manifest]`
Atomically rebuild an environment. Builds the new environment first; the old one is only destroyed after a successful build.
- Produces the same env_id for the same resolved manifest.
- If the build fails, the existing environment is preserved.

#### `karapace stop <env_id>`
Stop a running environment by sending SIGTERM/SIGKILL to its process.

#### `karapace freeze <env_id>`
Freeze an environment, preventing further entry. Transitions to Frozen state.

#### `karapace archive <env_id>`
Archive an environment. Preserves it in the store but prevents entry. Can be rebuilt later.

#### `karapace destroy <env_id>`
Destroy an environment's overlay and decrement its reference count.
- **Cannot destroy a running environment** — stop it first.

### Drift Control

#### `karapace diff <env_id>`
Show drift in the writable overlay. Lists added, modified, and removed files.

#### `karapace commit <env_id>`
Commit overlay drift into the content store as a snapshot layer.
- Only works on Built or Frozen environments.

#### `karapace export <env_id> <dest>`
Copy the writable overlay contents to a destination directory.

### Store Management

#### `karapace gc [--dry-run]`
Run garbage collection. Removes orphaned environments, layers, and objects.
- `--dry-run`: report only, do not delete.

#### `karapace verify-store`
Verify integrity of all objects in the store (blake3 content hash check).

#### `karapace verify-lock [manifest]`
Verify lock file integrity (recomputed env_id matches) and manifest consistency (no drift between manifest and lock).

### Inspection

#### `karapace inspect <env_id>`
Show environment metadata: state, layers, ref count, timestamps.

#### `karapace list`
List all known environments with short_id, state, and env_id.

#### `karapace validate [manifest]`
Validate a manifest file and print its preliminary env_id.

### Desktop Integration

#### `karapace export-app <env_id> <name> <binary>`
Export a GUI application from an environment as a `.desktop` file on the host.

#### `karapace unexport-app <env_id> <name>`
Remove an exported application's `.desktop` file from the host.

### Image Management

#### `karapace list-images`
List cached container images with status and size.

#### `karapace remove-image <name>`
Remove a cached container image from the store.

### Quick Start

#### `karapace quick [image] [-p packages] [--gpu] [--audio] [--enter]`
One-step environment creation for casual users. Generates a manifest from CLI flags, builds, and optionally enters.
- **Default image**: `rolling` (openSUSE Tumbleweed)
- `-p` / `--packages`: Comma-separated list of packages to install.
- `--gpu`: Enable GPU passthrough.
- `--audio`: Enable audio passthrough.
- `-e` / `--enter`: Enter the environment immediately after building.
- A real manifest and lock file are still generated (determinism is preserved).

Examples:
```bash
karapace quick rolling -p git,curl --enter
karapace quick ubuntu/24.04 -p build-essential,cmake --gpu
karapace quick fedora/41 --enter
```

### Tooling

#### `karapace completions <shell>`
Generate shell completions for the specified shell. Supported: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

```bash
karapace completions bash > /etc/bash_completion.d/karapace
karapace completions zsh > /usr/share/zsh/site-functions/_karapace
karapace completions fish > ~/.config/fish/completions/karapace.fish
```

#### `karapace man-pages [dir]`
Generate man pages for all commands in the specified directory.
- **Default directory**: `man`

#### `karapace push <env_id> [--tag <name@tag>] [--remote <url>]`
Push an environment (metadata + layers + objects) to a remote store. Skips blobs that already exist remotely.
- **`--tag`**: Publish under a registry key (e.g. `my-env@latest`).
- **`--remote`**: Remote store URL (overrides `~/.config/karapace/remote.json`).

#### `karapace pull <reference> [--remote <url>]`
Pull an environment from a remote store. Reference can be a registry key (e.g. `my-env@latest`) or a raw env_id.
- **`--remote`**: Remote store URL (overrides config).

#### `karapace remote-list [--remote <url>]`
List environments in the remote registry.

## Global Flags

| Flag | Description |
|---|---|
| `--store <path>` | Custom store location (default: `~/.local/share/karapace`). |
| `--json` | Structured JSON output for all applicable commands. |
| `--verbose` / `-v` | Enable debug-level logging output. |

### Environment Variables

| Variable | Description |
|---|---|
| `KARAPACE_LOG` | Log level filter: `error`, `warn` (default), `info`, `debug`, `trace`. |
| `KARAPACE_STORE` | Override default store path (used by the D-Bus service). |

## Exit Codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | General error. |
| `2` | Manifest validation error. |
| `3` | Store integrity error. |

## D-Bus API (Optional)

Interface: `org.karapace.Manager1`
Path: `/org/karapace/Manager1`

The D-Bus service exits after 30 seconds of idle (socket activation). Build with `cargo build -p karapace-dbus`.

All methods return proper D-Bus errors (`org.freedesktop.DBus.Error.Failed`) on failure. Mutating methods acquire the store lock automatically. Methods accepting `id_or_name` resolve by env_id, short_id, name, or prefix.

Desktop notifications are sent on build success/failure via `org.freedesktop.Notifications`.

### Properties

| Property | Type | Description |
|---|---|---|
| `ApiVersion` | `u32` | API version (currently `1`). |
| `StoreRoot` | `String` | Path to the store directory. |

### Methods

| Method | Signature | Description |
|---|---|---|
| `ListEnvironments` | `() → String` | JSON array of `{env_id, short_id, name?, state}`. |
| `GetEnvironmentStatus` | `(id_or_name) → String` | JSON status. Resolves by name. |
| `GetEnvironmentHash` | `(id_or_name) → String` | Returns env_id hash. Resolves by name. |
| `BuildEnvironment` | `(manifest_path) → String` | Build from manifest path. Sends notification. |
| `BuildNamedEnvironment` | `(manifest_path, name) → String` | Build and assign a name. Sends notification. |
| `DestroyEnvironment` | `(id_or_name) → String` | Destroy environment. Resolves by name. |
| `RunEnvironment` | `(id_or_name) → String` | Enter environment. Resolves by name. |
| `RenameEnvironment` | `(id_or_name, new_name) → String` | Rename an environment. |
| `ListPresets` | `() → String` | JSON array of `{name, description}` for built-in presets. |
| `GarbageCollect` | `(dry_run) → String` | Run GC. Acquires store lock. |
| `VerifyStore` | `() → String` | Verify store integrity. Returns `{checked, passed, failed}`. |

## Rust Crate API

### `karapace-core::Engine`

- `Engine::new(store_root)` — Create engine instance
- `Engine::init(manifest_path)` — Initialize environment (Defined state)
- `Engine::build(manifest_path)` — Full resolve → lock → build pipeline
- `Engine::enter(env_id)` — Enter environment interactively
- `Engine::exec(env_id, command)` — Execute command in environment
- `Engine::rebuild(manifest_path)` — Destroy + rebuild
- `Engine::stop(env_id)` — Stop running environment
- `Engine::freeze(env_id)` — Freeze environment
- `Engine::archive(env_id)` — Archive environment (preserve, prevent entry)
- `Engine::commit(env_id)` — Commit overlay drift
- `Engine::destroy(env_id)` — Destroy environment
- `Engine::inspect(env_id)` — Get environment metadata
- `Engine::list()` — List all environments
- `Engine::gc(dry_run)` — Garbage collection (caller must hold store lock)
- `Engine::set_name(env_id, name)` — Set or clear environment name
- `Engine::rename(env_id, new_name)` — Rename environment
