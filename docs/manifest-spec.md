# Karapace Manifest v0.1 Specification

## Overview

The Karapace manifest is a TOML file (typically `karapace.toml`) that declaratively defines an environment. It is the single source of truth for environment identity.

## Schema

### Required Fields

| Field | Type | Description |
|---|---|---|
| `manifest_version` | `u32` | Must be `1`. |
| `base.image` | `string` | Base image identifier. Must not be empty. |

### Optional Sections

#### `[system]`

| Field | Type | Default | Description |
|---|---|---|---|
| `packages` | `string[]` | `[]` | System packages to install. Duplicates are deduplicated during normalization. |

#### `[gui]`

| Field | Type | Default | Description |
|---|---|---|---|
| `apps` | `string[]` | `[]` | GUI applications to install. |

#### `[hardware]`

| Field | Type | Default | Description |
|---|---|---|---|
| `gpu` | `bool` | `false` | Request GPU passthrough (`/dev/dri`). |
| `audio` | `bool` | `false` | Request audio device passthrough (`/dev/snd`). |

#### `[mounts]`

Flat key-value pairs. Each key is a label; each value is `<host_path>:<container_path>`.

- Labels must not be empty.
- The `:` separator is required.
- Absolute host paths are validated against the mount whitelist.
- Relative host paths (e.g. `./`) are always permitted.

#### `[runtime]`

| Field | Type | Default | Description |
|---|---|---|---|
| `backend` | `string` | `"namespace"` | Runtime backend: `namespace`, `oci`, or `mock`. |
| `network_isolation` | `bool` | `false` | Isolate network from host. |

#### `[runtime.resource_limits]`

| Field | Type | Default | Description |
|---|---|---|---|
| `cpu_shares` | `u64?` | `null` | CPU shares limit. |
| `memory_limit_mb` | `u64?` | `null` | Memory limit in MB. |

## Validation Rules

1. `manifest_version` must equal `1`.
2. Unknown fields at any level cause a parse error (`deny_unknown_fields`).
3. `base.image` must not be empty or whitespace-only.
4. Mount specs must contain exactly one `:` with non-empty sides.

## Normalization

During normalization:

- All string values are trimmed.
- `system.packages` and `gui.apps` are sorted, deduplicated.
- Mounts are sorted by label.
- `runtime.backend` is lowercased.

The normalized form is serialized to canonical JSON for hashing.

## Example

```toml
manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["clang", "cmake", "git"]

[gui]
apps = ["ide", "debugger"]

[hardware]
gpu = true
audio = true

[mounts]
workspace = "./:/workspace"

[runtime]
backend = "namespace"
network_isolation = false

[runtime.resource_limits]
cpu_shares = 1024
memory_limit_mb = 4096
```

## Versioning

- The manifest format is versioned via `manifest_version`.
- Only version `1` is supported in Karapace 0.1.
- Future versions will increment this field.
- Backward-incompatible changes require a version bump.
