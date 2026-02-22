use crate::sandbox::BindMount;
use karapace_schema::NormalizedManifest;
use std::path::{Path, PathBuf};

pub struct HostIntegration {
    pub bind_mounts: Vec<BindMount>,
    pub env_vars: Vec<(String, String)>,
}

#[allow(clippy::too_many_lines)]
pub fn compute_host_integration(manifest: &NormalizedManifest) -> HostIntegration {
    let mut bind_mounts = Vec::new();
    let mut env_vars = Vec::new();

    // Wayland display
    if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
        env_vars.push(("WAYLAND_DISPLAY".to_owned(), wayland));
    }

    // X11 display
    if let Ok(display) = std::env::var("DISPLAY") {
        env_vars.push(("DISPLAY".to_owned(), display));
        if Path::new("/tmp/.X11-unix").exists() {
            bind_mounts.push(BindMount {
                source: PathBuf::from("/tmp/.X11-unix"),
                target: PathBuf::from("/tmp/.X11-unix"),
                read_only: true,
            });
        }
        // Xauthority
        if let Ok(xauth) = std::env::var("XAUTHORITY") {
            if Path::new(&xauth).exists() {
                bind_mounts.push(BindMount {
                    source: PathBuf::from(&xauth),
                    target: PathBuf::from(&xauth),
                    read_only: true,
                });
                env_vars.push(("XAUTHORITY".to_owned(), xauth));
            }
        }
    }

    // XDG_RUNTIME_DIR sockets
    if let Ok(xdg_run) = std::env::var("XDG_RUNTIME_DIR") {
        let xdg_path = PathBuf::from(&xdg_run);
        env_vars.push(("XDG_RUNTIME_DIR".to_owned(), xdg_run.clone()));

        // PipeWire socket
        let pipewire = xdg_path.join("pipewire-0");
        if pipewire.exists() {
            bind_mounts.push(BindMount {
                source: pipewire.clone(),
                target: pipewire,
                read_only: false,
            });
        }

        // PulseAudio socket
        let pulse = xdg_path.join("pulse/native");
        if pulse.exists() {
            bind_mounts.push(BindMount {
                source: pulse.clone(),
                target: pulse,
                read_only: false,
            });
        }

        // D-Bus session socket
        let dbus = xdg_path.join("bus");
        if dbus.exists() {
            bind_mounts.push(BindMount {
                source: dbus.clone(),
                target: dbus,
                read_only: false,
            });
            env_vars.push((
                "DBUS_SESSION_BUS_ADDRESS".to_owned(),
                format!("unix:path={xdg_run}/bus"),
            ));
        }

        // Wayland socket
        let wayland_sock = xdg_path.join("wayland-0");
        if wayland_sock.exists() {
            bind_mounts.push(BindMount {
                source: wayland_sock.clone(),
                target: wayland_sock,
                read_only: false,
            });
        }
    }

    // GPU passthrough
    if manifest.hardware_gpu {
        // DRI render nodes
        if Path::new("/dev/dri").exists() {
            bind_mounts.push(BindMount {
                source: PathBuf::from("/dev/dri"),
                target: PathBuf::from("/dev/dri"),
                read_only: false,
            });
        }
        // Nvidia devices
        for dev in &[
            "/dev/nvidia0",
            "/dev/nvidiactl",
            "/dev/nvidia-modeset",
            "/dev/nvidia-uvm",
        ] {
            if Path::new(dev).exists() {
                bind_mounts.push(BindMount {
                    source: PathBuf::from(dev),
                    target: PathBuf::from(dev),
                    read_only: false,
                });
            }
        }
    }

    // Audio passthrough
    if manifest.hardware_audio && Path::new("/dev/snd").exists() {
        bind_mounts.push(BindMount {
            source: PathBuf::from("/dev/snd"),
            target: PathBuf::from("/dev/snd"),
            read_only: false,
        });
    }

    // Manifest-declared mounts
    for mount in &manifest.mounts {
        let host_path = expand_path(&mount.host_path);
        bind_mounts.push(BindMount {
            source: host_path,
            target: PathBuf::from(&mount.container_path),
            read_only: false,
        });
    }

    // Standard env vars to propagate (safe, non-secret variables only).
    // Security-sensitive vars like SSH_AUTH_SOCK and GPG_AGENT_INFO are
    // excluded here — they are in SecurityPolicy.denied_env_vars.
    // Users who need SSH agent forwarding should declare an explicit mount.
    for key in &[
        "TERM", "LANG", "LANGUAGE", "LC_ALL", "SHELL", "EDITOR", "VISUAL",
    ] {
        if let Ok(val) = std::env::var(key) {
            if !env_vars.iter().any(|(k, _)| k == *key) {
                env_vars.push((key.to_string(), val));
            }
        }
    }

    // Font config and themes
    for dir in &["/usr/share/fonts", "/usr/share/icons", "/usr/share/themes"] {
        if Path::new(dir).exists() {
            bind_mounts.push(BindMount {
                source: PathBuf::from(dir),
                target: PathBuf::from(dir),
                read_only: true,
            });
        }
    }

    HostIntegration {
        bind_mounts,
        env_vars,
    }
}

fn expand_path(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    if path.starts_with("./") || path == "." {
        if let Ok(cwd) = std::env::current_dir() {
            return cwd.join(path.strip_prefix("./").unwrap_or(path));
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use karapace_schema::parse_manifest_str;

    #[test]
    fn host_integration_includes_gpu_when_requested() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[hardware]
gpu = true
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let hi = compute_host_integration(&manifest);
        let has_dri = hi
            .bind_mounts
            .iter()
            .any(|m| m.source.as_path() == Path::new("/dev/dri"));
        // Only assert if the device exists on this system
        if Path::new("/dev/dri").exists() {
            assert!(has_dri);
        }
    }

    #[test]
    fn host_integration_excludes_gpu_when_not_requested() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[hardware]
gpu = false
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let hi = compute_host_integration(&manifest);
        let has_dri = hi
            .bind_mounts
            .iter()
            .any(|m| m.source.as_path() == Path::new("/dev/dri"));
        assert!(!has_dri);
    }

    #[test]
    fn manifest_mounts_included() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
workspace = "/tmp/test-src:/workspace"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let hi = compute_host_integration(&manifest);
        assert!(hi
            .bind_mounts
            .iter()
            .any(|m| m.target.as_path() == Path::new("/workspace")));
    }

    #[test]
    fn expand_tilde_path() {
        let expanded = expand_path("~/projects");
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(expanded, PathBuf::from(home).join("projects"));
        }
    }
}
