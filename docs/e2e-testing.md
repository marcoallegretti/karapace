# End-to-End Testing

Karapace includes end-to-end tests that exercise the real namespace backend with `unshare`, `fuse-overlayfs`, and actual container images.

## Prerequisites

- Linux with user namespace support (`CONFIG_USER_NS=y`)
- `fuse-overlayfs` installed
- `curl` installed
- Network access (images are downloaded from `images.linuxcontainers.org`)

### Install on openSUSE Tumbleweed

```bash
sudo zypper install fuse-overlayfs curl
```

### Install on Ubuntu/Debian

```bash
sudo apt-get install fuse-overlayfs curl
```

### Install on Fedora

```bash
sudo dnf install fuse-overlayfs curl
```

## Running E2E Tests

E2E tests are `#[ignore]` by default. Run them explicitly:

```bash
cargo test --test e2e -- --ignored --test-threads=1
```

The `--test-threads=1` flag is important: E2E tests mount overlays and download images, so parallel execution can cause resource conflicts.

## Test Descriptions

| Test | What it does |
|---|---|
| `e2e_build_minimal_namespace` | Build a minimal openSUSE Tumbleweed environment with no packages |
| `e2e_exec_in_namespace` | Build + exec `echo hello` inside the container |
| `e2e_destroy_cleans_up` | Build + destroy, verify env_dir is removed |
| `e2e_rebuild_determinism` | Build + rebuild, verify env_id is identical |
| `e2e_build_with_packages` | Build with `which` package, verify resolved versions in lock file |

## CI

The GitHub Actions CI workflow includes an E2E job that runs on `ubuntu-latest`:

```yaml
e2e:
  name: E2E Tests
  runs-on: ubuntu-latest
  needs: [test]
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - name: Install prerequisites
      run: |
        sudo apt-get update -qq
        sudo apt-get install -y -qq fuse-overlayfs curl
        sudo sysctl -w kernel.unprivileged_userns_clone=1 || true
    - name: Run E2E tests
      run: cargo test --test e2e -- --ignored --test-threads=1
```

## Troubleshooting

- **"unshare: user namespaces not available"** — Enable with `sysctl kernel.unprivileged_userns_clone=1`
- **"fuse-overlayfs not found"** — Install the `fuse-overlayfs` package
- **"failed to download image"** — Check network connectivity and DNS
- **Stale mounts after failed test** — Run `fusermount3 -u /path/to/merged` or reboot
