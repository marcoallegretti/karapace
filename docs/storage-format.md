# Storage Format

Store format version: **2**. Defined in `karapace-store/src/layout.rs::STORE_FORMAT_VERSION`.

## Directory layout

Default root: `~/.local/share/karapace`.

```
<root>/
  store/
    version                # { "format_version": 2 }
    .lock                  # flock(2) exclusive lock
    objects/<blake3_hex>   # content-addressable blobs
    layers/<blake3_hex>    # layer manifests (JSON)
    metadata/<env_id>      # environment metadata (JSON)
    staging/               # temp workspace for atomic operations
    wal/<op_id>.json       # write-ahead log entries
  env/
    <env_id>/
      upper/               # overlay writable layer
      overlay/             # overlay mount point
  images/
    <cache_key>/
      rootfs/              # extracted base image filesystem
```

Paths defined in `karapace-store/src/layout.rs::StoreLayout`.

## Version file

```json
{ "format_version": 2 }
```

Checked on every store access. Mismatched versions are rejected with `StoreError::VersionMismatch`.

## Objects

Content-addressable blobs keyed by blake3 hex digest of their content.

- Write: `NamedTempFile` in objects dir → write content → `sync_all()` → `persist()` (atomic rename)
- Read: read file → recompute blake3 → compare to filename → reject on mismatch
- Idempotent: writing identical content is a no-op

Defined in `karapace-store/src/objects.rs::ObjectStore`.

## Layers

JSON files in `store/layers/`. Each describes a tar archive stored in the object store.

```json
{
  "hash": "<layer_hash>",
  "kind": "Base | Dependency | Policy | Snapshot",
  "parent": "<parent_hash> | null",
  "object_refs": ["<hash>", ...],
  "read_only": true,
  "tar_hash": "<blake3_of_tar>"
}
```

Defined in `karapace-store/src/layers.rs::LayerManifest`.

**Layer kinds:**

| Kind | Hash computation | Parent |
|------|-----------------|--------|
| `Base` | `tar_hash` | None |
| `Dependency` | `tar_hash` | Base layer |
| `Policy` | `tar_hash` | — |
| `Snapshot` | `blake3("snapshot:{env_id}:{base_layer}:{tar_hash}")` | Base layer |

Layer integrity is verified on read: the file content is re-hashed and compared to the filename.

### Deterministic tar packing

`karapace-store/src/layers.rs::pack_layer(source_dir)`:

- Entries sorted by path
- Timestamps set to 0
- Owner set to `0:0`
- Permissions preserved
- Symlink targets preserved

**Dropped during packing:** extended attributes, device nodes, hardlinks (stored as regular files), SELinux labels, ACLs, sparse file holes.

`unpack_layer(tar_data, target_dir)` reverses the process.

## Metadata

JSON files in `store/metadata/`, one per environment. Filename is the `env_id`.

```json
{
  "env_id": "...",
  "short_id": "...",
  "name": null,
  "state": "Built",
  "manifest_hash": "<object_hash>",
  "base_layer": "<layer_hash>",
  "dependency_layers": [],
  "policy_layer": null,
  "created_at": "RFC3339",
  "updated_at": "RFC3339",
  "ref_count": 1,
  "checksum": "<blake3_of_json>"
}
```

Defined in `karapace-store/src/metadata.rs::EnvMetadata`.

**States:** `Defined`, `Built`, `Running`, `Frozen`, `Archived`.

**Checksum:** blake3 of the JSON content (excluding the checksum field itself). Computed on every `put()`, verified on every `get()`. Absent in legacy metadata (`#[serde(default)]`).

**Names:** optional, validated by `validate_env_name`: pattern `[a-zA-Z0-9_-]`, 1–64 characters. Unique across all environments.

## Manifest format

File: `karapace.toml`. Parsed by `karapace-schema/src/manifest.rs`.

```toml
manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["git", "curl"]

[gui]
apps = []

[hardware]
gpu = false
audio = false

[mounts]
workspace = "./:/workspace"

[runtime]
backend = "namespace"
network_isolation = false

[runtime.resource_limits]
cpu_shares = 1024
memory_limit_mb = 4096
```

**Required:** `manifest_version` (must be `1`), `base.image` (non-empty).

**Optional:** all other sections. Unknown fields cause a parse error (`deny_unknown_fields`).

**Normalization** (`ManifestV1::normalize`): trim strings, sort and deduplicate packages/apps, sort mounts by label, lowercase backend name. Produces `NormalizedManifest` with a `canonical_json()` method.

## Lock file

File: `karapace.lock`. Written next to the manifest. TOML format.

```toml
lock_version = 2
env_id = "46e1d96f..."
short_id = "46e1d96fdd6f"
base_image = "rolling"
base_image_digest = "a1b2c3d4..."
runtime_backend = "namespace"
hardware_gpu = false
hardware_audio = false
network_isolation = false

[[resolved_packages]]
name = "git"
version = "2.44.0-1"
```

Defined in `karapace-schema/src/lock.rs::LockFile`.

**Verification:**
- `verify_integrity()`: recomputes `env_id` from locked fields, compares to stored value
- `verify_manifest_intent()`: checks manifest hasn't drifted from what was locked

## Hashing

All hashing uses **blake3**, 256-bit output, hex-encoded (64 characters).

Used for: object keys, layer hashes, env_id computation, metadata checksums, image content digests.

## Write-ahead log

`store/wal/<op_id>.json`. Defined in `karapace-store/src/wal.rs`.

```json
{
  "op_id": "20260215120000123-a1b2c3d4",
  "kind": "Build",
  "env_id": "...",
  "timestamp": "RFC3339",
  "rollback_steps": [
    { "RemoveDir": "/path" },
    { "RemoveFile": "/path" }
  ]
}
```

**Operations:** `Build`, `Rebuild`, `Commit`, `Restore`, `Destroy`, `Gc`.

**Recovery:** on `Engine::new()`, all WAL entries are scanned. Each entry's rollback steps execute in reverse order. The entry is then deleted. Corrupt entries are silently removed.

## Atomic write contract

All store writes follow: `NamedTempFile::new_in(dir)` → write → `flush()` → `persist()` (atomic rename). No partial files are visible. Defined throughout `karapace-store`.
