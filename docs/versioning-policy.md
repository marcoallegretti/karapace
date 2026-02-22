# Karapace Versioning Policy

## Versioned Artifacts

Karapace versions three independent artifacts:

| Artifact | Current Version | Location |
|---|---|---|
| Manifest format | `1` | `manifest_version` field in manifest |
| Store layout | `2` | `store/version` file |
| DBus API | `1` | `org.karapace.Manager1` interface |
| Remote protocol | `v1-draft` | `X-Karapace-Protocol` header |

## Freeze Policies

### Store Format Freeze

- **Store layout v2 is frozen as of Karapace 1.0.**
- No breaking changes to the store directory layout, object naming scheme, or metadata JSON schema within 1.x releases.
- New optional fields may be added to metadata JSON with `#[serde(default)]`.
- Store v1 is not supported; Karapace 1.0+ rejects v1 stores with a clear error.
- If a future major version changes the store format, `karapace migrate` will be provided.

### Remote Protocol Freeze

- **Remote protocol v1 is currently `v1-draft`** and may change before Karapace 1.1.
- The protocol will be frozen (declared stable) in Karapace 1.1.
- After freeze: blob routes, registry format, and integrity checking are stable.
- New optional endpoints may be added without a version bump.
- Breaking changes require a protocol version bump and `X-Karapace-Protocol` negotiation.

## Compatibility Rules

### Manifest Format

- The `manifest_version` field is required and checked on parse.
- Only version `1` is supported in Karapace 0.1.
- Adding optional fields is a backward-compatible change (no version bump).
- Removing or renaming fields requires a version bump.
- Changing normalization or hashing behavior requires a version bump.

### Store Layout

- The store format version is stored in `store/version`.
- Karapace refuses to operate on a store with a different version.
- Adding new file types to the store is backward-compatible.
- Changing the object naming scheme or layout requires a version bump.

### DBus API

- The API version is returned by `ApiVersion()`.
- Adding new methods is backward-compatible.
- Changing method signatures requires a version bump.
- The interface name includes the major version (`Manager1`).

## Release Policy

- **Patch releases** (0.1.x): bug fixes only, no format changes.
- **Minor releases** (0.x.0): may add backward-compatible features.
- **Major releases** (x.0.0): may break backward compatibility with version bumps.

## Migration

- Karapace 0.1 does not support migration from other tools.
- Future versions may include `karapace migrate` for store upgrades.
- Manifest version migration is the user's responsibility.
