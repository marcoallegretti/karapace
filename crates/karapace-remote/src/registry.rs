use crate::RemoteError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A single entry in the remote registry, mapping a tag to an env_id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryEntry {
    pub env_id: String,
    pub short_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub pushed_at: String,
}

/// The registry index: maps `name@tag` keys to environment entries.
/// Example: `"my-env@latest"` → `RegistryEntry { env_id: "abc...", ... }`
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Registry {
    pub entries: BTreeMap<String, RegistryEntry>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, RemoteError> {
        serde_json::from_slice(data)
            .map_err(|e| RemoteError::Serialization(format!("invalid registry: {e}")))
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, RemoteError> {
        serde_json::to_vec_pretty(self).map_err(|e| RemoteError::Serialization(e.to_string()))
    }

    /// Insert or update an entry. Key format: `name@tag` or just `env_id`.
    pub fn publish(&mut self, key: &str, entry: RegistryEntry) {
        self.entries.insert(key.to_owned(), entry);
    }

    /// Look up an entry by key.
    pub fn lookup(&self, key: &str) -> Option<&RegistryEntry> {
        self.entries.get(key)
    }

    /// List all keys in the registry.
    pub fn list_keys(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// Find entries by env_id.
    pub fn find_by_env_id(&self, env_id: &str) -> Vec<(&str, &RegistryEntry)> {
        self.entries
            .iter()
            .filter(|(_, v)| v.env_id == env_id)
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }
}

/// Parse a reference like `name@tag` into (name, tag).
/// If no `@` is present, the whole string is treated as the name with tag "latest".
pub fn parse_ref(reference: &str) -> (&str, &str) {
    match reference.split_once('@') {
        Some((name, tag)) => (name, tag),
        None => (reference, "latest"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_roundtrip() {
        let mut reg = Registry::new();
        reg.publish(
            "my-env@latest",
            RegistryEntry {
                env_id: "abc123".to_owned(),
                short_id: "abc123".to_owned(),
                name: Some("my-env".to_owned()),
                pushed_at: "2025-01-01T00:00:00Z".to_owned(),
            },
        );

        let bytes = reg.to_bytes().unwrap();
        let loaded = Registry::from_bytes(&bytes).unwrap();
        assert_eq!(loaded, reg);
    }

    #[test]
    fn registry_lookup() {
        let mut reg = Registry::new();
        reg.publish(
            "dev@v1",
            RegistryEntry {
                env_id: "hash1".to_owned(),
                short_id: "hash1".to_owned(),
                name: None,
                pushed_at: "2025-01-01T00:00:00Z".to_owned(),
            },
        );
        assert!(reg.lookup("dev@v1").is_some());
        assert!(reg.lookup("nonexistent").is_none());
    }

    #[test]
    fn parse_ref_with_tag() {
        assert_eq!(parse_ref("my-env@v2"), ("my-env", "v2"));
    }

    #[test]
    fn parse_ref_without_tag() {
        assert_eq!(parse_ref("my-env"), ("my-env", "latest"));
    }

    #[test]
    fn find_by_env_id_works() {
        let mut reg = Registry::new();
        reg.publish(
            "a@latest",
            RegistryEntry {
                env_id: "hash1".to_owned(),
                short_id: "hash1".to_owned(),
                name: None,
                pushed_at: "t".to_owned(),
            },
        );
        reg.publish(
            "b@latest",
            RegistryEntry {
                env_id: "hash1".to_owned(),
                short_id: "hash1".to_owned(),
                name: None,
                pushed_at: "t".to_owned(),
            },
        );
        reg.publish(
            "c@latest",
            RegistryEntry {
                env_id: "hash2".to_owned(),
                short_id: "hash2".to_owned(),
                name: None,
                pushed_at: "t".to_owned(),
            },
        );
        let found = reg.find_by_env_id("hash1");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn empty_registry_roundtrip() {
        let reg = Registry::new();
        let bytes = reg.to_bytes().unwrap();
        let loaded = Registry::from_bytes(&bytes).unwrap();
        assert!(loaded.entries.is_empty());
    }
}
