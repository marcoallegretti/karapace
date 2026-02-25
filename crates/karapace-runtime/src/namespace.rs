use crate::backend::{RuntimeBackend, RuntimeSpec, RuntimeStatus};
use crate::host::compute_host_integration;
use crate::image::{
    compute_image_digest, detect_package_manager, force_remove, install_packages_command,
    parse_version_output, query_versions_command, resolve_image, ImageCache,
};
use crate::sandbox::{
    exec_in_container, install_packages_in_container, mount_overlay, setup_container_rootfs,
    spawn_enter_interactive, unmount_overlay, SandboxConfig,
};
use crate::terminal;
use crate::RuntimeError;
use karapace_schema::{ResolutionResult, ResolvedPackage};
use libc::{SIGKILL, SIGTERM};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};

pub struct NamespaceBackend {
    store_root: PathBuf,
}

impl Default for NamespaceBackend {
    fn default() -> Self {
        Self {
            store_root: default_store_root(),
        }
    }
}

impl NamespaceBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_store_root(store_root: impl Into<PathBuf>) -> Self {
        Self {
            store_root: store_root.into(),
        }
    }

    fn env_dir(&self, env_id: &str) -> PathBuf {
        self.store_root.join("env").join(env_id)
    }
}

impl RuntimeBackend for NamespaceBackend {
    fn name(&self) -> &'static str {
        "namespace"
    }

    fn available(&self) -> bool {
        let output = std::process::Command::new("unshare")
            .args(["--user", "--map-root-user", "--fork", "true"])
            .output();
        matches!(output, Ok(o) if o.status.success())
    }

    fn resolve(&self, spec: &RuntimeSpec) -> Result<ResolutionResult, RuntimeError> {
        let progress = |msg: &str| {
            eprintln!("[karapace] {msg}");
        };

        let resolved = resolve_image(&spec.manifest.base_image)?;
        let image_cache = ImageCache::new(&self.store_root);
        let rootfs = image_cache.ensure_image(&resolved, &progress, spec.offline)?;

        let base_image_digest = compute_image_digest(&rootfs)?;

        if spec.offline && !spec.manifest.system_packages.is_empty() {
            return Err(RuntimeError::ExecFailed(
                "offline mode: cannot resolve system packages".to_owned(),
            ));
        }

        let resolved_packages = if spec.manifest.system_packages.is_empty() {
            Vec::new()
        } else {
            let tmp_dir = tempfile::tempdir()
                .map_err(|e| RuntimeError::ExecFailed(format!("failed to create temp dir: {e}")))?;
            let tmp_env = tmp_dir.path().join("resolve-env");
            std::fs::create_dir_all(&tmp_env)?;

            let mut sandbox = SandboxConfig::new(rootfs.clone(), "resolve-tmp", &tmp_env);
            sandbox.isolate_network = false;

            mount_overlay(&sandbox)?;
            setup_container_rootfs(&sandbox)?;

            let resolve_inner = || -> Result<Vec<(String, String)>, RuntimeError> {
                let pkg_mgr = detect_package_manager(&sandbox.overlay_merged)
                    .or_else(|| detect_package_manager(&rootfs))
                    .ok_or_else(|| {
                        RuntimeError::ExecFailed(
                            "no supported package manager found in the image".to_owned(),
                        )
                    })?;

                let install_cmd = install_packages_command(pkg_mgr, &spec.manifest.system_packages);
                install_packages_in_container(&sandbox, &install_cmd)?;

                let query_cmd = query_versions_command(pkg_mgr, &spec.manifest.system_packages);
                let output = exec_in_container(&sandbox, &query_cmd)?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                Ok(parse_version_output(pkg_mgr, &stdout))
            };

            let result = resolve_inner();

            let _ = unmount_overlay(&sandbox);
            let _ = std::fs::remove_dir_all(&tmp_env);

            let versions = result?;

            spec.manifest
                .system_packages
                .iter()
                .map(|name| {
                    let version = versions
                        .iter()
                        .find(|(n, _)| n == name)
                        .map_or_else(|| "unresolved".to_owned(), |(_, v)| v.clone());
                    ResolvedPackage {
                        name: name.clone(),
                        version,
                    }
                })
                .collect()
        };

        Ok(ResolutionResult {
            base_image_digest,
            resolved_packages,
        })
    }

    fn build(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let env_dir = self.env_dir(&spec.env_id);
        std::fs::create_dir_all(&env_dir)?;

        let progress = |msg: &str| {
            eprintln!("[karapace] {msg}");
        };

        let resolved = resolve_image(&spec.manifest.base_image)?;
        let image_cache = ImageCache::new(&self.store_root);
        let rootfs = image_cache.ensure_image(&resolved, &progress, spec.offline)?;

        let mut sandbox = SandboxConfig::new(rootfs.clone(), &spec.env_id, &env_dir);
        sandbox.isolate_network = spec.offline || spec.manifest.network_isolation;

        mount_overlay(&sandbox)?;

        setup_container_rootfs(&sandbox)?;

        if !spec.manifest.system_packages.is_empty() {
            if spec.offline {
                return Err(RuntimeError::ExecFailed(
                    "offline mode: cannot install system packages".to_owned(),
                ));
            }
            let pkg_mgr = detect_package_manager(&sandbox.overlay_merged)
                .or_else(|| detect_package_manager(&rootfs))
                .ok_or_else(|| {
                    RuntimeError::ExecFailed(
                        "no supported package manager found in the image. \
                         Supported: apt, dnf, zypper, pacman"
                            .to_owned(),
                    )
                })?;

            progress(&format!(
                "installing {} packages via {pkg_mgr}...",
                spec.manifest.system_packages.len()
            ));

            let install_cmd = install_packages_command(pkg_mgr, &spec.manifest.system_packages);
            install_packages_in_container(&sandbox, &install_cmd)?;

            progress("packages installed");
        }

        unmount_overlay(&sandbox)?;

        std::fs::write(env_dir.join(".built"), "1")?;

        progress(&format!(
            "environment {} built successfully ({} base)",
            &spec.env_id[..12.min(spec.env_id.len())],
            resolved.display_name
        ));

        Ok(())
    }

    fn enter(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let env_dir = self.env_dir(&spec.env_id);
        if !env_dir.join(".built").exists() {
            return Err(RuntimeError::ExecFailed(format!(
                "environment {} has not been built yet. Run 'karapace build' first.",
                &spec.env_id[..12.min(spec.env_id.len())]
            )));
        }

        let resolved = resolve_image(&spec.manifest.base_image)?;
        let image_cache = ImageCache::new(&self.store_root);
        let rootfs = image_cache.rootfs_path(&resolved.cache_key);

        if !rootfs.join("etc").exists() {
            return Err(RuntimeError::ExecFailed(
                "base image rootfs is missing or corrupted. Run 'karapace rebuild'.".to_owned(),
            ));
        }

        let mut sandbox = SandboxConfig::new(rootfs, &spec.env_id, &env_dir);
        sandbox.isolate_network = spec.offline || spec.manifest.network_isolation;
        sandbox.hostname = format!("karapace-{}", &spec.env_id[..12.min(spec.env_id.len())]);

        let host = compute_host_integration(&spec.manifest);
        sandbox.bind_mounts.extend(host.bind_mounts);
        sandbox.env_vars.extend(host.env_vars);

        mount_overlay(&sandbox)?;
        setup_container_rootfs(&sandbox)?;

        terminal::emit_container_push(&spec.env_id, &sandbox.hostname);
        terminal::print_container_banner(
            &spec.env_id,
            &spec.manifest.base_image,
            &sandbox.hostname,
        );

        let mut child = match spawn_enter_interactive(&sandbox) {
            Ok(c) => c,
            Err(e) => {
                terminal::emit_container_pop();
                terminal::print_container_exit(&spec.env_id);
                let _ = unmount_overlay(&sandbox);
                return Err(e);
            }
        };

        if let Err(e) = std::fs::write(env_dir.join(".running"), format!("{}", child.id())) {
            let _ = child.kill();
            terminal::emit_container_pop();
            terminal::print_container_exit(&spec.env_id);
            let _ = unmount_overlay(&sandbox);
            return Err(e.into());
        }

        // Wait for the interactive session to complete.
        let exit_code = match child.wait() {
            Ok(status) => {
                let code = status.code().unwrap_or_else(|| match status.signal() {
                    Some(sig) if sig == SIGTERM || sig == SIGKILL => 0,
                    _ => 1,
                });
                Ok(code)
            }
            Err(e) => Err(RuntimeError::ExecFailed(format!(
                "failed to wait for sandbox: {e}"
            ))),
        };

        // Cleanup
        terminal::emit_container_pop();
        terminal::print_container_exit(&spec.env_id);
        let _ = std::fs::remove_file(env_dir.join(".running"));
        let _ = unmount_overlay(&sandbox);

        match exit_code {
            Ok(0) => Ok(()),
            Ok(code) => Err(RuntimeError::ExecFailed(format!(
                "container shell exited with code {code}"
            ))),
            Err(e) => Err(e),
        }
    }

    fn exec(
        &self,
        spec: &RuntimeSpec,
        command: &[String],
    ) -> Result<std::process::Output, RuntimeError> {
        let env_dir = self.env_dir(&spec.env_id);
        if !env_dir.join(".built").exists() {
            return Err(RuntimeError::ExecFailed(format!(
                "environment {} has not been built yet. Run 'karapace build' first.",
                &spec.env_id[..12.min(spec.env_id.len())]
            )));
        }

        let resolved = resolve_image(&spec.manifest.base_image)?;
        let image_cache = ImageCache::new(&self.store_root);
        let rootfs = image_cache.rootfs_path(&resolved.cache_key);

        let mut sandbox = SandboxConfig::new(rootfs, &spec.env_id, &env_dir);
        sandbox.isolate_network = spec.offline || spec.manifest.network_isolation;

        let host = compute_host_integration(&spec.manifest);
        sandbox.bind_mounts.extend(host.bind_mounts);
        sandbox.env_vars.extend(host.env_vars);

        mount_overlay(&sandbox)?;
        setup_container_rootfs(&sandbox)?;

        let output = exec_in_container(&sandbox, command);
        let _ = unmount_overlay(&sandbox);

        output
    }

    fn destroy(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let env_dir = self.env_dir(&spec.env_id);

        // Unmount overlay if mounted
        let sandbox_config =
            SandboxConfig::new(PathBuf::from("/nonexistent"), &spec.env_id, &env_dir);
        let _ = unmount_overlay(&sandbox_config);

        // Remove environment directory (force_remove handles restrictive permissions)
        force_remove(&env_dir)?;

        Ok(())
    }

    fn status(&self, env_id: &str) -> Result<RuntimeStatus, RuntimeError> {
        let env_dir = self.env_dir(env_id);
        let running_file = env_dir.join(".running");

        if running_file.exists() {
            let pid_str = match std::fs::read_to_string(&running_file) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(
                        "failed to read .running file for {}: {e}",
                        &env_id[..12.min(env_id.len())]
                    );
                    return Ok(RuntimeStatus {
                        env_id: env_id.to_owned(),
                        running: false,
                        pid: None,
                    });
                }
            };
            let pid = pid_str.trim().parse::<u32>().ok();
            if pid.is_none() && !pid_str.trim().is_empty() {
                tracing::warn!(
                    "corrupt .running file for {}: could not parse PID from '{}'",
                    &env_id[..12.min(env_id.len())],
                    pid_str.trim()
                );
                let _ = std::fs::remove_file(&running_file);
            }
            // Check if process is actually alive
            if let Some(p) = pid {
                let alive = Path::new(&format!("/proc/{p}")).exists();
                if !alive {
                    let _ = std::fs::remove_file(&running_file);
                    return Ok(RuntimeStatus {
                        env_id: env_id.to_owned(),
                        running: false,
                        pid: None,
                    });
                }
                return Ok(RuntimeStatus {
                    env_id: env_id.to_owned(),
                    running: true,
                    pid: Some(p),
                });
            }
        }

        Ok(RuntimeStatus {
            env_id: env_id.to_owned(),
            running: false,
            pid: None,
        })
    }
}

fn default_store_root() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/karapace")
    } else {
        PathBuf::from("/tmp/karapace")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_backend_available() {
        let backend = NamespaceBackend::new();
        // This test verifies the check runs without panicking.
        // Result depends on host system capabilities.
        let _ = backend.available();
    }

    #[test]
    fn status_reports_not_running_for_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let backend = NamespaceBackend::with_store_root(dir.path());
        let status = backend.status("nonexistent").unwrap();
        assert!(!status.running);
    }
}
