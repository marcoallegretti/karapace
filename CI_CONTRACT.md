# CI Contract

This document defines the required CI jobs that must exist in `.github/workflows/ci.yml`.
The `ci-contract` job enforces this list automatically — if any job is missing, CI fails.

## Required Jobs (ci.yml)

| Job Name | Purpose | Enforced |
|----------|---------|----------|
| `fmt` | Rustfmt check | Yes |
| `clippy` | Clippy lint | Yes |
| `test` | `cargo test --workspace` on 3 OS matrix | Yes |
| `e2e` | E2E namespace backend tests (`--ignored`) | Yes |
| `enospc` | ENOSPC simulation with tmpfs (requires `sudo`) | Yes |
| `e2e-resolve` | Package resolver E2E on Ubuntu/Fedora/openSUSE | Yes |
| `build-release` | Release build + SBOM + checksums (gnu + musl matrix) | Yes |
| `smoke-test` | Verify release binary runs (gnu + musl matrix) | Yes |
| `reproducibility-check` | Same-run glibc build reproducibility (build A vs B) | Yes |
| `reproducibility-check-musl` | Same-run musl build reproducibility (build A vs B) | Yes |
| `cross-run-reproducibility` | Cross-run glibc reproducibility (ubuntu-latest vs ubuntu-22.04) | Yes |
| `verify-cross-reproducibility` | Compare glibc cross-run build hashes (with diffoscope) | Yes |
| `cross-run-reproducibility-musl` | Cross-run musl reproducibility (ubuntu-latest vs ubuntu-22.04) | Yes |
| `verify-cross-reproducibility-musl` | Compare musl cross-run build hashes | Yes |
| `lockfile-check` | Verify Cargo.lock has not drifted | Yes |
| `cargo-deny` | Dependency policy (licenses, registries, advisories) | Yes |
| `ci-contract` | Self-check: all required jobs exist | Yes |

## Supply Chain Jobs (supply-chain-test.yml)

| Job Name | Purpose |
|----------|---------|
| `build-and-sign` | Build, sign binaries/SBOM, generate provenance attestation |
| `verify-signatures` | Verify cosign signatures + provenance content |
| `tamper-binary` | Modify 1 byte in binary, verify detection |
| `tamper-sbom` | Modify SBOM, verify detection |
| `tamper-signature-removal` | Delete .sig, verify detection |
| `adversarial-env-injection` | Test RUSTFLAGS injection, RUSTC_WRAPPER, proxy leak |
| `adversarial-artifact-tampering` | Tamper multi-rlib, .rmeta, .d, .fingerprint; verify rebuild defense |
| `adversarial-build-script` | Inject rogue build.rs (marker, HOME, hostname); verify hash change |
| `adversarial-credential-injection` | Inject fake API keys, AWS secrets, registry tokens; verify no leak |
| `adversarial-rustflags-bypass` | RUSTFLAGS override + SOURCE_DATE_EPOCH manipulation |
| `verify-docs-executable` | Execute docs/verification.md commands verbatim |

## Release Jobs (release.yml)

| Job Name | Purpose |
|----------|---------|
| `build` | Deterministic release build + SBOM + checksums (gnu + musl matrix) |
| `sign` | Cosign OIDC signing + provenance attestation (per target) |
| `verify` | Verify all signatures + checksums before publishing (per target) |
| `publish` | Create GitHub Release with all artifacts (both targets) |

## Branch Protection

The following jobs MUST be configured as required status checks in GitHub repository settings:

- `Format` (`fmt`)
- `Clippy` (`clippy`)
- `Test (ubuntu)` / `Test (fedora)` / `Test (opensuse)` (`test`)
- `E2E Tests` (`e2e`)
- `ENOSPC Tests` (`enospc`)
- `E2E Resolver (ubuntu)` / `E2E Resolver (fedora)` / `E2E Resolver (opensuse)` (`e2e-resolve`)
- `Release Build (x86_64-unknown-linux-gnu)` / `Release Build (x86_64-unknown-linux-musl)` (`build-release`)
- `Smoke Test Release (x86_64-unknown-linux-gnu)` / `Smoke Test Release (x86_64-unknown-linux-musl)` (`smoke-test`)
- `Reproducibility Check (same-run)` (`reproducibility-check`)
- `Reproducibility Check (musl, same-run)` (`reproducibility-check-musl`)
- `Verify Cross-Run Reproducibility` (`verify-cross-reproducibility`)
- `Cross-Run Reproducibility musl (ubuntu-latest)` / `Cross-Run Reproducibility musl (ubuntu-22.04)` (`cross-run-reproducibility-musl`)
- `Verify Cross-Run Reproducibility (musl)` (`verify-cross-reproducibility-musl`)
- `Lockfile Integrity` (`lockfile-check`)
- `Dependency Policy (cargo-deny)` (`cargo-deny`)
- `CI Contract` (`ci-contract`)
- `Build, Sign & Attest` (`build-and-sign`)
- `Verify Signatures & Provenance` (`verify-signatures`)
- `Tamper Test: Binary` (`tamper-binary`)
- `Tamper Test: SBOM` (`tamper-sbom`)
- `Tamper Test: Signature Removal` (`tamper-signature-removal`)
- `Adversarial: Environment Injection` (`adversarial-env-injection`)
- `Adversarial: Intermediate Artifact Tampering` (`adversarial-artifact-tampering`)
- `Adversarial: Build Script Injection` (`adversarial-build-script`)
- `Adversarial: Credential Injection` (`adversarial-credential-injection`)
- `Adversarial: RUSTFLAGS Bypass` (`adversarial-rustflags-bypass`)
- `Verify docs/verification.md Commands` (`verify-docs-executable`)

## Enforcement

The `ci-contract` job parses `ci.yml` and verifies that every ci.yml job listed above is present.
If a required job is renamed or removed, CI fails immediately.

Supply-chain test jobs are enforced by the `supply-chain-test.yml` workflow running on every PR.
