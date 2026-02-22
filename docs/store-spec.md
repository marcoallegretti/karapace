# Karapace Store Format Specification (v2)

## Overview

The Karapace store is a content-addressable filesystem structure that holds all environment data: objects, layers, metadata, environment directories, and crash recovery state.

## Directory Layout

```
<store_root>/
  store/
    version          # JSON: { "format_version": 2 }
    .lock            # flock(2) file for exclusive access
    objects/<hash>   # Content-addressable blobs (blake3)
    layers/<hash>    # Layer manifests (JSON)
    metadata/<env_id> # Environment metadata (JSON)
    staging/         # Temporary workspace for atomic operations
    wal/             # Write-ahead log entries (JSON)
  env/
    <env_id>/
      upper/         # Writable overlay layer (fuse-overlayfs upperdir)
      lower -> ...   # Symlink to base image rootfs
      work/          # Overlay workdir (ephemeral)
      merged/        # Overlay mount point
  images/
    <cache_key>/
      rootfs/        # Extracted base image rootfs
```

## Format Version

- Current version: **2**
- Stored in `store/version` as JSON.
- Checked on every store access; mismatches are rejected.
- Version 1 stores are not auto-migrated; a clean rebuild is required.

## Objects

- Keyed by blake3 hex digest of their content.
- Written atomically: write to tempfile, then rename.
- Integrity verified on every read: content re-hashed and compared to filename.
- Idempotent: writing the same content twice is a no-op.

## Layers

Each layer is a JSON manifest:

```json
{
  "hash": "<layer_hash>",
  "kind": "Base" | "Dependency" | "Policy" | "Snapshot",
  "parent": "<parent_hash>" | null,
  "object_refs": ["<hash>", ...],
  "read_only": true,
  "tar_hash": "<blake3_hash>"
}
```

- `tar_hash` (v2): blake3 hash of the deterministic tar archive stored in the object store.
- Base layers have no parent. Their `hash` equals their `tar_hash`.
- Dependency layers reference a base parent.
- Snapshot layers are created by `commit`. Their `hash` is a composite identity: `blake3("snapshot:{env_id}:{base_layer}:{tar_hash}")` to prevent collision with base layers.

## Metadata

Each environment has a JSON metadata file:

```json
{
  "env_id": "...",
  "short_id": "...",
  "name": "my-env",
  "state": "Defined" | "Built" | "Running" | "Frozen" | "Archived",
  "manifest_hash": "<object_hash>",
  "base_layer": "<layer_hash>",
  "dependency_layers": ["<hash>", ...],
  "policy_layer": null | "<hash>",
  "created_at": "RFC3339",
  "updated_at": "RFC3339",
  "ref_count": 1
}
```

- `name` is optional (`#[serde(default)]`). Old metadata without this field deserializes correctly.

## Atomic Write Contract

All writes follow the pattern:
1. Create `NamedTempFile` in the target directory.
2. Write full content.
3. `flush()`.
4. `persist()` (atomic rename).

This ensures no partial files are visible.

## Garbage Collection

- Environments with `ref_count == 0` and state not in {`Running`, `Archived`} are eligible for collection.
- Layers not referenced by any live environment are orphaned.
- Objects not referenced by any live layer or live metadata (`manifest_hash`) are orphaned.
- GC never deletes running or archived environments.
- GC supports graceful cancellation via signal handler (`SIGINT`/`SIGTERM`).
- `--dry-run` reports what would be removed without acting.
- The caller must hold the store lock before running GC.

## Write-Ahead Log (WAL)

The `store/wal/` directory contains JSON entries for in-flight mutating operations. Each entry tracks:

```json
{
  "op_id": "20260215120000123-a1b2c3d4",
  "kind": "Build" | "Rebuild" | "Commit" | "Restore" | "Destroy",
  "env_id": "...",
  "timestamp": "RFC3339",
  "rollback_steps": [
    { "RemoveDir": "/path/to/orphaned/dir" },
    { "RemoveFile": "/path/to/orphaned/file" }
  ]
}
```

### Recovery Protocol

1. On `Engine::new()`, the WAL directory is scanned for incomplete entries.
2. Each entry's rollback steps are executed in **reverse order**.
3. The WAL entry is then removed.
4. Corrupt or unreadable WAL entries are silently deleted.

### Invariants

- **INV-W1**: Kill during rebuild → next startup rolls back orphaned env_dir.
- **INV-W2**: Kill during build → orphaned env_dir cleaned.
- **INV-W3**: Successful operations leave zero WAL entries.

## Staging Directory

The `store/staging/` directory is a temporary workspace used for atomic operations:

- **Restore**: snapshot tar is unpacked into `staging/restore-{env_id}`, then renamed to replace the overlay upper directory.
- **Layer packing**: temporary files during tar creation.

The staging directory is cleaned up after each operation. Leftover staging data is safe to delete.

## Backward Compatibility

- Layout changes require a format version bump.
- Karapace 1.0 requires format version 2.
- Version 1 stores are not auto-migrated; environments must be rebuilt.
- The `name` and `tar_hash` fields use `#[serde(default)]` for forward-compatible deserialization.
