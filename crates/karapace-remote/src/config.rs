use crate::RemoteError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub url: String,
    #[serde(default)]
    pub auth_token: Option<String>,
}

impl RemoteConfig {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_owned(),
            auth_token: None,
        }
    }

    #[must_use]
    pub fn with_token(mut self, token: &str) -> Self {
        self.auth_token = Some(token.to_owned());
        self
    }

    /// Load config from `~/.config/karapace/remote.json`.
    pub fn load_default() -> Result<Self, RemoteError> {
        let path = default_config_path()?;
        Self::load(&path)
    }

    pub fn load(path: &Path) -> Result<Self, RemoteError> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| RemoteError::Config(format!("invalid remote config: {e}")))
    }

    pub fn save(&self, path: &Path) -> Result<(), RemoteError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| RemoteError::Serialization(e.to_string()))?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

fn default_config_path() -> Result<PathBuf, RemoteError> {
    let home = std::env::var("HOME").map_err(|_| RemoteError::Config("HOME not set".to_owned()))?;
    Ok(PathBuf::from(home).join(".config/karapace/remote.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote.json");

        let config = RemoteConfig::new("https://store.example.com/v1").with_token("secret123");
        config.save(&path).unwrap();

        let loaded = RemoteConfig::load(&path).unwrap();
        assert_eq!(loaded.url, "https://store.example.com/v1");
        assert_eq!(loaded.auth_token.as_deref(), Some("secret123"));
    }

    #[test]
    fn config_strips_trailing_slash() {
        let config = RemoteConfig::new("https://example.com/");
        assert_eq!(config.url, "https://example.com");
    }
}
