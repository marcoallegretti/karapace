use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse manifest: {0}")]
    ParseToml(#[from] toml::de::Error),
    #[error("unsupported manifest_version: {0}, expected 1")]
    UnsupportedVersion(u32),
    #[error("base.image must not be empty")]
    EmptyBaseImage,
    #[error("base.image is not pinned: '{0}' (expected http(s)://...)")]
    UnpinnedBaseImage(String),
    #[error("mount label must not be empty")]
    EmptyMountLabel,
    #[error("invalid mount declaration for '{label}': '{spec}', expected '<host>:<container>'")]
    InvalidMount { label: String, spec: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestV1 {
    pub manifest_version: u32,
    pub base: BaseSection,
    #[serde(default)]
    pub system: SystemSection,
    #[serde(default)]
    pub gui: GuiSection,
    #[serde(default)]
    pub hardware: HardwareSection,
    #[serde(default)]
    pub mounts: MountsSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BaseSection {
    pub image: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SystemSection {
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GuiSection {
    #[serde(default)]
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HardwareSection {
    #[serde(default)]
    pub gpu: bool,
    #[serde(default)]
    pub audio: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MountsSection {
    #[serde(flatten)]
    pub entries: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSection {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default)]
    pub network_isolation: bool,
    #[serde(default)]
    pub resource_limits: ResourceLimits,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            network_isolation: false,
            resource_limits: ResourceLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceLimits {
    #[serde(default)]
    pub cpu_shares: Option<u64>,
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
}

fn default_backend() -> String {
    "namespace".to_owned()
}

pub fn parse_manifest_str(input: &str) -> Result<ManifestV1, ManifestError> {
    Ok(toml::from_str(input)?)
}

pub fn parse_manifest_file(path: impl AsRef<Path>) -> Result<ManifestV1, ManifestError> {
    let content = fs::read_to_string(path)?;
    parse_manifest_str(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_manifest() {
        let input = r#"
manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["clang", "cmake", "git"]

[gui]
apps = ["ide", "debugger"]

[hardware]
gpu = true
audio = true

[mounts]
workspace = "./:/workspace"

[runtime]
backend = "oci"
network_isolation = true

[runtime.resource_limits]
cpu_shares = 1024
memory_limit_mb = 4096
"#;
        let manifest = parse_manifest_str(input).expect("should parse");
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(manifest.base.image, "rolling");
        assert_eq!(manifest.system.packages.len(), 3);
        assert_eq!(manifest.runtime.backend, "oci");
        assert!(manifest.runtime.network_isolation);
        assert_eq!(manifest.runtime.resource_limits.cpu_shares, Some(1024));
    }

    #[test]
    fn parses_minimal_manifest() {
        let input = r#"
manifest_version = 1

[base]
image = "rolling"
"#;
        let manifest = parse_manifest_str(input).expect("should parse");
        assert_eq!(manifest.runtime.backend, "namespace");
        assert!(!manifest.runtime.network_isolation);
    }

    #[test]
    fn rejects_unknown_fields() {
        let input = r#"
manifest_version = 1

[base]
image = "rolling"
unknown_field = true
"#;
        assert!(parse_manifest_str(input).is_err());
    }

    #[test]
    fn rejects_missing_base() {
        let input = r"
manifest_version = 1
";
        assert!(parse_manifest_str(input).is_err());
    }
}
