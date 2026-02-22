use crate::normalize::NormalizedManifest;
use crate::types::{EnvId, ShortId};
use serde::Serialize;

/// Deterministic identity for an environment, derived from its manifest content.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EnvIdentity {
    pub env_id: EnvId,
    pub short_id: ShortId,
}

/// Compute a **preliminary** environment identity from unresolved manifest data.
///
/// This is NOT the canonical identity. The canonical identity is computed by
/// [`LockFile::compute_identity()`] after dependency resolution, which uses:
/// - Actual base image content digest (not tag name hash)
/// - Resolved package versions (not just package names)
/// - Full hardware/mount/runtime policy
///
/// This function is used only for:
/// - The `init` command (before resolution has occurred)
/// - Internal lookup during rebuild (to find old environments)
///
/// After `build`, the env_id stored in metadata comes from the lock file.
pub fn compute_env_id(normalized: &NormalizedManifest) -> EnvIdentity {
    let mut hasher = blake3::Hasher::new();

    hasher.update(normalized.canonical_json().as_bytes());

    let base_digest = blake3::hash(normalized.base_image.as_bytes())
        .to_hex()
        .to_string();
    hasher.update(base_digest.as_bytes());

    for pkg in &normalized.system_packages {
        hasher.update(format!("pkg:{pkg}").as_bytes());
    }
    for app in &normalized.gui_apps {
        hasher.update(format!("app:{app}").as_bytes());
    }

    if normalized.hardware_gpu {
        hasher.update(b"hw:gpu");
    }
    if normalized.hardware_audio {
        hasher.update(b"hw:audio");
    }

    for mount in &normalized.mounts {
        hasher.update(
            format!(
                "mount:{}:{}:{}",
                mount.label, mount.host_path, mount.container_path
            )
            .as_bytes(),
        );
    }

    hasher.update(format!("backend:{}", normalized.runtime_backend).as_bytes());

    if normalized.network_isolation {
        hasher.update(b"net:isolated");
    }
    if let Some(cpu) = normalized.cpu_shares {
        hasher.update(format!("cpu:{cpu}").as_bytes());
    }
    if let Some(mem) = normalized.memory_limit_mb {
        hasher.update(format!("mem:{mem}").as_bytes());
    }

    let hex = hasher.finalize().to_hex().to_string();
    let short = hex[..12].to_owned();

    EnvIdentity {
        env_id: EnvId::new(hex),
        short_id: ShortId::new(short),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest_str;

    #[test]
    fn stable_id_for_equivalent_manifests() {
        let a = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "clang"]
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let b = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["clang", "git"]
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        assert_eq!(compute_env_id(&a), compute_env_id(&b));
    }

    #[test]
    fn different_inputs_produce_different_ids() {
        let a = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git"]
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let b = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "cmake"]
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        assert_ne!(compute_env_id(&a), compute_env_id(&b));
    }

    #[test]
    fn backend_change_changes_id() {
        let a = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[runtime]
backend = "namespace"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let b = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[runtime]
backend = "oci"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        assert_ne!(compute_env_id(&a), compute_env_id(&b));
    }

    #[test]
    fn short_id_is_12_chars() {
        let n = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let id = compute_env_id(&n);
        assert_eq!(id.short_id.as_str().len(), 12);
        assert!(id.env_id.as_str().starts_with(id.short_id.as_str()));
    }
}
