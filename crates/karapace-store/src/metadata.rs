use crate::layout::StoreLayout;
use crate::{fsync_dir, StoreError};
use karapace_schema::types::{EnvId, LayerHash, ObjectHash, ShortId};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use tempfile::NamedTempFile;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EnvState {
    Defined,
    Built,
    Running,
    Frozen,
    Archived,
}

impl std::fmt::Display for EnvState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnvState::Defined => write!(f, "defined"),
            EnvState::Built => write!(f, "built"),
            EnvState::Running => write!(f, "running"),
            EnvState::Frozen => write!(f, "frozen"),
            EnvState::Archived => write!(f, "archived"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvMetadata {
    pub env_id: EnvId,
    pub short_id: ShortId,
    #[serde(default)]
    pub name: Option<String>,
    pub state: EnvState,
    pub manifest_hash: ObjectHash,
    pub base_layer: LayerHash,
    pub dependency_layers: Vec<LayerHash>,
    pub policy_layer: Option<LayerHash>,
    pub created_at: String,
    pub updated_at: String,
    pub ref_count: u32,
    /// blake3 checksum for integrity verification. `None` for legacy metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

impl EnvMetadata {
    /// Compute the checksum over the metadata content (excluding the checksum field itself).
    fn compute_checksum(&self) -> Result<String, StoreError> {
        let mut copy = self.clone();
        copy.checksum = None;
        // Serialize without the checksum field (skip_serializing_if = None)
        let json = serde_json::to_string_pretty(&copy)?;
        Ok(blake3::hash(json.as_bytes()).to_hex().to_string())
    }
}

pub fn validate_env_name(name: &str) -> Result<(), StoreError> {
    if name.is_empty() || name.len() > 64 {
        return Err(StoreError::InvalidName(
            "environment name must be 1-64 characters".to_owned(),
        ));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(StoreError::InvalidName(
            "environment name must match [a-zA-Z0-9_-]".to_owned(),
        ));
    }
    Ok(())
}

pub struct MetadataStore {
    layout: StoreLayout,
}

impl MetadataStore {
    pub fn new(layout: StoreLayout) -> Self {
        Self { layout }
    }

    pub fn put(&self, meta: &EnvMetadata) -> Result<(), StoreError> {
        let dest = self.layout.metadata_dir().join(&meta.env_id);

        // Compute and embed checksum before writing
        let mut meta_with_checksum = meta.clone();
        meta_with_checksum.checksum = Some(meta_with_checksum.compute_checksum()?);
        let content = serde_json::to_string_pretty(&meta_with_checksum)?;

        let dir = self.layout.metadata_dir();
        let mut tmp = NamedTempFile::new_in(&dir)?;
        tmp.write_all(content.as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(&dest).map_err(|e| StoreError::Io(e.error))?;
        fsync_dir(&dir)?;

        Ok(())
    }

    pub fn get(&self, env_id: &str) -> Result<EnvMetadata, StoreError> {
        let path = self.layout.metadata_dir().join(env_id);
        if !path.exists() {
            return Err(StoreError::EnvNotFound(env_id.to_owned()));
        }
        let content = fs::read_to_string(&path)?;
        let meta: EnvMetadata = serde_json::from_str(&content)?;

        // Verify checksum if present (backward-compatible: legacy files have None)
        if let Some(ref expected) = meta.checksum {
            let actual = meta.compute_checksum()?;
            if actual != *expected {
                return Err(StoreError::IntegrityFailure {
                    hash: env_id.to_owned(),
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        Ok(meta)
    }

    pub fn update_state(&self, env_id: &str, new_state: EnvState) -> Result<(), StoreError> {
        let mut meta = self.get(env_id)?;
        meta.state = new_state;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        self.put(&meta)
    }

    pub fn exists(&self, env_id: &str) -> bool {
        self.layout.metadata_dir().join(env_id).exists()
    }

    pub fn remove(&self, env_id: &str) -> Result<(), StoreError> {
        let path = self.layout.metadata_dir().join(env_id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<EnvMetadata>, StoreError> {
        let dir = self.layout.metadata_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name();
                let name_str = name.to_str().unwrap_or("");
                if !name_str.starts_with('.') {
                    match self.get(name_str) {
                        Ok(meta) => results.push(meta),
                        Err(e) => {
                            tracing::warn!("skipping corrupted metadata entry '{name_str}': {e}");
                        }
                    }
                }
            }
        }
        results.sort_by(|a, b| a.env_id.cmp(&b.env_id));
        Ok(results)
    }

    /// Like `list()`, but returns per-entry `Result`s so callers (e.g.
    /// `verify-store`) can surface individual corruption errors.
    #[allow(clippy::type_complexity)]
    pub fn list_with_errors(
        &self,
    ) -> Result<Vec<Result<EnvMetadata, (String, StoreError)>>, StoreError> {
        let dir = self.layout.metadata_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let name = entry.file_name();
                let name_str = name.to_str().unwrap_or("").to_owned();
                if !name_str.starts_with('.') {
                    match self.get(&name_str) {
                        Ok(meta) => results.push(Ok(meta)),
                        Err(e) => results.push(Err((name_str, e))),
                    }
                }
            }
        }
        Ok(results)
    }

    pub fn increment_ref(&self, env_id: &str) -> Result<u32, StoreError> {
        let mut meta = self.get(env_id)?;
        meta.ref_count += 1;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        self.put(&meta)?;
        Ok(meta.ref_count)
    }

    pub fn decrement_ref(&self, env_id: &str) -> Result<u32, StoreError> {
        let mut meta = self.get(env_id)?;
        meta.ref_count = meta.ref_count.saturating_sub(1);
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        self.put(&meta)?;
        Ok(meta.ref_count)
    }

    pub fn get_by_name(&self, name: &str) -> Result<EnvMetadata, StoreError> {
        let all = self.list()?;
        all.into_iter()
            .find(|m| m.name.as_deref() == Some(name))
            .ok_or_else(|| StoreError::EnvNotFound(format!("name '{name}'")))
    }

    pub fn update_name(&self, env_id: &str, name: Option<String>) -> Result<(), StoreError> {
        if let Some(ref n) = name {
            validate_env_name(n)?;
            // Check uniqueness
            if let Ok(existing) = self.get_by_name(n) {
                if *existing.env_id != *env_id {
                    return Err(StoreError::NameConflict {
                        name: n.clone(),
                        existing_env_id: existing.env_id[..12.min(existing.env_id.len())]
                            .to_owned(),
                    });
                }
            }
        }
        let mut meta = self.get(env_id)?;
        meta.name = name;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        self.put(&meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata_store() -> (tempfile::TempDir, MetadataStore) {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        (dir, MetadataStore::new(layout))
    }

    fn sample_meta() -> EnvMetadata {
        EnvMetadata {
            env_id: "abc123def456".into(),
            short_id: "abc123def456".into(),
            name: None,
            state: EnvState::Defined,
            manifest_hash: "mhash".into(),
            base_layer: "base1".into(),
            dependency_layers: vec!["dep1".into()],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 1,
            checksum: None,
        }
    }

    #[test]
    fn metadata_roundtrip() {
        let (_dir, store) = test_metadata_store();
        let meta = sample_meta();
        store.put(&meta).unwrap();
        let retrieved = store.get(&meta.env_id).unwrap();
        // put() computes and embeds the checksum, so compare core fields
        assert_eq!(meta.env_id, retrieved.env_id);
        assert_eq!(meta.state, retrieved.state);
        assert_eq!(meta.ref_count, retrieved.ref_count);
        // Verify checksum was written
        assert!(retrieved.checksum.is_some(), "put() must embed a checksum");
    }

    #[test]
    fn state_transition() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        store.update_state("abc123def456", EnvState::Built).unwrap();
        let meta = store.get("abc123def456").unwrap();
        assert_eq!(meta.state, EnvState::Built);
    }

    #[test]
    fn ref_counting() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        let count = store.increment_ref("abc123def456").unwrap();
        assert_eq!(count, 2);
        let count = store.decrement_ref("abc123def456").unwrap();
        assert_eq!(count, 1);
        let count = store.decrement_ref("abc123def456").unwrap();
        assert_eq!(count, 0);
        let count = store.decrement_ref("abc123def456").unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn list_metadata() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn name_roundtrip() {
        let (_dir, store) = test_metadata_store();
        let mut meta = sample_meta();
        meta.name = Some("my-env".to_owned());
        store.put(&meta).unwrap();
        let retrieved = store.get(&meta.env_id).unwrap();
        assert_eq!(retrieved.name, Some("my-env".to_owned()));
    }

    #[test]
    fn get_by_name_works() {
        let (_dir, store) = test_metadata_store();
        let mut meta = sample_meta();
        meta.name = Some("dev-env".to_owned());
        store.put(&meta).unwrap();
        let found = store.get_by_name("dev-env").unwrap();
        assert_eq!(found.env_id, meta.env_id);
    }

    #[test]
    fn get_by_name_not_found() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        assert!(store.get_by_name("nonexistent").is_err());
    }

    #[test]
    fn update_name_validates() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        assert!(store
            .update_name("abc123def456", Some("valid-name".to_owned()))
            .is_ok());
        assert!(store
            .update_name("abc123def456", Some(String::new()))
            .is_err());
        assert!(store
            .update_name("abc123def456", Some("has spaces".to_owned()))
            .is_err());
        assert!(store
            .update_name("abc123def456", Some("a".repeat(65)).clone())
            .is_err());
    }

    #[test]
    fn name_uniqueness_enforced() {
        let (_dir, store) = test_metadata_store();
        let mut m1 = sample_meta();
        m1.name = Some("shared-name".to_owned());
        store.put(&m1).unwrap();

        let mut m2 = sample_meta();
        m2.env_id = "xyz789".into();
        m2.short_id = "xyz789".into();
        store.put(&m2).unwrap();

        assert!(store
            .update_name("xyz789", Some("shared-name".to_owned()))
            .is_err());
    }

    #[test]
    fn backward_compat_no_name_field() {
        let (_dir, store) = test_metadata_store();
        // Simulate old metadata without name field
        let json = r#"{
            "env_id": "old123",
            "short_id": "old123",
            "state": "Built",
            "manifest_hash": "mh",
            "base_layer": "bl",
            "dependency_layers": [],
            "policy_layer": null,
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z",
            "ref_count": 1
        }"#;
        let dir = store.layout.metadata_dir();
        fs::write(dir.join("old123"), json).unwrap();
        let meta = store.get("old123").unwrap();
        assert_eq!(meta.name, None);
    }

    #[test]
    fn exists_returns_true_for_known() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        assert!(store.exists("abc123def456"));
    }

    #[test]
    fn exists_returns_false_for_unknown() {
        let (_dir, store) = test_metadata_store();
        assert!(!store.exists("unknown_id"));
    }

    #[test]
    fn remove_deletes_metadata() {
        let (_dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();
        store.remove("abc123def456").unwrap();
        assert!(!store.exists("abc123def456"));
    }

    #[test]
    fn get_nonexistent_fails() {
        let (_dir, store) = test_metadata_store();
        assert!(store.get("nonexistent").is_err());
    }

    #[test]
    fn validate_env_name_valid_chars() {
        assert!(validate_env_name("my-env_123").is_ok());
        assert!(validate_env_name("a").is_ok());
        assert!(validate_env_name(&"x".repeat(64)).is_ok());
    }

    #[test]
    fn validate_env_name_rejects_empty() {
        assert!(validate_env_name("").is_err());
    }

    #[test]
    fn validate_env_name_rejects_too_long() {
        assert!(validate_env_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn validate_env_name_rejects_special_chars() {
        assert!(validate_env_name("has space").is_err());
        assert!(validate_env_name("has/slash").is_err());
        assert!(validate_env_name("has.dot").is_err());
    }

    #[test]
    fn update_name_to_none_clears_name() {
        let (_dir, store) = test_metadata_store();
        let mut meta = sample_meta();
        meta.name = Some("named".to_owned());
        store.put(&meta).unwrap();
        store.update_name("abc123def456", None).unwrap();
        let retrieved = store.get("abc123def456").unwrap();
        assert_eq!(retrieved.name, None);
    }

    #[test]
    fn list_empty_store() {
        let (_dir, store) = test_metadata_store();
        let list = store.list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn list_multiple_envs() {
        let (_dir, store) = test_metadata_store();
        let mut m1 = sample_meta();
        m1.env_id = "env1".into();
        m1.short_id = "env1".into();
        store.put(&m1).unwrap();

        let mut m2 = sample_meta();
        m2.env_id = "env2".into();
        m2.short_id = "env2".into();
        store.put(&m2).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn list_warns_on_corruption() {
        let (dir, store) = test_metadata_store();
        // Store a valid entry
        store.put(&sample_meta()).unwrap();

        // Write a corrupt metadata file
        let corrupt_path = StoreLayout::new(dir.path())
            .metadata_dir()
            .join("corrupt_env");
        fs::write(&corrupt_path, "NOT VALID JSON").unwrap();

        // list() should return only the valid entry, skipping the corrupt one
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].env_id.to_string(), "abc123def456");
    }

    #[test]
    fn list_with_errors_surfaces_corruption() {
        let (dir, store) = test_metadata_store();
        store.put(&sample_meta()).unwrap();

        // Write a corrupt metadata file
        let corrupt_path = StoreLayout::new(dir.path())
            .metadata_dir()
            .join("corrupt_env");
        fs::write(&corrupt_path, "GARBAGE").unwrap();

        let results = store.list_with_errors().unwrap();
        assert_eq!(results.len(), 2);
        let ok_count = results.iter().filter(|r| r.is_ok()).count();
        let err_count = results.iter().filter(|r| r.is_err()).count();
        assert_eq!(ok_count, 1);
        assert_eq!(err_count, 1);
    }

    #[test]
    fn same_name_same_env_allowed() {
        let (_dir, store) = test_metadata_store();
        let mut meta = sample_meta();
        meta.name = Some("my-name".to_owned());
        store.put(&meta).unwrap();
        // Renaming to the same name on the same env should succeed
        assert!(store
            .update_name("abc123def456", Some("my-name".to_owned()))
            .is_ok());
    }
}
