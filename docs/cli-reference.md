# CLI Reference

Binary: `karapace`. Defined in `crates/karapace-cli/src/main.rs`.

## Global flags

| Flag | Default | Description |
|------|---------|-------------|
| `--store <path>` | `~/.local/share/karapace` | Store directory path |
| `--json` | `false` | JSON output |
| `--verbose` / `-v` | `false` | Debug-level logging |
| `--trace` | `false` | Trace-level logging (implies debug) |

## Environment variables

| Variable | Used by | Description |
|----------|---------|-------------|
| `KARAPACE_LOG` | cli, dbus | Log level filter: `error`, `warn`, `info`, `debug`, `trace`. Overrides `--verbose`/`--trace`. |
| `KARAPACE_STORE` | dbus | Override default store path. |
| `KARAPACE_SKIP_PREREQS` | cli | Set to `1` to skip runtime prerequisite checks. |

## Exit codes

| Code | Constant | Condition |
|------|----------|-----------|
| 0 | `EXIT_SUCCESS` | Success |
| 1 | `EXIT_FAILURE` | General error |
| 2 | `EXIT_MANIFEST_ERROR` | Manifest parse or validation error |
| 3 | `EXIT_STORE_ERROR` | Store integrity or lock error |

Defined in `crates/karapace-cli/src/commands/mod.rs`.

---

## Commands

### `build`

Build an environment from a manifest.

```
karapace build [manifest] [--name <name>]
```

| Argument | Default | Description |
|----------|---------|-------------|
| `manifest` | `karapace.toml` | Path to manifest file |
| `--name` | — | Assign a human-readable name |

Executes: parse → normalize → resolve → lock → build. Writes `karapace.lock` next to the manifest. Requires runtime prerequisites (user namespaces, fuse-overlayfs).

### `rebuild`

Destroy the existing environment and build a new one from the manifest.

```
karapace rebuild [manifest] [--name <name>]
```

Same arguments as `build`. The old environment is destroyed only after the new one builds successfully.

### `enter`

Enter an environment interactively, or run a command.

```
karapace enter <env_id> [-- cmd...]
```

| Argument | Description |
|----------|-------------|
| `env_id` | Full env_id, short_id, or name |
| `-- cmd...` | Optional command to run instead of interactive shell |

Sets state to `Running` on entry, back to `Built` on exit.

### `exec`

Run a command inside an environment (non-interactive).

```
karapace exec <env_id> -- <cmd...>
```

| Argument | Description |
|----------|-------------|
| `env_id` | Full env_id, short_id, or name |
| `cmd...` | Required. Command and arguments. |

### `destroy`

Destroy an environment and its overlay.

```
karapace destroy <env_id>
```

Cannot destroy a `Running` environment. Stop it first.

### `stop`

Stop a running environment.

```
karapace stop <env_id>
```

Sends `SIGTERM`. If the process does not exit within the timeout, sends `SIGKILL`.

### `freeze`

Freeze an environment. Prevents further writes.

```
karapace freeze <env_id>
```

### `archive`

Archive an environment. Prevents entry but preserves store data.

```
karapace archive <env_id>
```

Archived environments are protected from garbage collection.

### `list`

List all environments.

```
karapace list
```

Output columns: `SHORT_ID`, `NAME`, `STATE`, `ENV_ID`.

### `inspect`

Show environment metadata.

```
karapace inspect <env_id>
```

### `diff`

Show changes in the writable overlay.

```
karapace diff <env_id>
```

Lists added, modified, and removed files relative to the base layer.

### `snapshots`

List snapshots for an environment.

```
karapace snapshots <env_id>
```

### `commit`

Save overlay changes as a snapshot layer.

```
karapace commit <env_id>
```

Only valid for `Built` or `Frozen` environments.

### `restore`

Restore an environment's overlay from a snapshot.

```
karapace restore <env_id> <snapshot_hash>
```

| Argument | Description |
|----------|-------------|
| `env_id` | Environment to restore |
| `snapshot_hash` | Layer hash from `snapshots` output |

### `gc`

Garbage collect orphaned store data.

```
karapace gc [--dry-run]
```

| Flag | Description |
|------|-------------|
| `--dry-run` | Report what would be removed without deleting |

### `verify-store`

Verify integrity of all objects in the store.

```
karapace verify-store
```

Re-hashes every object, layer, and metadata entry against its stored key or checksum.

### `push`

Push an environment to a remote store.

```
karapace push <env_id> [--tag <name@tag>] [--remote <url>]
```

| Flag | Description |
|------|-------------|
| `--tag` | Registry key, e.g. `my-env@latest` |
| `--remote` | Remote URL. Overrides `~/.config/karapace/remote.json`. |

Skips blobs that already exist on the remote.

### `pull`

Pull an environment from a remote store.

```
karapace pull <reference> [--remote <url>]
```

| Argument | Description |
|----------|-------------|
| `reference` | Registry key (`name@tag`) or raw `env_id` |

Downloaded objects are verified with blake3 before storage.

### `rename`

Rename an environment.

```
karapace rename <env_id> <new_name>
```

Names must match `[a-zA-Z0-9_-]`, 1–64 characters. Validated in `karapace-store/src/metadata.rs::validate_env_name`.

### `completions`

Generate shell completions.

```
karapace completions <shell>
```

Supported shells: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

### `man-pages`

Generate man pages.

```
karapace man-pages [dir]
```

| Argument | Default | Description |
|----------|---------|-------------|
| `dir` | `man` | Output directory |

### `doctor`

Check system prerequisites and store health.

```
karapace doctor
```

Checks: user namespace support, `fuse-overlayfs` availability, `curl` availability. Exits non-zero if any check fails.

### `migrate`

Check store format version and show migration guidance.

```
karapace migrate
```
