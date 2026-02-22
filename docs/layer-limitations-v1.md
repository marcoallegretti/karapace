# Karapace Layer Limitations — Phase 1 (v1.0)

## Overview

Karapace 1.0 ships with Phase 1 layer support: deterministic tar-based content-addressed layers. This document describes what is supported, what is not, and what is planned for future phases.

## Supported (Phase 1)

- **Regular files** — full content preservation, deterministic packing.
- **Directories** — including empty directories.
- **Symbolic links** — target path preserved exactly.
- **Deterministic packing** — sorted entries, zero timestamps (`mtime = 0`), owner `0:0`, consistent permissions.
- **Content addressing** — blake3 hash of the tar archive.
- **Snapshot layers** — `commit` captures overlay upper directory as a tar, `restore` unpacks it atomically.
- **Composite snapshot identity** — snapshot layer hash is `blake3("snapshot:{env_id}:{base_layer}:{tar_hash}")` to prevent collision with base layers.
- **Atomic restore** — unpack to staging directory, then rename-swap with the upper directory.

## Not Supported (Phase 1)

The following filesystem features are **silently dropped** during `pack_layer`:

| Feature | Status | Planned |
|---------|--------|---------|
| Extended attributes (xattrs) | Dropped | Phase 2 (1.1) |
| Device nodes | Dropped | Phase 2 (1.1) |
| Hardlinks | Stored as regular files (deduplicated content) | Phase 2 (1.1) |
| SELinux labels | Dropped | Phase 2 (1.1) |
| ACLs | Dropped | Phase 2 (1.1) |
| Sparse files | Stored as full files | Phase 2 (1.1) |
| UID/GID remapping | Not supported | Phase 3 (2.0) |
| Per-file dedup | Not supported | Phase 3 (2.0) |
| Whiteout files (overlay deletion markers) | Included as regular files | Phase 2 (1.1) |

## Implications

### Security-Sensitive Workloads

Environments relying on SELinux labels, xattrs for capabilities (`security.capability`), or ACLs will not have those attributes preserved across `commit`/`restore` cycles. This is acceptable for development environments but not for production container images.

### Hardlinks

If two files in the upper directory are hardlinked, they will be stored as separate regular files in the tar. This means:
- Restoring from a snapshot may increase disk usage.
- File identity (inode sharing) is not preserved.

### Device Nodes

Device nodes (`/dev/*`) created inside the environment are dropped during packing. This is intentional — device nodes are host-specific and should not be stored in content-addressed layers.

### Whiteout Handling

Overlay whiteout files (`.wh.*` and `.wh..wh..opq`) are currently stored as-is in the tar. They are only meaningful when applied on top of the correct base layer. Restoring a snapshot to a different base layer may produce incorrect results.

## Determinism Guarantees

| Property | Guaranteed |
|----------|-----------|
| Same directory content → same tar bytes | Yes |
| Same tar bytes → same blake3 hash | Yes |
| Roundtrip fidelity (regular files, dirs, symlinks) | Yes |
| Timestamp preservation | No (zeroed for determinism) |
| Owner/group preservation | No (set to 0:0) |
| Permission preservation | Yes (mode bits preserved) |

## Phase 2 Roadmap (1.1)

- Extended attribute support via `tar` crate's xattr feature.
- Hardlink detection and deduplication within a single layer.
- Overlay whiteout awareness (proper deletion semantics).
- Device node opt-in for privileged builds.
- SELinux label preservation.

## Phase 3 Roadmap (2.0)

- UID/GID remapping for rootless environments.
- Per-file content deduplication across layers.
- Layer diffing and incremental snapshots.
