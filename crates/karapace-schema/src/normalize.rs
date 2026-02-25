use crate::manifest::{ManifestError, ManifestV1};
use serde::{Deserialize, Serialize};

/// Canonical, sorted, deduplicated representation of a parsed manifest.
///
/// All optional fields are resolved to defaults, packages are sorted, and mounts
/// are validated. This is the input to identity hashing and lock file generation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedManifest {
    pub manifest_version: u32,
    pub base_image: String,
    pub system_packages: Vec<String>,
    pub gui_apps: Vec<String>,
    pub hardware_gpu: bool,
    pub hardware_audio: bool,
    pub mounts: Vec<NormalizedMount>,
    pub runtime_backend: String,
    pub network_isolation: bool,
    pub cpu_shares: Option<u64>,
    pub memory_limit_mb: Option<u64>,
}

/// A validated bind-mount specification with label, host path, and container path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NormalizedMount {
    pub label: String,
    pub host_path: String,
    pub container_path: String,
}

impl ManifestV1 {
    /// Normalize the manifest: validate fields, sort packages, resolve defaults.
    pub fn normalize(&self) -> Result<NormalizedManifest, ManifestError> {
        if self.manifest_version != 1 {
            return Err(ManifestError::UnsupportedVersion(self.manifest_version));
        }

        let base_image = self.base.image.trim().to_owned();
        if base_image.is_empty() {
            return Err(ManifestError::EmptyBaseImage);
        }

        let mut mounts = Vec::with_capacity(self.mounts.entries.len());
        for (label, spec) in &self.mounts.entries {
            let trimmed_label = label.trim().to_owned();
            if trimmed_label.is_empty() {
                return Err(ManifestError::EmptyMountLabel);
            }
            let (host_path, container_path) = parse_mount_spec(label, spec)?;
            mounts.push(NormalizedMount {
                label: trimmed_label,
                host_path,
                container_path,
            });
        }
        mounts.sort_by(|a, b| a.label.cmp(&b.label));

        let runtime_backend = self.runtime.backend.trim().to_lowercase();

        Ok(NormalizedManifest {
            manifest_version: self.manifest_version,
            base_image,
            system_packages: normalize_string_list(&self.system.packages),
            gui_apps: normalize_string_list(&self.gui.apps),
            hardware_gpu: self.hardware.gpu,
            hardware_audio: self.hardware.audio,
            mounts,
            runtime_backend,
            network_isolation: self.runtime.network_isolation,
            cpu_shares: self.runtime.resource_limits.cpu_shares,
            memory_limit_mb: self.runtime.resource_limits.memory_limit_mb,
        })
    }
}

impl NormalizedManifest {
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

fn parse_mount_spec(label: &str, spec: &str) -> Result<(String, String), ManifestError> {
    let Some((host_raw, container_raw)) = spec.split_once(':') else {
        return Err(ManifestError::InvalidMount {
            label: label.to_owned(),
            spec: spec.to_owned(),
        });
    };

    let host_path = host_raw.trim().to_owned();
    let container_path = container_raw.trim().to_owned();

    if host_path.is_empty() || container_path.is_empty() {
        return Err(ManifestError::InvalidMount {
            label: label.to_owned(),
            spec: spec.to_owned(),
        });
    }

    Ok((host_path, container_path))
}

fn normalize_string_list(values: &[String]) -> Vec<String> {
    let mut out: Vec<String> = values
        .iter()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use crate::manifest::parse_manifest_str;

    #[test]
    fn normalizes_and_sorts_deterministically() {
        let input = r#"
manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["git", "cmake", "git", "clang"]

[gui]
apps = ["debugger", "ide"]

[hardware]
gpu = true
audio = false

[mounts]
workspace = "./:/workspace"
cache = "~/.cache:/cache"
"#;
        let manifest = parse_manifest_str(input).unwrap();
        let normalized = manifest.normalize().unwrap();

        assert_eq!(normalized.system_packages, vec!["clang", "cmake", "git"]);
        assert_eq!(normalized.gui_apps, vec!["debugger", "ide"]);
        assert_eq!(normalized.mounts[0].label, "cache");
        assert_eq!(normalized.mounts[1].label, "workspace");
        assert_eq!(normalized.runtime_backend, "namespace");
    }

    #[test]
    fn equivalent_manifests_produce_same_canonical_json() {
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

        assert_eq!(a.canonical_json().unwrap(), b.canonical_json().unwrap());
    }

    #[test]
    fn rejects_empty_base_image() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "   "
"#,
        )
        .unwrap();
        assert!(manifest.normalize().is_err());
    }

    #[test]
    fn rejects_invalid_mount_spec() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[mounts]
workspace = "./no-colon"
"#,
        )
        .unwrap();
        assert!(manifest.normalize().is_err());
    }

    #[test]
    fn runtime_backend_included_in_normalization() {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[runtime]
backend = "OCI"
"#,
        )
        .unwrap();
        let normalized = manifest.normalize().unwrap();
        assert_eq!(normalized.runtime_backend, "oci");
    }
}
