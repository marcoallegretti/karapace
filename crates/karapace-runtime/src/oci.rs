use crate::backend::{RuntimeBackend, RuntimeSpec, RuntimeStatus};
use crate::host::compute_host_integration;
use crate::image::{
    compute_image_digest, detect_package_manager, force_remove, install_packages_command,
    parse_version_output, query_versions_command, resolve_image, ImageCache,
};
use crate::sandbox::{
    exec_in_container, install_packages_in_container, mount_overlay, setup_container_rootfs,
    unmount_overlay, SandboxConfig,
};
use crate::terminal;
use crate::RuntimeError;
use karapace_schema::{ResolutionResult, ResolvedPackage};
use std::path::PathBuf;
use std::process::Command;

pub struct OciBackend {
    store_root: PathBuf,
}

impl Default for OciBackend {
    fn default() -> Self {
        Self {
            store_root: default_store_root(),
        }
    }
}

impl OciBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_store_root(store_root: impl Into<PathBuf>) -> Self {
        Self {
            store_root: store_root.into(),
        }
    }

    fn find_runtime() -> Option<String> {
        for candidate in &["crun", "runc", "youki"] {
            if let Ok(output) = Command::new(candidate).arg("--version").output() {
                if output.status.success() {
                    return Some(candidate.to_string());
                }
            }
        }
        None
    }

    fn env_dir(&self, env_id: &str) -> PathBuf {
        self.store_root.join("env").join(env_id)
    }

    fn generate_oci_spec(config: &SandboxConfig, spec: &RuntimeSpec) -> String {
        let uid = config.uid;
        let gid = config.gid;
        let home = config.home_dir.display().to_string();
        let hostname = &config.hostname;

        let mut env_arr = Vec::new();
        env_arr.push(format!("\"HOME={home}\""));
        env_arr.push(format!("\"USER={}\"", config.username));
        env_arr.push(format!("\"HOSTNAME={hostname}\""));
        env_arr.push("\"TERM=xterm-256color\"".to_owned());
        env_arr.push("\"KARAPACE_ENV=1\"".to_owned());
        for (k, v) in &config.env_vars {
            env_arr.push(format!("\"{}={}\"", k, v.replace('"', "\\\"")));
        }

        let mut mounts = Vec::new();
        // Standard mounts
        mounts.push(r#"{"destination":"/proc","type":"proc","source":"proc"}"#.to_owned());
        mounts.push(
            r#"{"destination":"/dev","type":"tmpfs","source":"tmpfs","options":["nosuid","strictatime","mode=755","size=65536k"]}"#
                .to_owned(),
        );
        mounts.push(
            r#"{"destination":"/dev/pts","type":"devpts","source":"devpts","options":["nosuid","noexec","newinstance","ptmxmode=0666","mode=0620"]}"#
                .to_owned(),
        );
        mounts.push(
            r#"{"destination":"/dev/shm","type":"tmpfs","source":"shm","options":["nosuid","noexec","nodev","mode=1777","size=65536k"]}"#
                .to_owned(),
        );
        mounts.push(
            r#"{"destination":"/sys","type":"sysfs","source":"sysfs","options":["nosuid","noexec","nodev","ro"]}"#
                .to_owned(),
        );

        // Home bind mount
        mounts.push(format!(
            r#"{{"destination":"{home}","type":"bind","source":"{home}","options":["rbind","rw"]}}"#
        ));

        // resolv.conf
        mounts.push(
            r#"{"destination":"/etc/resolv.conf","type":"bind","source":"/etc/resolv.conf","options":["bind","ro"]}"#
                .to_owned(),
        );

        // Custom bind mounts
        for bm in &config.bind_mounts {
            let opts = if bm.read_only {
                "\"rbind\",\"ro\""
            } else {
                "\"rbind\",\"rw\""
            };
            mounts.push(format!(
                r#"{{"destination":"{}","type":"bind","source":"{}","options":[{}]}}"#,
                bm.target.display(),
                bm.source.display(),
                opts
            ));
        }

        let mounts_json = mounts.join(",");
        let env_json = env_arr.join(",");

        let network_ns = if spec.manifest.network_isolation {
            r#",{"type":"network"}"#
        } else {
            ""
        };

        let oci_spec = format!(
            r#"{{
  "ociVersion": "1.0.2",
  "process": {{
    "terminal": true,
    "user": {{ "uid": {uid}, "gid": {gid} }},
    "args": ["/bin/bash", "-l"],
    "env": [{env_json}],
    "cwd": "{home}"
  }},
  "root": {{
    "path": "rootfs",
    "readonly": false
  }},
  "hostname": "{hostname}",
  "mounts": [{mounts_json}],
  "linux": {{
    "namespaces": [
      {{"type":"pid"}},
      {{"type":"mount"}},
      {{"type":"ipc"}},
      {{"type":"uts"}}
      {network_ns}
    ],
    "uidMappings": [{{ "containerID": 0, "hostID": {uid}, "size": 1 }}],
    "gidMappings": [{{ "containerID": 0, "hostID": {gid}, "size": 1 }}]
  }}
}}"#
        );

        oci_spec
    }
}

impl RuntimeBackend for OciBackend {
    fn name(&self) -> &'static str {
        "oci"
    }

    fn available(&self) -> bool {
        Self::find_runtime().is_some()
    }

    fn resolve(&self, spec: &RuntimeSpec) -> Result<ResolutionResult, RuntimeError> {
        let progress = |msg: &str| {
            eprintln!("[karapace/oci] {msg}");
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

            // Run resolution inside an inner closure so cleanup always runs,
            // even if detect/install/query fails.
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

            // Always cleanup: unmount overlay and remove temp directory
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
            eprintln!("[karapace/oci] {msg}");
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
                        "no supported package manager found in the image".to_owned(),
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

        // Generate OCI bundle config.json
        let bundle_dir = env_dir.join("bundle");
        std::fs::create_dir_all(&bundle_dir)?;

        // Symlink rootfs into bundle
        let bundle_rootfs = bundle_dir.join("rootfs");
        if !bundle_rootfs.exists() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&sandbox.overlay_merged, &bundle_rootfs)?;
        }

        std::fs::write(env_dir.join(".built"), "1")?;

        progress(&format!(
            "environment {} built (OCI, {} base)",
            &spec.env_id[..12.min(spec.env_id.len())],
            resolved.display_name
        ));

        Ok(())
    }

    fn enter(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let runtime = Self::find_runtime().ok_or_else(|| {
            RuntimeError::BackendUnavailable("no OCI runtime found (crun/runc/youki)".to_owned())
        })?;

        let env_dir = self.env_dir(&spec.env_id);
        if !env_dir.join(".built").exists() {
            return Err(RuntimeError::ExecFailed(format!(
                "environment {} has not been built",
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

        // Write OCI config.json
        let bundle_dir = env_dir.join("bundle");
        std::fs::create_dir_all(&bundle_dir)?;

        let bundle_rootfs = bundle_dir.join("rootfs");
        if !bundle_rootfs.exists() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&sandbox.overlay_merged, &bundle_rootfs)?;
        }

        let oci_config = Self::generate_oci_spec(&sandbox, spec);
        std::fs::write(bundle_dir.join("config.json"), &oci_config)?;

        let container_id = format!("karapace-{}", &spec.env_id[..12.min(spec.env_id.len())]);

        std::fs::write(env_dir.join(".running"), format!("{}", std::process::id()))?;

        terminal::emit_container_push(&spec.env_id, &sandbox.hostname);
        terminal::print_container_banner(
            &spec.env_id,
            &spec.manifest.base_image,
            &sandbox.hostname,
        );

        let status = Command::new(&runtime)
            .args([
                "run",
                "--bundle",
                &bundle_dir.to_string_lossy(),
                &container_id,
            ])
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status()
            .map_err(|e| RuntimeError::ExecFailed(format!("{runtime} run failed: {e}")))?;

        terminal::emit_container_pop();
        terminal::print_container_exit(&spec.env_id);
        let _ = std::fs::remove_file(env_dir.join(".running"));
        let _ = unmount_overlay(&sandbox);

        // Clean up OCI container state
        let _ = Command::new(&runtime)
            .args(["delete", "--force", &container_id])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();

        if status.success() {
            Ok(())
        } else {
            Err(RuntimeError::ExecFailed(format!(
                "OCI runtime exited with code {}",
                status.code().unwrap_or(1)
            )))
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
                "environment {} has not been built yet",
                &spec.env_id[..12.min(spec.env_id.len())]
            )));
        }

        let resolved = resolve_image(&spec.manifest.base_image)?;
        let image_cache = ImageCache::new(&self.store_root);
        let rootfs = image_cache.rootfs_path(&resolved.cache_key);

        let mut sandbox = SandboxConfig::new(rootfs, &spec.env_id, &env_dir);
        sandbox.isolate_network = spec.manifest.network_isolation;

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
        let sandbox = SandboxConfig::new(PathBuf::from("/nonexistent"), &spec.env_id, &env_dir);
        let _ = unmount_overlay(&sandbox);

        // Clean up any lingering OCI containers
        if let Some(runtime) = Self::find_runtime() {
            let container_id = format!("karapace-{}", &spec.env_id[..12.min(spec.env_id.len())]);
            let _ = Command::new(&runtime)
                .args(["delete", "--force", &container_id])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }

        force_remove(&env_dir)?;
        Ok(())
    }

    fn status(&self, env_id: &str) -> Result<RuntimeStatus, RuntimeError> {
        let runtime = Self::find_runtime().ok_or_else(|| {
            RuntimeError::BackendUnavailable("no OCI runtime found (crun/runc/youki)".to_owned())
        })?;

        let container_id = format!("karapace-{}", &env_id[..12.min(env_id.len())]);
        let output = Command::new(&runtime)
            .args(["state", &container_id])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.to_lowercase();
            if msg.contains("does not exist")
                || msg.contains("not found")
                || msg.contains("no such file or directory")
            {
                return Ok(RuntimeStatus {
                    env_id: env_id.to_owned(),
                    running: false,
                    pid: None,
                });
            }
            return Err(RuntimeError::ExecFailed(format!(
                "{runtime} state failed: {}",
                stderr.trim()
            )));
        }

        let state: serde_json::Value = serde_json::from_slice(&output.stdout).map_err(|e| {
            RuntimeError::ExecFailed(format!("failed to parse {runtime} state output: {e}"))
        })?;

        let pid = state
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|p| u32::try_from(p).ok())
            .filter(|p| *p != 0);

        Ok(RuntimeStatus {
            env_id: env_id.to_owned(),
            running: pid.is_some(),
            pid,
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
    fn oci_env_dir_layout() {
        let dir = tempfile::tempdir().unwrap();
        let backend = OciBackend::with_store_root(dir.path());
        let env_dir = backend.env_dir("abc123");
        assert_eq!(env_dir, dir.path().join("env").join("abc123"));
    }

    #[test]
    fn oci_status_reports_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let backend = OciBackend::with_store_root(dir.path());
        let status = backend.status("oci-test").unwrap();
        assert!(!status.running);
    }

    #[test]
    fn oci_availability_check() {
        let backend = OciBackend::new();
        // Just ensure this doesn't panic; result depends on host
        let _ = backend.available();
    }
}
