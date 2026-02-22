# Karapace Hash Contract

## Overview

The environment identity (`env_id`) is a deterministic blake3 hash that uniquely identifies an environment's fully resolved state. Two identical lock files on any machine must produce the same `env_id`.

## Algorithm

Blake3 (256-bit output, hex-encoded, 64 characters).

## Two-Phase Identity

Karapace computes identity in two phases:

### Preliminary Identity (`compute_env_id`)

Used only during `init` (before resolution) and for internal lookup. Computed from unresolved manifest data. **Not the canonical identity.**

### Canonical Identity (`LockFile::compute_identity`)

The authoritative identity used after `build`. Computed from the fully resolved lock file state. This is what gets stored in metadata and the lock file.

## Canonical Hash Input

The canonical hash includes the following inputs, fed in order:

1. **Base image content digest**: `base_digest:<blake3_of_rootfs>` — real content hash, not a tag name hash.
2. **Resolved packages**: each as `pkg:<name>@<version>` (sorted by name).
3. **Resolved apps**: each as `app:<name>` (sorted).
4. **Hardware policy**: `hw:gpu` if GPU enabled, `hw:audio` if audio enabled.
5. **Mount policy**: each as `mount:<label>:<host_path>:<container_path>` (sorted by label).
6. **Runtime backend**: `backend:<name>` (lowercased).
7. **Network isolation**: `net:isolated` if enabled.
8. **CPU shares**: `cpu:<value>` if set.
9. **Memory limit**: `mem:<value>` if set.

## Hash MUST NOT Include

- Writable overlay state (mutable drift).
- Timestamps (creation, modification).
- Host-specific non-declared paths.
- Machine identifiers (hostname, MAC, etc.).
- Store location.
- Unresolved package names without versions.

## Properties

- **Deterministic**: same resolved inputs → same hash, always.
- **Stable**: consistent across identical systems with same resolved packages.
- **Immutable**: once built, the env_id never changes for that lock state.
- **Version-sensitive**: different package versions produce different identities.

## Short ID

The `short_id` is the first 12 hex characters of `env_id`. Used for display and prefix-matching in the CLI.

## Implementation

- **Canonical**: `karapace-schema/src/lock.rs::LockFile::compute_identity()`
- **Preliminary**: `karapace-schema/src/identity.rs::compute_env_id()`
