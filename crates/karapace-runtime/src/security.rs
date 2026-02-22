use crate::RuntimeError;
use karapace_schema::NormalizedManifest;
use serde::{Deserialize, Serialize};

/// Resolve `.` and `..` components in an absolute path without touching the filesystem.
///
/// This is critical for security: we must not rely on `std::fs::canonicalize()`
/// because the path may not exist yet, and we need deterministic behavior.
fn canonicalize_logical(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityPolicy {
    pub allowed_mount_prefixes: Vec<String>,
    pub allowed_devices: Vec<String>,
    pub allow_network: bool,
    pub allow_gpu: bool,
    pub allow_audio: bool,
    pub allowed_env_vars: Vec<String>,
    pub denied_env_vars: Vec<String>,
    pub max_cpu_shares: Option<u64>,
    pub max_memory_mb: Option<u64>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            allowed_mount_prefixes: vec!["/home".to_owned(), "/tmp".to_owned()],
            allowed_devices: Vec::new(),
            allow_network: false,
            allow_gpu: false,
            allow_audio: false,
            allowed_env_vars: vec![
                "TERM".to_owned(),
                "LANG".to_owned(),
                "HOME".to_owned(),
                "USER".to_owned(),
                "PATH".to_owned(),
                "SHELL".to_owned(),
                "XDG_RUNTIME_DIR".to_owned(),
            ],
            denied_env_vars: vec![
                "SSH_AUTH_SOCK".to_owned(),
                "GPG_AGENT_INFO".to_owned(),
                "AWS_SECRET_ACCESS_KEY".to_owned(),
                "DOCKER_HOST".to_owned(),
            ],
            max_cpu_shares: None,
            max_memory_mb: None,
        }
    }
}

impl SecurityPolicy {
    pub fn from_manifest(manifest: &NormalizedManifest) -> Self {
        let mut allowed_devices = Vec::new();
        if manifest.hardware_gpu {
            allowed_devices.push("/dev/dri".to_owned());
        }
        if manifest.hardware_audio {
            allowed_devices.push("/dev/snd".to_owned());
        }

        Self {
            allow_gpu: manifest.hardware_gpu,
            allow_audio: manifest.hardware_audio,
            allow_network: !manifest.network_isolation,
            allowed_devices,
            max_cpu_shares: manifest.cpu_shares,
            max_memory_mb: manifest.memory_limit_mb,
            ..Self::default()
        }
    }

    pub fn validate_mounts(&self, manifest: &NormalizedManifest) -> Result<(), RuntimeError> {
        for mount in &manifest.mounts {
            let host = &mount.host_path;
            if host.starts_with('/') {
                let canonical = canonicalize_logical(host);
                let allowed = self
                    .allowed_mount_prefixes
                    .iter()
                    .any(|prefix| canonical.starts_with(prefix));
                if !allowed {
                    return Err(RuntimeError::MountDenied(format!(
                        "mount '{host}' (resolved: {canonical}) is not under any allowed prefix: {:?}",
                        self.allowed_mount_prefixes
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn validate_devices(&self, manifest: &NormalizedManifest) -> Result<(), RuntimeError> {
        if manifest.hardware_gpu && !self.allow_gpu {
            return Err(RuntimeError::DeviceDenied(
                "GPU access requested but not allowed by policy".to_owned(),
            ));
        }
        if manifest.hardware_audio && !self.allow_audio {
            return Err(RuntimeError::DeviceDenied(
                "audio access requested but not allowed by policy".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn filter_env_vars(&self) -> Vec<(String, String)> {
        let mut result = Vec::new();
        for key in &self.allowed_env_vars {
            if self.denied_env_vars.contains(key) {
                continue;
            }
            if let Ok(val) = std::env::var(key) {
                result.push((key.clone(), val));
            }
        }
        result
    }

    pub fn validate_resource_limits(
        &self,
        manifest: &NormalizedManifest,
    ) -> Result<(), RuntimeError> {
        if let (Some(req), Some(max)) = (manifest.cpu_shares, self.max_cpu_shares) {
            if req > max {
                return Err(RuntimeError::PolicyViolation(format!(
                    "requested CPU shares {req} exceeds policy max {max}"
                )));
            }
        }
        if let (Some(req), Some(max)) = (manifest.memory_limit_mb, self.max_memory_mb) {
            if req > max {
                return Err(RuntimeError::PolicyViolation(format!(
                    "requested memory {req}MB exceeds policy max {max}MB"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use karapace_schema::parse_manifest_str;

    #[test]
    fn default_policy_denies_gpu() {
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

        let policy = SecurityPolicy::default();
        assert!(policy.validate_devices(&manifest).is_err());
    }

    #[test]
    fn manifest_derived_policy_allows_declared_hardware() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[hardware]
gpu = true
audio = true
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::from_manifest(&manifest);
        assert!(policy.validate_devices(&manifest).is_ok());
        assert!(policy.allow_gpu);
        assert!(policy.allow_audio);
        assert!(policy.allowed_devices.contains(&"/dev/dri".to_owned()));
    }

    #[test]
    fn absolute_mounts_checked_against_whitelist() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
bad = "/etc/shadow:/secrets"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        assert!(policy.validate_mounts(&manifest).is_err());
    }

    #[test]
    fn denied_env_vars_are_filtered() {
        let policy = SecurityPolicy::default();
        assert!(policy.denied_env_vars.contains(&"SSH_AUTH_SOCK".to_owned()));
        assert!(policy
            .denied_env_vars
            .contains(&"AWS_SECRET_ACCESS_KEY".to_owned()));
        let filtered = policy.filter_env_vars();
        assert!(filtered
            .iter()
            .all(|(k, _)| !policy.denied_env_vars.contains(k)));
    }

    #[test]
    fn resource_limits_enforced() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[runtime]
backend = "namespace"
[runtime.resource_limits]
cpu_shares = 2048
memory_limit_mb = 8192
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let mut policy = SecurityPolicy::from_manifest(&manifest);
        assert!(policy.validate_resource_limits(&manifest).is_ok());

        policy.max_cpu_shares = Some(1024);
        assert!(policy.validate_resource_limits(&manifest).is_err());
    }

    #[test]
    fn relative_mounts_always_allowed() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
workspace = "./:/workspace"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        assert!(policy.validate_mounts(&manifest).is_ok());
    }

    #[test]
    fn path_traversal_via_dotdot_is_rejected() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
bad = "/../etc/shadow:/secrets"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        assert!(
            policy.validate_mounts(&manifest).is_err(),
            "path traversal via /../ must be rejected"
        );
    }

    #[test]
    fn path_traversal_via_allowed_prefix_breakout_is_rejected() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
bad = "/home/../etc/passwd:/data"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        // /home/../etc/passwd canonicalizes to /etc/passwd, which is NOT under /home
        assert!(
            policy.validate_mounts(&manifest).is_err(),
            "/home/../etc/passwd must be rejected (resolves to /etc/passwd)"
        );
    }

    #[test]
    fn root_path_is_rejected() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
bad = "/:/rootfs"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        assert!(
            policy.validate_mounts(&manifest).is_err(),
            "mounting / must be rejected"
        );
    }

    #[test]
    fn etc_shadow_is_rejected() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
bad = "/etc/shadow:/shadow"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        assert!(
            policy.validate_mounts(&manifest).is_err(),
            "/etc/shadow must be rejected"
        );
    }

    #[test]
    fn proc_is_rejected() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
bad = "/proc/self/root:/escape"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let policy = SecurityPolicy::default();
        assert!(
            policy.validate_mounts(&manifest).is_err(),
            "/proc must be rejected"
        );
    }
}
