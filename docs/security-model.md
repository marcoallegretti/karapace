# Karapace Security Model

## Core Invariants

1. **Host filesystem protected by default.** No writes outside the store and overlay.
2. **No implicit `/dev` access.** Device passthrough is explicit and opt-in.
3. **GPU passthrough requires `hardware.gpu = true`** in the manifest.
4. **Audio passthrough requires `hardware.audio = true`** in the manifest.
5. **Network isolation is configurable** via `runtime.network_isolation`.
6. **No permanent root daemon required.** All operations are rootless.
7. **No SUID binaries required.**

## Mount Policy

- Absolute host paths are checked against a whitelist.
- Default allowed prefixes: `/home`, `/tmp`.
- Relative paths (e.g. `./`) are always allowed.
- Mounts outside the whitelist are rejected at build time.

## Device Policy

- Default policy denies all device access.
- `hardware.gpu = true` adds `/dev/dri` to allowed devices.
- `hardware.audio = true` adds `/dev/snd` to allowed devices.
- No implicit device passthrough.

## Environment Variable Control

- A whitelist of safe environment variables is passed to the environment:
  `TERM`, `LANG`, `HOME`, `USER`, `PATH`, `SHELL`, `XDG_RUNTIME_DIR`.
- A denylist prevents sensitive variables from leaking:
  `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`, `AWS_SECRET_ACCESS_KEY`, `DOCKER_HOST`.
- Only whitelisted, non-denied variables are propagated.

## Resource Limits

- CPU shares and memory limits are declared in the manifest.
- The security policy enforces upper bounds.
- Exceeding policy limits causes a build failure, not a silent cap.

## Privilege Model

- No privileged escalation at any point.
- User namespaces provide isolation without root.
- The OCI backend uses rootless runtimes (crun, runc, youki).

## Threat Model

### Privilege Boundary

- Karapace operates entirely within the user's privilege level.
- The store is owned by the user and protected by filesystem permissions.
- No daemon runs with elevated privileges.

### Attack Surface

- **Manifest parsing**: strict TOML parser with `deny_unknown_fields`.
- **Store integrity**: blake3 verification on every object read.
- **Image integrity**: blake3 digest stored on download, verified on cache hit.
- **Shell injection**: all paths and values in sandbox scripts use POSIX single-quote escaping (`shell_quote`). Environment variable keys are validated against `[a-zA-Z0-9_]`.
- **Mount injection**: prevented by whitelist enforcement.
- **Environment variable leakage**: prevented by deny/allow lists.
- **Concurrent access**: file locking on all mutating CLI and D-Bus operations.
- **Process safety**: cannot destroy a running environment (must stop first).
- **Rebuild atomicity**: new environment is built before the old one is destroyed.

### Isolation Assumptions

- The host kernel provides functioning user namespaces.
- The filesystem permissions on the store directory are correct.
- The OCI runtime (if used) is trusted and correctly installed.

### Security Review Checklist

- [x] No SUID binaries.
- [x] No root daemon.
- [x] Mount whitelist enforced.
- [x] Device passthrough explicit.
- [x] Environment variables controlled.
- [x] Resource limits supported.
- [x] Store integrity verified.
- [x] Concurrent access safe.
- [x] Signal handling (SIGINT/SIGTERM) clean.
