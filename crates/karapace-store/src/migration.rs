//! Store format migration engine.
//!
//! Provides automatic migration from older store format versions to the current
//! [`STORE_FORMAT_VERSION`]. Creates a backup of the version file before any
//! modification and writes all changes atomically.

use crate::layout::STORE_FORMAT_VERSION;
use crate::{fsync_dir, StoreError};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tracing::{info, warn};

/// Result of a successful migration.
#[derive(Debug)]
pub struct MigrationResult {
    pub from_version: u32,
    pub to_version: u32,
    pub environments_migrated: usize,
    pub backup_path: PathBuf,
}

/// Migrate a store from its current format version to [`STORE_FORMAT_VERSION`].
///
/// - Returns `Ok(None)` if the store is already at the current version.
/// - Returns `Err(VersionMismatch)` if the store is from a *newer* version.
/// - Creates a backup of the version file at `store/version.backup.{timestamp}`.
/// - Rewrites metadata files atomically to add any missing v2 fields.
/// - Writes the new version file atomically as the final step.
pub fn migrate_store(root: &Path) -> Result<Option<MigrationResult>, StoreError> {
    let store_dir = root.join("store");
    let version_path = store_dir.join("version");

    if !version_path.exists() {
        return Err(StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no version file at {}", version_path.display()),
        )));
    }

    let content = fs::read_to_string(&version_path)?;
    let ver: serde_json::Value =
        serde_json::from_str(&content).map_err(StoreError::Serialization)?;
    let found = ver
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;

    if found == STORE_FORMAT_VERSION {
        return Ok(None);
    }

    if found > STORE_FORMAT_VERSION {
        return Err(StoreError::VersionMismatch {
            expected: STORE_FORMAT_VERSION,
            found,
        });
    }

    // --- Backup ---
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_path = store_dir.join(format!("version.backup.{timestamp}"));
    fs::copy(&version_path, &backup_path)?;
    info!("backed up store version file to {}", backup_path.display());

    // --- Migrate metadata files ---
    let metadata_dir = store_dir.join("metadata");
    let mut envs_migrated = 0;
    if metadata_dir.is_dir() {
        for entry in fs::read_dir(&metadata_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            match migrate_metadata_file(&path) {
                Ok(true) => envs_migrated += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!("skipping metadata file {}: {e}", path.display());
                }
            }
        }
    }

    // --- Write new version file atomically (LAST step) ---
    let new_ver = serde_json::json!({ "format_version": STORE_FORMAT_VERSION });
    let new_content = serde_json::to_string_pretty(&new_ver).map_err(StoreError::Serialization)?;
    let mut tmp = NamedTempFile::new_in(&store_dir)?;
    tmp.write_all(new_content.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(&version_path)
        .map_err(|e| StoreError::Io(e.error))?;
    fsync_dir(&store_dir)?;

    info!("migrated store from v{found} to v{STORE_FORMAT_VERSION} ({envs_migrated} environments)");

    Ok(Some(MigrationResult {
        from_version: found,
        to_version: STORE_FORMAT_VERSION,
        environments_migrated: envs_migrated,
        backup_path,
    }))
}

/// Migrate a single metadata JSON file to v2 format.
///
/// v2 added: `name` (Option<String>), `checksum` (Option<String>), `policy_layer` (Option).
/// If any of these are missing, they are added with default values.
///
/// Returns `Ok(true)` if the file was rewritten, `Ok(false)` if no changes needed.
fn migrate_metadata_file(path: &Path) -> Result<bool, StoreError> {
    let content = fs::read_to_string(path)?;
    let mut val: serde_json::Value =
        serde_json::from_str(&content).map_err(StoreError::Serialization)?;

    let obj = val.as_object_mut().ok_or_else(|| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "metadata is not a JSON object",
        ))
    })?;

    let mut changed = false;

    // v2 fields with defaults
    if !obj.contains_key("name") {
        obj.insert("name".to_owned(), serde_json::Value::Null);
        changed = true;
    }
    if !obj.contains_key("checksum") {
        obj.insert("checksum".to_owned(), serde_json::Value::Null);
        changed = true;
    }
    if !obj.contains_key("policy_layer") {
        obj.insert("policy_layer".to_owned(), serde_json::Value::Null);
        changed = true;
    }

    if !changed {
        return Ok(false);
    }

    // Rewrite atomically
    let new_content = serde_json::to_string_pretty(&val).map_err(StoreError::Serialization)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(new_content.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| StoreError::Io(e.error))?;
    fsync_dir(dir)?;

    Ok(true)
}
