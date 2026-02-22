use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub manifest: &'static str,
}

pub const BUILTIN_PRESETS: &[Preset] = &[
    Preset {
        name: "dev",
        description: "Development environment with common build tools",
        manifest: r#"manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["git", "curl", "wget", "vim", "gcc", "make", "cmake"]

[runtime]
backend = "namespace"
"#,
    },
    Preset {
        name: "dev-rust",
        description: "Rust development environment",
        manifest: r#"manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["git", "curl", "gcc", "make", "rustup"]

[runtime]
backend = "namespace"
"#,
    },
    Preset {
        name: "dev-python",
        description: "Python development environment",
        manifest: r#"manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["git", "curl", "python3", "python3-pip", "python3-venv"]

[runtime]
backend = "namespace"
"#,
    },
    Preset {
        name: "gui-app",
        description: "GUI application environment with GPU and audio passthrough",
        manifest: r#"manifest_version = 1

[base]
image = "rolling"

[hardware]
gpu = true
audio = true

[runtime]
backend = "namespace"
"#,
    },
    Preset {
        name: "gaming",
        description: "Gaming environment with GPU, audio, and Vulkan support",
        manifest: r#"manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["mesa-dri", "vulkan-loader", "libvulkan1", "alsa-plugins"]

[hardware]
gpu = true
audio = true

[runtime]
backend = "namespace"
"#,
    },
    Preset {
        name: "minimal",
        description: "Minimal environment with no extra packages",
        manifest: r#"manifest_version = 1

[base]
image = "rolling"

[runtime]
backend = "namespace"
"#,
    },
];

pub fn get_preset(name: &str) -> Option<&'static Preset> {
    BUILTIN_PRESETS.iter().find(|p| p.name == name)
}

pub fn list_presets() -> &'static [Preset] {
    BUILTIN_PRESETS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_parse() {
        for preset in BUILTIN_PRESETS {
            let result = crate::parse_manifest_str(preset.manifest);
            assert!(
                result.is_ok(),
                "preset '{}' failed to parse: {:?}",
                preset.name,
                result.err()
            );
        }
    }

    #[test]
    fn get_preset_by_name() {
        assert!(get_preset("dev").is_some());
        assert!(get_preset("nonexistent").is_none());
    }

    #[test]
    fn all_presets_have_unique_names() {
        let mut names: Vec<&str> = BUILTIN_PRESETS.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), BUILTIN_PRESETS.len());
    }
}
