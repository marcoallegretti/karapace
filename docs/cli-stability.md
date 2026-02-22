# Karapace CLI Stability Contract

## Scope

This document defines the stability guarantee for the Karapace CLI between 1.x releases.

## Stable Commands (21)

The following commands have **stable signatures** — no breaking changes to arguments, flags, or output format between 1.x releases:

| Command | Description |
|---------|-------------|
| `build` | Build environment from manifest |
| `rebuild` | Destroy + rebuild environment |
| `enter` | Enter environment interactively |
| `exec` | Execute command inside environment |
| `destroy` | Destroy environment |
| `stop` | Stop a running environment |
| `freeze` | Freeze environment (read-only overlay) |
| `archive` | Archive environment (preserve, prevent entry) |
| `list` | List all environments |
| `inspect` | Show environment metadata |
| `diff` | Show overlay drift |
| `snapshots` | List snapshots for an environment |
| `commit` | Commit overlay drift as snapshot |
| `restore` | Restore overlay from snapshot |
| `gc` | Garbage collect orphaned store data |
| `verify-store` | Check store integrity |
| `push` | Push environment to remote store |
| `pull` | Pull environment from remote store |
| `rename` | Rename environment |
| `doctor` | Run diagnostic checks on system and store |
| `migrate` | Check store version and show migration guidance |

## Zero-Maintenance Commands (2)

These commands are auto-generated and have no hand-maintained logic:

| Command | Description |
|---------|-------------|
| `completions` | Generate shell completions (bash/zsh/fish/elvish/powershell) |
| `man-pages` | Generate man pages |

**Total: 23 commands.**

## Global Flags

All commands accept these flags (stable):

- `--store <path>` — custom store location (default: `~/.local/share/karapace`)
- `--json` — structured JSON output on all query and store commands
- `--verbose` / `-v` — enable debug logging
- `--trace` — enable trace-level logging (more detailed than `--verbose`)

## What "Stable" Means

- **No removed flags** — existing flags continue to work.
- **No changed flag semantics** — same flag produces same behavior.
- **No changed exit codes** — exit code meanings are fixed.
- **No changed JSON output keys** — new keys may be added, existing keys are never removed or renamed.
- **New flags may be added** — additive changes are allowed.

## What May Change

- Human-readable (non-JSON) output formatting.
- Spinner and progress indicator appearance.
- Error message wording (not error codes).
- Addition of new commands.
- Addition of new optional flags to existing commands.

## Removed Commands

The following commands were removed before 1.0 and will not return:

`init`, `preset`, `list-presets`, `export-app`, `unexport-app`, `quick`, `validate`, `verify-lock`, `export`, `list-images`, `remove-image`, `remote-list`, `tui`

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General failure |
| 2 | Manifest error (parse, validation) |
| 3 | Store error (integrity, version mismatch) |
