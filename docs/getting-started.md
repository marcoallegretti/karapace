# Getting Started with Karapace

Karapace creates isolated, reproducible development environments on Linux using
namespaces and overlay filesystems. No root, no daemon, no Docker.

This guide walks you through installation, first use, and common workflows.

## Prerequisites

Karapace requires a Linux system with:

- **User namespace support** (`CONFIG_USER_NS=y` — enabled on all major distros)
- **fuse-overlayfs** (overlay filesystem in userspace)
- **curl** (for downloading base images)

Optional:

- **crun**, **runc**, or **youki** (only if using the OCI backend)

### Install prerequisites by distro

**openSUSE Tumbleweed / Leap:**
```bash
sudo zypper install fuse-overlayfs curl
```

**Ubuntu / Debian:**
```bash
sudo apt install fuse-overlayfs curl
```

**Fedora:**
```bash
sudo dnf install fuse-overlayfs curl
```

**Arch Linux:**
```bash
sudo pacman -S fuse-overlayfs curl
```

Run `karapace doctor` at any time to check that all prerequisites are met.

## Installation

### From source (recommended)

```bash
git clone https://github.com/marcoallegretti/karapace.git
cd karapace
cargo build --release
sudo install -Dm755 target/release/karapace /usr/local/bin/karapace
```

### Via cargo install

```bash
cargo install --git https://github.com/marcoallegretti/karapace.git karapace-cli
```

### Shell completions

```bash
# Bash
karapace completions bash > ~/.local/share/bash-completion/completions/karapace

# Zsh
karapace completions zsh > ~/.local/share/zsh/site-functions/_karapace

# Fish
karapace completions fish > ~/.config/fish/completions/karapace.fish
```

## Your first environment

### 1. Write a manifest

Create a file called `karapace.toml`:

```toml
manifest_version = 1

[base]
image = "rolling"              # openSUSE Tumbleweed

[system]
packages = ["git", "curl"]

[runtime]
backend = "namespace"
```

Available base images: `"rolling"` (openSUSE Tumbleweed), `"ubuntu/24.04"`,
`"debian/12"`, `"fedora/41"`, `"arch"`.

### 2. Build the environment

```bash
karapace build
```

This downloads the base image, installs the requested packages, and produces
a content-addressed environment. The output shows the environment ID (`env_id`)
and a short ID for convenience.

### 3. Enter the environment

```bash
karapace enter <env_id>
```

You can use the short ID (first 8 characters) or a name instead of the full ID.
Inside the environment you have a full Linux userspace with the packages you
requested.

### 4. Run a single command

```bash
karapace exec <env_id> -- git --version
```

## Naming environments

By default, environments are identified by their content hash. You can assign
a human-readable name:

```bash
karapace build --name mydev
karapace enter mydev
```

Or rename an existing environment:

```bash
karapace rename <env_id> mydev
```

## Common workflows

### Snapshot and restore

After making changes inside an environment (installing extra packages, editing
config files), you can snapshot and later restore:

```bash
# See what changed
karapace diff mydev

# Save a snapshot
karapace commit mydev

# List snapshots
karapace snapshots mydev

# Restore a previous snapshot
karapace restore mydev <snapshot_hash>
```

### Freeze and archive

```bash
# Freeze: prevent further changes (still enterable in read-only mode)
karapace freeze mydev

# Archive: preserve metadata but prevent entry
karapace archive mydev
```

### Rebuild from scratch

If you change your manifest, rebuild destroys the old environment and builds
a new one:

```bash
karapace rebuild
```

### Push and pull (remote sharing)

```bash
# Push to a remote store
karapace push mydev --tag my-env@latest

# Pull on another machine
karapace pull my-env@latest --remote https://your-server.example.com
```

### GUI application export

Export a GUI application from inside the container to your host desktop:

```bash
karapace exec mydev -- karapace-export-app firefox
```

This creates a `.desktop` file on the host that launches the app inside the
container with GPU and audio passthrough.

## Hardware passthrough

Enable GPU and audio in your manifest:

```toml
[hardware]
gpu = true       # Passes /dev/dri into the container
audio = true     # Passes PipeWire/PulseAudio socket
```

Karapace also forwards Wayland, X11, D-Bus session bus, SSH agent, fonts,
and GTK/icon themes automatically when available.

## Custom bind mounts

Mount host directories into the container:

```toml
[mounts]
workspace = "~/projects:/workspace"
data = "/data/datasets:/datasets"
```

## Built-in presets

For quick setup without writing a manifest:

```bash
# List available presets
karapace list-presets

# Build from a preset
karapace preset dev-rust
```

Available presets: `dev`, `dev-rust`, `dev-python`, `gui-app`, `gaming`, `minimal`.

## Quick one-liner

The `quick` command combines build + enter in a single step:

```bash
karapace quick
```

This uses the `karapace.toml` in the current directory (or creates a minimal one).

## Diagnostics

```bash
# Check system prerequisites
karapace doctor

# Verify store integrity
karapace verify-store

# List all environments
karapace list

# Inspect an environment
karapace inspect mydev

# Garbage collect unused objects
karapace gc --dry-run
karapace gc
```

## Environment variables

| Variable | Effect |
|----------|--------|
| `KARAPACE_LOG` | Log level: `error`, `warn`, `info`, `debug`, `trace` |
| `KARAPACE_STORE` | Custom store directory (default: `~/.local/share/karapace`) |

Or use CLI flags: `--verbose` / `-v` for debug, `--trace` for trace,
`--store <path>` for a custom store, `--json` for machine-readable output.

## Next steps

- [Manifest v1 Specification](manifest-spec.md) — full manifest reference
- [Architecture Overview](architecture.md) — how Karapace works internally
- [CLI Stability Contract](cli-stability.md) — which commands are stable
- [Security Model](security-model.md) — isolation guarantees and threat model
- [Verification](verification.md) — verifying release artifact integrity
