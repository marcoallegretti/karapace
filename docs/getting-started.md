# Getting Started

This tutorial walks through the first commands you typically use with Karapace.

It is written as a hands-on sequence:

1. Create a `karapace.toml` manifest.
2. Pin the base image reference.
3. Build an environment (produces an `env_id`).
4. Inspect and run commands inside the environment.
5. Save and restore filesystem changes with snapshots.

For full command flags and exit codes, see [cli-reference.md](cli-reference.md).

## Prerequisites

- A Linux host.
- Runtime prerequisites must be available on your machine (user namespaces, overlay tooling, etc.).

You can validate prerequisites and store health with:

```bash
karapace doctor
```

If you are building the CLI from source, the binary is `karapace` (crate `karapace-cli`).

## Choose a store location

Karapace keeps all persistent data in a *store directory*.

- Default store path: `~/.local/share/karapace`
- Override per-command with `--store <path>`

In this tutorial, we use a disposable store directory so you can experiment safely:

```bash
STORE="$(mktemp -d /tmp/karapace-store.XXXXXX)"
```

## 1) Create a manifest (`karapace new`)

Create a new `karapace.toml` in an empty project directory:

```bash
mkdir -p my-project
cd my-project

karapace --store "$STORE" new demo --template minimal
```

What this does:

- Writes `./karapace.toml` in the current directory.
- If your terminal is interactive (TTY), the command may prompt for optional fields:
  - Packages (space-separated)
  - A workspace mount
  - Runtime backend (`namespace`, `oci`, `mock`)
  - Network isolation

What to expect:

- On success, it prints that `karapace.toml` was written.
- If `./karapace.toml` already exists:
  - With `--force`, it overwrites.
  - Without `--force`, it prompts on a TTY; otherwise it fails.

## 2) Pin the base image (`karapace pin`)

Many workflows rely on using a pinned base image reference.

Check whether the manifest is already pinned:

```bash
karapace --store "$STORE" pin --check karapace.toml
```

What to expect:

- On a fresh `minimal` template, `pin --check` typically fails with an error indicating `base.image` is not pinned.

Pin the base image in-place:

```bash
karapace --store "$STORE" pin karapace.toml
```

Then re-check:

```bash
karapace --store "$STORE" pin --check karapace.toml
```

What this does:

- Resolves the `base.image` value to an explicit `http(s)://...` URL.
- Rewrites the manifest file atomically.

## 3) Build an environment (`karapace build`)

Build an environment from the manifest:

```bash
karapace --store "$STORE" build --require-pinned-image karapace.toml
```

What this does:

- Resolves and prepares the base image.
- Builds the environment filesystem.
- Writes `karapace.lock` next to the manifest.
- Produces a deterministic `env_id` (a 64-character hex string). The first 12 characters are the `short_id`.

What to expect:

- The first build for a base image may download and extract a root filesystem.
- On success, output includes the `env_id`.

## 4) Discover and inspect environments (`list`, `inspect`)

List environments in the store:

```bash
karapace --store "$STORE" list
```

Inspect a specific environment:

```bash
karapace --store "$STORE" inspect <env_id>
```

What to expect:

- `list` shows `SHORT_ID`, `NAME`, `STATE`, and `ENV_ID`.
- After a build, the state is typically `built`.

## 5) Run a command inside the environment (`exec`)

Run a non-interactive command inside an environment:

```bash
karapace --store "$STORE" exec <env_id> -- sh -lc "echo hello"
```

What this does:

- Transitions the environment to `Running` for the duration of the command.
- Streams stdout/stderr back to your terminal.
- Returns to `Built` when the command finishes.

## 6) Check filesystem drift (`diff`)

If you write to the environment, those changes live in the writable overlay.

Show changes in the overlay:

```bash
karapace --store "$STORE" diff <env_id>
```

What to expect:

- If you created or modified files via `exec`, `diff` reports added/modified/removed paths.

## 7) Save changes as a snapshot (`commit`) and restore them (`snapshots`, `restore`)

Create a snapshot from the current overlay:

```bash
karapace --store "$STORE" commit <env_id>
```

List snapshots:

```bash
karapace --store "$STORE" snapshots <env_id>
```

Restore from a snapshot:

```bash
karapace --store "$STORE" restore <env_id> <restore_hash>
```

What to expect:

- `commit` returns a snapshot identifier.
- `snapshots` lists snapshots and includes a `restore_hash` value used with `restore`.
- After `restore`, the overlay directory is replaced with the snapshot content.

## Next steps

- Interactive sessions: `karapace enter <env_id>`
- Stop a running session from another terminal: `karapace stop <env_id>`
- State management: `karapace freeze`, `karapace archive`
- Store maintenance: `karapace verify-store`, `karapace gc`, `karapace destroy`

For details and flags, see [cli-reference.md](cli-reference.md).
