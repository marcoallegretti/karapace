# Karapace Remote Protocol v1 (Draft)

> **Status: v1-draft** — This protocol is subject to change before 1.1.

## Overview

The Karapace remote protocol defines how environments are transferred between local stores and a remote HTTP backend. It is a content-addressable blob store with an optional registry index for named references.

## Blob Types

| Kind | Key | Content |
|------|-----|---------|
| `Object` | blake3 hex hash | Raw object data (tar layers, manifests) |
| `Layer` | layer hash | JSON `LayerManifest` |
| `Metadata` | env_id | JSON `EnvMetadata` |

## HTTP Routes

All routes are relative to the remote base URL.

### Blobs

| Method | Route | Description |
|--------|-------|-------------|
| `PUT` | `/blobs/{kind}/{key}` | Upload a blob |
| `GET` | `/blobs/{kind}/{key}` | Download a blob |
| `HEAD` | `/blobs/{kind}/{key}` | Check if blob exists |
| `GET` | `/blobs/{kind}` | List blob keys |

### Registry

| Method | Route | Description |
|--------|-------|-------------|
| `PUT` | `/registry` | Upload registry index |
| `GET` | `/registry` | Download registry index |

## Push Protocol

1. Read local `EnvMetadata` for the target env_id.
2. Collect layer hashes: `base_layer` + `dependency_layers`.
3. For each layer, collect `object_refs`.
4. Upload objects (skip if `HEAD` returns 200).
5. Upload layer manifests (skip if `HEAD` returns 200).
6. Upload metadata blob.
7. If a registry tag is provided, download current registry, merge entry, upload updated registry.

## Pull Protocol

1. Download `Metadata` blob for the target env_id.
2. Collect layer hashes from metadata.
3. Download layer manifests (skip if local store has them).
4. Collect `object_refs` from all layers.
5. Download objects (skip if local store has them).
6. **Verify integrity**: compute blake3 hash of each downloaded object; reject if hash does not match key.
7. Store metadata locally.

## Registry Format

```json
{
  "entries": {
    "my-env@latest": {
      "env_id": "abc123...",
      "short_id": "abc123def456",
      "name": "my-env",
      "pushed_at": "2026-01-15T12:00:00Z"
    }
  }
}
```

### Reference Resolution

References follow the format `name@tag` (default tag: `latest`). Resolution:
1. Parse reference into `(name, tag)`.
2. Look up `{name}@{tag}` in the registry.
3. Return the associated `env_id`.

## Integrity

- All objects are keyed by their blake3 hash.
- On pull, every downloaded object is re-hashed and compared to its key.
- Mismatches produce `RemoteError::IntegrityFailure`.
- Layer manifests and metadata are JSON — no hash verification (they are keyed by logical identifiers, not content hashes).

## Headers

| Header | Value | When |
|--------|-------|------|
| `Content-Type` | `application/octet-stream` | Blob uploads/downloads |
| `Content-Type` | `application/json` | Registry uploads/downloads |

## Version Negotiation

Not yet implemented. The protocol version will be negotiated via a `X-Karapace-Protocol` header in 1.1.

## Error Responses

| Status | Meaning |
|--------|---------|
| 200 | Success |
| 404 | Blob or registry not found |
| 500 | Server error |

## Security Considerations

- No authentication in v1-draft. Authentication headers will be added in 1.1.
- HTTPS is strongly recommended for all remote URLs.
- Object integrity is verified client-side via blake3.
