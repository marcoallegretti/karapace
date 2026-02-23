# Security Model

Defined in `karapace-runtime/src/security.rs::SecurityPolicy`.

## Privilege model

Karapace runs entirely as an unprivileged user. No SUID binaries. No root daemon. Isolation is provided by Linux user namespaces (`unshare`).

The OCI backend delegates to rootless runtimes (`crun`, `runc`, `youki`).

## Mount policy

Absolute host paths in manifest `[mounts]` are validated against an allowlist.

**Default allowed prefixes:** `/home`, `/tmp`.

Relative paths (e.g. `./`) are always permitted. Mounts outside the allowlist are rejected at build time with `RuntimeError::MountDenied`.

Path traversal is prevented by `canonicalize_logical()` in `security.rs`, which resolves `..` components before checking the prefix.

Defined in `SecurityPolicy::validate_mounts`.

## Device policy

Default policy denies all device access.

- `hardware.gpu = true` → allows `/dev/dri`
- `hardware.audio = true` → allows `/dev/snd`

No implicit device passthrough. Defined in `SecurityPolicy::validate_devices`.

## Environment variable control

**Allowed** (propagated into the container):

```
TERM, LANG, HOME, USER, PATH, SHELL, XDG_RUNTIME_DIR
```

**Denied** (never propagated):

```
SSH_AUTH_SOCK, GPG_AGENT_INFO, AWS_SECRET_ACCESS_KEY, DOCKER_HOST
```

Only variables present in the allowed list and absent from the denied list are passed through. Defined in `SecurityPolicy::filter_env_vars`.

## Resource limits

Declared in manifest `[runtime.resource_limits]`:

- `cpu_shares`: CPU shares limit
- `memory_limit_mb`: memory limit in MB

If the policy defines upper bounds, requesting values above them causes a build-time error (`RuntimeError::ResourceLimitExceeded`). Defined in `SecurityPolicy::validate_resource_limits`.

## Store integrity

- **Objects:** blake3 hash verified on every read. Key = hash of content.
- **Metadata:** blake3 checksum embedded in each metadata file, verified on every `get()`.
- **Layers:** file content re-hashed against filename on read.
- **Images:** content digest stored on download, re-verified on cache hits.

## Concurrency

File locking (`flock(2)`) on `store/.lock` for all mutating operations. Both CLI and D-Bus service acquire the lock. Defined in `karapace-core/src/concurrency.rs::StoreLock`.

Running environments cannot be destroyed (must be stopped first).

## Shell injection prevention

Sandbox scripts use POSIX single-quote escaping for all paths and values. Environment variable keys are validated against `[a-zA-Z0-9_]`. Defined in `karapace-runtime/src/sandbox.rs`.

## Manifest parsing

Strict TOML parser with `deny_unknown_fields` on all sections. Unknown keys cause a parse error.

## What is NOT protected

- Karapace does not protect against a compromised host kernel.
- Karapace does not verify the authenticity of upstream base images beyond content hashing.
- The OCI runtime (if used) is trusted.
- Filesystem permissions on the store directory are the user's responsibility.
- Network isolation (`runtime.network_isolation`) depends on the backend implementation.
- No MAC (SELinux/AppArmor) enforcement within the container.

## Trust assumptions

1. The host kernel provides functioning, secure user namespaces.
2. The store directory has correct ownership and permissions.
3. `fuse-overlayfs` is correctly installed and not compromised.
4. The OCI runtime (if used) is a trusted binary.
5. Base images from `images.linuxcontainers.org` are fetched over HTTPS but not GPG-verified.

## Unsafe code

Five `unsafe` blocks, all FFI calls to libc:

| File | Call | Purpose |
|------|------|---------|
| `karapace-core/src/engine.rs` | `libc::kill(SIGTERM)` | Stop running environment |
| `karapace-core/src/engine.rs` | `libc::kill(SIGKILL)` | Force-kill after timeout |
| `karapace-runtime/src/sandbox.rs` | `libc::getuid()` | Get UID for namespace mapping |
| `karapace-runtime/src/sandbox.rs` | `libc::getgid()` | Get GID for namespace mapping |
| `karapace-runtime/src/terminal.rs` | `libc::isatty()` | Detect terminal for interactive mode |
