# Verifying Karapace Downloads

Every tagged release includes signed binaries for both glibc and musl (static) targets,
SHA256 checksums, and in-toto provenance attestations. All artifacts are signed with
cosign using GitHub Actions OIDC — no manual keys involved.

These instructions are mechanically verified in CI: the `verify-docs-executable` job
in `supply-chain-test.yml` executes these exact commands on every PR.

## Choosing a Binary

| Binary | Linking | Use Case |
|--------|---------|----------|
| `karapace-linux-x86_64-gnu` | Dynamic (glibc) | Standard Linux distributions |
| `karapace-linux-x86_64-musl` | Static (musl) | Minimal containers, Alpine, any distro |
| `karapace-dbus-linux-x86_64-gnu` | Dynamic (glibc) | D-Bus service on standard distros |
| `karapace-dbus-linux-x86_64-musl` | Static (musl) | D-Bus service in containers |

The musl binaries are fully statically linked and require no system libraries.

## 1. Verify SHA256 Checksums

```bash
# For glibc binaries:
sha256sum -c SHA256SUMS-gnu

# For musl binaries:
sha256sum -c SHA256SUMS-musl
```

Both `karapace` and `karapace-dbus` must show `OK`.

## 2. Verify Cosign Signatures

Install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/):

```bash
# Verify karapace binary (use -gnu or -musl suffix as appropriate)
cosign verify-blob karapace-linux-x86_64-gnu \
  --signature karapace-gnu.sig \
  --certificate karapace-gnu.crt \
  --certificate-identity-regexp 'https://github.com/marcoallegretti/karapace' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com

# Verify karapace-dbus binary
cosign verify-blob karapace-dbus-linux-x86_64-gnu \
  --signature karapace-dbus-gnu.sig \
  --certificate karapace-dbus-gnu.crt \
  --certificate-identity-regexp 'https://github.com/marcoallegretti/karapace' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Both commands must print `Verified OK`. Replace `-gnu` with `-musl` for static binaries.

## 3. Verify Provenance Attestation

```bash
cosign verify-blob provenance-gnu.json \
  --signature provenance-gnu.json.sig \
  --certificate provenance-gnu.json.crt \
  --certificate-identity-regexp 'https://github.com/marcoallegretti/karapace' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Inspect the provenance to verify the build origin:

```bash
python3 -c "
import json
with open('provenance-gnu.json') as f:
    prov = json.load(f)
src = prov['predicate']['invocation']['configSource']
print(f'Commit: {src[\"digest\"][\"sha1\"]}')
print(f'Repo:   {src[\"uri\"]}')
print(f'Workflow: {src[\"entryPoint\"]}')
print(f'Builder: {prov[\"predicate\"][\"builder\"][\"id\"]}')
"
```

## 4. Inspect SBOM

The CycloneDX SBOM lists all Rust dependencies and their versions:

```bash
python3 -m json.tool karapace_bom.json | head -50
```

## Build Reproducibility

All CI release builds enforce:
- `CARGO_INCREMENTAL=0` — disables incremental compilation
- `cargo clean` before every release build — eliminates stale intermediate artifacts
- `SOURCE_DATE_EPOCH=0` — deterministic timestamps
- `--remap-path-prefix` — eliminates runner-specific filesystem paths
- `strip = true` + `lto = "thin"` — deterministic output

**Reproducibility requirement:** Build invocations must use identical `-p` flags.
Building `-p karapace-cli` alone may produce a different binary than
`-p karapace-cli -p karapace-dbus` due to codegen unit ordering.

Both glibc and musl builds are verified for same-run and cross-run reproducibility
(ubuntu-latest vs ubuntu-22.04). Musl static builds are expected to be fully
runner-independent since they have no dynamic library dependencies.

## Local Development Builds

Local dev builds use `.cargo/config.toml` to remap dependency paths via `--remap-path-prefix`.
This eliminates local filesystem paths from release binaries. The remapping is configured for
the project maintainer's paths; other developers should update the paths in `.cargo/config.toml`
or set `RUSTFLAGS` directly.

**Local builds are for development only. CI builds are the authoritative release artifacts.**

## Release Artifacts

Each GitHub release contains artifacts for both `x86_64-unknown-linux-gnu` (glibc) and
`x86_64-unknown-linux-musl` (static) targets:

| File | Description |
|------|-------------|
| `karapace-linux-x86_64-gnu` | CLI binary (glibc) |
| `karapace-linux-x86_64-musl` | CLI binary (static musl) |
| `karapace-dbus-linux-x86_64-gnu` | D-Bus service binary (glibc) |
| `karapace-dbus-linux-x86_64-musl` | D-Bus service binary (static musl) |
| `SHA256SUMS-gnu` | SHA256 checksums for glibc binaries |
| `SHA256SUMS-musl` | SHA256 checksums for musl binaries |
| `karapace-gnu.sig` / `.crt` | Cosign signature + certificate (glibc CLI) |
| `karapace-musl.sig` / `.crt` | Cosign signature + certificate (musl CLI) |
| `karapace-dbus-gnu.sig` / `.crt` | Cosign signature + certificate (glibc D-Bus) |
| `karapace-dbus-musl.sig` / `.crt` | Cosign signature + certificate (musl D-Bus) |
| `provenance-gnu.json` / `.sig` / `.crt` | Provenance attestation (glibc) |
| `provenance-musl.json` / `.sig` / `.crt` | Provenance attestation (musl) |
