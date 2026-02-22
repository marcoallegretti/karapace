//! Content-addressable object store, layer management, and environment metadata for Karapace.
//!
//! This crate provides the storage layer: a content-addressable `ObjectStore` backed
//! by blake3 hashing with atomic writes, `LayerStore` for overlay filesystem layer
//! manifests, `MetadataStore` for environment state tracking, `StoreLayout` for
//! directory structure management, and `GarbageCollector` for orphan cleanup.

pub mod gc;
pub mod integrity;
pub mod layers;
pub mod layout;
pub mod metadata;
pub mod migration;
pub mod objects;
pub mod wal;

pub use gc::{GarbageCollector, GcReport};
pub use integrity::{verify_store_integrity, IntegrityFailure, IntegrityReport};
pub use layers::{pack_layer, unpack_layer, LayerKind, LayerManifest, LayerStore};
pub use layout::{StoreLayout, STORE_FORMAT_VERSION};
pub use metadata::{validate_env_name, EnvMetadata, EnvState, MetadataStore};
pub use migration::{migrate_store, MigrationResult};
pub use objects::ObjectStore;
pub use wal::{RollbackStep, WalOpKind, WriteAheadLog};

use std::path::Path;
use thiserror::Error;

/// Fsync a directory to ensure that a preceding `rename()` is durable.
///
/// On Linux with ext4 `data=ordered` (the default), renames are usually
/// durable without an explicit dir fsync, but POSIX does not guarantee this.
/// Calling `fsync()` on the parent directory makes the rename durable on
/// all filesystems and mount configurations.
pub(crate) fn fsync_dir(dir: &Path) -> Result<(), std::io::Error> {
    let f = std::fs::File::open(dir)?;
    f.sync_all()
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("store I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("integrity check failed for object '{hash}': expected {expected}, got {actual}")]
    IntegrityFailure {
        hash: String,
        expected: String,
        actual: String,
    },
    #[error("object not found: {0}")]
    ObjectNotFound(String),
    #[error("layer not found: {0}")]
    LayerNotFound(String),
    #[error("environment not found: {0}")]
    EnvNotFound(String),
    #[error("lock acquisition failed: {0}")]
    LockFailed(String),
    #[error("store format version mismatch: expected {expected}, found {found}")]
    VersionMismatch { expected: u32, found: u32 },
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid environment name: {0}")]
    InvalidName(String),
    #[error("name '{name}' is already used by environment {existing_env_id}")]
    NameConflict {
        name: String,
        existing_env_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_error_display_invalid_name() {
        let e = StoreError::InvalidName("bad".to_owned());
        assert!(e.to_string().contains("invalid environment name"));
    }

    #[test]
    fn store_error_display_name_conflict() {
        let e = StoreError::NameConflict {
            name: "dup".to_owned(),
            existing_env_id: "abc123".to_owned(),
        };
        let msg = e.to_string();
        assert!(msg.contains("dup"));
        assert!(msg.contains("abc123"));
    }

    #[test]
    fn store_error_display_object_not_found() {
        let e = StoreError::ObjectNotFound("hash123".to_owned());
        assert!(e.to_string().contains("hash123"));
    }

    #[test]
    fn store_error_display_layer_not_found() {
        let e = StoreError::LayerNotFound("lhash".to_owned());
        assert!(e.to_string().contains("lhash"));
    }

    #[test]
    fn store_error_display_env_not_found() {
        let e = StoreError::EnvNotFound("envid".to_owned());
        assert!(e.to_string().contains("envid"));
    }

    #[test]
    fn store_error_display_lock_failed() {
        let e = StoreError::LockFailed("reason".to_owned());
        assert!(e.to_string().contains("reason"));
    }

    #[test]
    fn store_error_display_version_mismatch() {
        let e = StoreError::VersionMismatch {
            expected: 2,
            found: 1,
        };
        let msg = e.to_string();
        assert!(msg.contains('2'));
        assert!(msg.contains('1'));
    }

    #[test]
    fn store_error_display_integrity_failure() {
        let e = StoreError::IntegrityFailure {
            hash: "h".to_owned(),
            expected: "exp".to_owned(),
            actual: "act".to_owned(),
        };
        let msg = e.to_string();
        assert!(msg.contains("exp"));
        assert!(msg.contains("act"));
    }
}
