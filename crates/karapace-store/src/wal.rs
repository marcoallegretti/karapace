use crate::layout::StoreLayout;
use crate::metadata::{EnvMetadata, EnvState, MetadataStore};
use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tempfile::NamedTempFile;
use tracing::{debug, info, warn};

fn parse_env_state(s: &str) -> Option<EnvState> {
    match s {
        "Defined" | "defined" => Some(EnvState::Defined),
        "Built" | "built" => Some(EnvState::Built),
        "Running" | "running" => Some(EnvState::Running),
        "Frozen" | "frozen" => Some(EnvState::Frozen),
        "Archived" | "archived" => Some(EnvState::Archived),
        _ => None,
    }
}

/// A single rollback step that can undo part of an operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RollbackStep {
    /// Remove a directory tree (e.g. orphaned env_dir).
    RemoveDir(PathBuf),
    /// Remove a single file (e.g. metadata, layer manifest).
    RemoveFile(PathBuf),
    /// Reset an environment's metadata state (e.g. Running → Built after crash).
    ResetState {
        env_id: String,
        target_state: String,
    },
}

/// The type of mutating operation being tracked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalOpKind {
    Build,
    Rebuild,
    Commit,
    Restore,
    Destroy,
    Gc,
    Enter,
    Exec,
}

impl std::fmt::Display for WalOpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalOpKind::Build => write!(f, "build"),
            WalOpKind::Rebuild => write!(f, "rebuild"),
            WalOpKind::Commit => write!(f, "commit"),
            WalOpKind::Restore => write!(f, "restore"),
            WalOpKind::Destroy => write!(f, "destroy"),
            WalOpKind::Gc => write!(f, "gc"),
            WalOpKind::Enter => write!(f, "enter"),
            WalOpKind::Exec => write!(f, "exec"),
        }
    }
}

/// A WAL entry representing an in-flight operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    pub op_id: String,
    pub kind: WalOpKind,
    pub env_id: String,
    pub timestamp: String,
    pub rollback_steps: Vec<RollbackStep>,
}

/// Write-ahead log for crash recovery.
///
/// Mutating engine methods create a WAL entry before starting work,
/// append rollback steps as side effects occur, and remove the entry
/// on successful completion. On startup, incomplete entries are rolled back.
pub struct WriteAheadLog {
    wal_dir: PathBuf,
}

impl WriteAheadLog {
    pub fn new(layout: &StoreLayout) -> Self {
        let wal_dir = layout.root().join("store").join("wal");
        Self { wal_dir }
    }

    /// Ensure the WAL directory exists.
    pub fn initialize(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.wal_dir)?;
        Ok(())
    }

    /// Begin a new WAL entry for an operation. Returns the op_id.
    pub fn begin(&self, kind: WalOpKind, env_id: &str) -> Result<String, StoreError> {
        let op_id = format!(
            "{}-{}",
            chrono::Utc::now().format("%Y%m%d%H%M%S%3f"),
            &blake3::hash(env_id.as_bytes()).to_hex()[..8]
        );
        let entry = WalEntry {
            op_id: op_id.clone(),
            kind,
            env_id: env_id.to_owned(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            rollback_steps: Vec::new(),
        };
        self.write_entry(&entry)?;
        debug!("WAL begin: {} for {env_id} (op_id={op_id})", entry.kind);
        Ok(op_id)
    }

    /// Append a rollback step to an existing WAL entry.
    pub fn add_rollback_step(&self, op_id: &str, step: RollbackStep) -> Result<(), StoreError> {
        let mut entry = self.read_entry(op_id)?;
        entry.rollback_steps.push(step);
        self.write_entry(&entry)?;
        Ok(())
    }

    /// Commit (remove) a WAL entry after successful completion.
    pub fn commit(&self, op_id: &str) -> Result<(), StoreError> {
        let path = self.entry_path(op_id);
        if path.exists() {
            fs::remove_file(&path)?;
            debug!("WAL commit: {op_id}");
        }
        Ok(())
    }

    /// List all incomplete WAL entries.
    pub fn list_incomplete(&self) -> Result<Vec<WalEntry>, StoreError> {
        if !self.wal_dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for dir_entry in fs::read_dir(&self.wal_dir)? {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                match fs::read_to_string(&path) {
                    Ok(content) => match serde_json::from_str::<WalEntry>(&content) {
                        Ok(entry) => entries.push(entry),
                        Err(e) => {
                            warn!("corrupt WAL entry {}: {e}", path.display());
                            // Remove corrupt entries
                            let _ = fs::remove_file(&path);
                        }
                    },
                    Err(e) => {
                        warn!("unreadable WAL entry {}: {e}", path.display());
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
        entries.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        Ok(entries)
    }

    /// Roll back all incomplete WAL entries.
    /// Returns the number of entries rolled back.
    pub fn recover(&self) -> Result<usize, StoreError> {
        let entries = self.list_incomplete()?;
        let count = entries.len();
        for entry in &entries {
            info!(
                "WAL recovery: rolling back {} on {} (op_id={})",
                entry.kind, entry.env_id, entry.op_id
            );
            self.rollback_entry(entry);
            // Remove the WAL entry after rollback
            let _ = fs::remove_file(self.entry_path(&entry.op_id));
        }
        if count > 0 {
            info!("WAL recovery complete: {count} entries rolled back");
        }
        Ok(count)
    }

    fn rollback_entry(&self, entry: &WalEntry) {
        // Execute rollback steps in reverse order
        for step in entry.rollback_steps.iter().rev() {
            match step {
                RollbackStep::RemoveDir(path) => {
                    if path.exists() {
                        if let Err(e) = fs::remove_dir_all(path) {
                            warn!("WAL rollback: failed to remove dir {}: {e}", path.display());
                        } else {
                            debug!("WAL rollback: removed dir {}", path.display());
                        }
                    }
                }
                RollbackStep::RemoveFile(path) => {
                    if path.exists() {
                        if let Err(e) = fs::remove_file(path) {
                            warn!(
                                "WAL rollback: failed to remove file {}: {e}",
                                path.display()
                            );
                        } else {
                            debug!("WAL rollback: removed file {}", path.display());
                        }
                    }
                }
                RollbackStep::ResetState {
                    env_id,
                    target_state,
                } => {
                    let Some(new_state) = parse_env_state(target_state) else {
                        warn!("WAL rollback: unknown target state '{target_state}' for {env_id}");
                        continue;
                    };

                    // wal_dir = <root>/store/wal
                    let Some(store_dir) = self.wal_dir.parent() else {
                        continue;
                    };
                    let Some(root_dir) = store_dir.parent() else {
                        continue;
                    };

                    let meta_path = store_dir.join("metadata").join(env_id);
                    if !meta_path.exists() {
                        continue;
                    }

                    let content = match fs::read_to_string(&meta_path) {
                        Ok(c) => c,
                        Err(e) => {
                            warn!("WAL rollback: failed to read metadata for {env_id}: {e}");
                            continue;
                        }
                    };

                    let mut meta: EnvMetadata = match serde_json::from_str(&content) {
                        Ok(m) => m,
                        Err(e) => {
                            warn!("WAL rollback: failed to parse metadata for {env_id}: {e}");
                            continue;
                        }
                    };

                    meta.state = new_state;
                    meta.updated_at = chrono::Utc::now().to_rfc3339();
                    meta.checksum = None;

                    let layout = StoreLayout::new(root_dir);
                    let meta_store = MetadataStore::new(layout);
                    if let Err(e) = meta_store.put(&meta) {
                        warn!("WAL rollback: failed to persist metadata for {env_id}: {e}");
                    } else {
                        debug!("WAL rollback: reset {env_id} state to {target_state}");
                    }
                }
            }
        }
    }

    fn entry_path(&self, op_id: &str) -> PathBuf {
        self.wal_dir.join(format!("{op_id}.json"))
    }

    fn write_entry(&self, entry: &WalEntry) -> Result<(), StoreError> {
        fs::create_dir_all(&self.wal_dir)?;
        let content = serde_json::to_string_pretty(entry)?;
        let mut tmp = NamedTempFile::new_in(&self.wal_dir)?;
        tmp.write_all(content.as_bytes())?;
        tmp.as_file().sync_all()?;
        let dest = self.entry_path(&entry.op_id);
        tmp.persist(&dest).map_err(|e| StoreError::Io(e.error))?;
        crate::fsync_dir(&self.wal_dir)?;
        Ok(())
    }

    fn read_entry(&self, op_id: &str) -> Result<WalEntry, StoreError> {
        let path = self.entry_path(op_id);
        let content = fs::read_to_string(&path)?;
        let entry: WalEntry = serde_json::from_str(&content)?;
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, WriteAheadLog) {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        let wal = WriteAheadLog::new(&layout);
        wal.initialize().unwrap();
        (dir, wal)
    }

    #[test]
    fn begin_creates_entry() {
        let (_dir, wal) = setup();
        let op_id = wal.begin(WalOpKind::Build, "test-env-123").unwrap();
        assert!(!op_id.is_empty());
        let entries = wal.list_incomplete().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].env_id, "test-env-123");
    }

    #[test]
    fn commit_removes_entry() {
        let (_dir, wal) = setup();
        let op_id = wal.begin(WalOpKind::Build, "test-env").unwrap();
        assert_eq!(wal.list_incomplete().unwrap().len(), 1);
        wal.commit(&op_id).unwrap();
        assert!(wal.list_incomplete().unwrap().is_empty());
    }

    #[test]
    fn successful_ops_leave_zero_entries() {
        let (_dir, wal) = setup();
        let op1 = wal.begin(WalOpKind::Build, "env1").unwrap();
        let op2 = wal.begin(WalOpKind::Commit, "env2").unwrap();
        wal.commit(&op1).unwrap();
        wal.commit(&op2).unwrap();
        assert!(wal.list_incomplete().unwrap().is_empty());
    }

    #[test]
    fn add_rollback_step_persists() {
        let (_dir, wal) = setup();
        let op_id = wal.begin(WalOpKind::Build, "env1").unwrap();
        wal.add_rollback_step(&op_id, RollbackStep::RemoveDir(PathBuf::from("/tmp/fake")))
            .unwrap();
        let entries = wal.list_incomplete().unwrap();
        assert_eq!(entries[0].rollback_steps.len(), 1);
    }

    #[test]
    fn recover_rolls_back_incomplete() {
        let (dir, wal) = setup();
        let op_id = wal.begin(WalOpKind::Build, "env1").unwrap();

        // Create a directory that should be rolled back
        let orphan_dir = dir.path().join("orphan_env");
        fs::create_dir_all(&orphan_dir).unwrap();
        fs::write(orphan_dir.join("file.txt"), "data").unwrap();
        assert!(orphan_dir.exists());

        wal.add_rollback_step(&op_id, RollbackStep::RemoveDir(orphan_dir.clone()))
            .unwrap();

        // Simulate crash: don't call commit. Recovery should clean up.
        let count = wal.recover().unwrap();
        assert_eq!(count, 1);
        assert!(
            !orphan_dir.exists(),
            "orphan dir must be removed by recovery"
        );
        assert!(wal.list_incomplete().unwrap().is_empty());
    }

    #[test]
    fn recover_removes_file_rollback_step() {
        let (dir, wal) = setup();
        let op_id = wal.begin(WalOpKind::Commit, "env1").unwrap();

        let orphan_file = dir.path().join("orphan.json");
        fs::write(&orphan_file, "{}").unwrap();

        wal.add_rollback_step(&op_id, RollbackStep::RemoveFile(orphan_file.clone()))
            .unwrap();

        let count = wal.recover().unwrap();
        assert_eq!(count, 1);
        assert!(!orphan_file.exists());
    }

    #[test]
    fn recover_with_no_entries_is_noop() {
        let (_dir, wal) = setup();
        let count = wal.recover().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn op_kind_display() {
        assert_eq!(WalOpKind::Build.to_string(), "build");
        assert_eq!(WalOpKind::Rebuild.to_string(), "rebuild");
        assert_eq!(WalOpKind::Commit.to_string(), "commit");
        assert_eq!(WalOpKind::Restore.to_string(), "restore");
        assert_eq!(WalOpKind::Destroy.to_string(), "destroy");
        assert_eq!(WalOpKind::Enter.to_string(), "enter");
        assert_eq!(WalOpKind::Exec.to_string(), "exec");
    }

    #[test]
    fn recover_reset_state_rollback() {
        let (dir, wal) = setup();

        // Write a fake metadata file in the expected location (store/metadata/env1)
        let metadata_dir = dir.path().join("store").join("metadata");
        let meta_json = r#"{
            "env_id": "env1",
            "short_id": "env1",
            "state": "Running",
            "manifest_hash": "mh",
            "base_layer": "bl",
            "dependency_layers": [],
            "policy_layer": null,
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z",
            "ref_count": 1
        }"#;
        fs::write(metadata_dir.join("env1"), meta_json).unwrap();

        // Create a WAL entry with a ResetState rollback step
        let op_id = wal.begin(WalOpKind::Enter, "env1").unwrap();
        wal.add_rollback_step(
            &op_id,
            RollbackStep::ResetState {
                env_id: "env1".to_owned(),
                target_state: "Built".to_owned(),
            },
        )
        .unwrap();

        // Simulate crash: don't commit. Recovery should reset state.
        let count = wal.recover().unwrap();
        assert_eq!(count, 1);

        // Verify state was reset to Built
        let content = fs::read_to_string(metadata_dir.join("env1")).unwrap();
        let meta: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(meta["state"], "Built");
    }

    #[test]
    fn recover_corrupt_wal_entry_is_removed() {
        let (dir, wal) = setup();

        // Write a corrupt WAL entry directly
        let wal_dir = dir.path().join("store").join("wal");
        fs::write(wal_dir.join("corrupt-op.json"), "THIS IS NOT JSON{{{").unwrap();

        // Also write a valid entry
        let op_id = wal.begin(WalOpKind::Build, "env1").unwrap();
        let orphan = dir.path().join("orphan_from_valid");
        fs::create_dir_all(&orphan).unwrap();
        wal.add_rollback_step(&op_id, RollbackStep::RemoveDir(orphan.clone()))
            .unwrap();

        // Recovery must handle corrupt entry (remove it) and still roll back valid one
        let count = wal.recover().unwrap();
        assert_eq!(
            count, 1,
            "only the valid entry should be counted as rolled back"
        );
        assert!(!orphan.exists(), "valid rollback must still execute");

        // Corrupt entry file must be gone
        assert!(
            !wal_dir.join("corrupt-op.json").exists(),
            "corrupt WAL entry must be removed during recovery"
        );

        // No WAL entries remain
        assert!(wal.list_incomplete().unwrap().is_empty());
    }

    #[test]
    fn recover_no_duplicate_objects_after_partial_build() {
        let (dir, wal) = setup();
        let layout = StoreLayout::new(dir.path());

        // Simulate a partial build: object was written, then crash
        let obj_store = crate::ObjectStore::new(layout.clone());
        let hash1 = obj_store.put(b"real object data").unwrap();

        // WAL entry says to remove the object (rollback of partial build)
        let obj_path = layout.objects_dir().join(&hash1);
        let op_id = wal.begin(WalOpKind::Build, "env1").unwrap();
        wal.add_rollback_step(&op_id, RollbackStep::RemoveFile(obj_path.clone()))
            .unwrap();

        // Simulate crash: don't commit
        let count = wal.recover().unwrap();
        assert_eq!(count, 1);

        // Object must be gone (rolled back)
        assert!(
            !obj_path.exists(),
            "partial object must be removed by recovery"
        );

        // No duplicate: writing the same data again must succeed cleanly
        let hash2 = obj_store.put(b"real object data").unwrap();
        assert_eq!(hash1, hash2, "same data must produce same hash");
        assert!(
            layout.objects_dir().join(&hash2).exists(),
            "re-written object must exist"
        );
    }

    #[test]
    fn recover_version_file_unchanged() {
        let (dir, wal) = setup();
        let layout = StoreLayout::new(dir.path());

        // Read version before
        let version_before = fs::read_to_string(dir.path().join("store").join("version")).unwrap();

        // Create WAL entries and recover
        let op1 = wal.begin(WalOpKind::Build, "env1").unwrap();
        let orphan = dir.path().join("test_orphan");
        fs::create_dir_all(&orphan).unwrap();
        wal.add_rollback_step(&op1, RollbackStep::RemoveDir(orphan.clone()))
            .unwrap();

        let count = wal.recover().unwrap();
        assert_eq!(count, 1);

        // Version file must be identical
        let version_after = fs::read_to_string(dir.path().join("store").join("version")).unwrap();
        assert_eq!(
            version_before, version_after,
            "version file must not change during WAL recovery"
        );

        // Store integrity must pass
        let report = crate::verify_store_integrity(&layout).unwrap();
        assert!(
            report.failed.is_empty(),
            "store integrity must pass after WAL recovery: {:?}",
            report.failed
        );
    }
}
