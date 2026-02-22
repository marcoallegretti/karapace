use crate::StoreError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// Current store format version. Incremented on incompatible layout changes.
pub const STORE_FORMAT_VERSION: u32 = 2;
const VERSION_FILE: &str = "version";

/// Directory layout for the Karapace content-addressable store.
///
/// Manages paths for objects, layers, metadata, environments, and the store
/// version marker. All subdirectories are created lazily on [`initialize`](Self::initialize).
#[derive(Debug, Clone)]
pub struct StoreLayout {
    root: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreVersion {
    format_version: u32,
}

impl StoreLayout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[inline]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[inline]
    pub fn objects_dir(&self) -> PathBuf {
        self.root.join("store").join("objects")
    }

    #[inline]
    pub fn layers_dir(&self) -> PathBuf {
        self.root.join("store").join("layers")
    }

    #[inline]
    pub fn metadata_dir(&self) -> PathBuf {
        self.root.join("store").join("metadata")
    }

    #[inline]
    pub fn env_dir(&self) -> PathBuf {
        self.root.join("env")
    }

    #[inline]
    pub fn env_path(&self, env_id: &str) -> PathBuf {
        self.root.join("env").join(env_id)
    }

    #[inline]
    pub fn overlay_dir(&self, env_id: &str) -> PathBuf {
        self.env_path(env_id).join("overlay")
    }

    /// The writable upper layer of the overlay filesystem.
    /// This is where fuse-overlayfs stores all mutations during container use.
    /// Drift detection, export, and commit must scan this directory.
    #[inline]
    pub fn upper_dir(&self, env_id: &str) -> PathBuf {
        self.env_path(env_id).join("upper")
    }

    /// Temporary staging area for layer packing/unpacking operations.
    #[inline]
    pub fn staging_dir(&self) -> PathBuf {
        self.root.join("store").join("staging")
    }

    #[inline]
    pub fn lock_file(&self) -> PathBuf {
        self.root.join("store").join(".lock")
    }

    pub fn initialize(&self) -> Result<(), StoreError> {
        fs::create_dir_all(self.objects_dir())?;
        fs::create_dir_all(self.layers_dir())?;
        fs::create_dir_all(self.metadata_dir())?;
        fs::create_dir_all(self.env_dir())?;
        fs::create_dir_all(self.staging_dir())?;

        let version_path = self.root.join("store").join(VERSION_FILE);
        if version_path.exists() {
            self.verify_version()?;
        } else {
            let ver = StoreVersion {
                format_version: STORE_FORMAT_VERSION,
            };
            let content = serde_json::to_string_pretty(&ver)?;
            let store_dir = self.root.join("store");
            let mut tmp = NamedTempFile::new_in(&store_dir)?;
            tmp.write_all(content.as_bytes())?;
            tmp.as_file().sync_all()?;
            tmp.persist(&version_path)
                .map_err(|e| StoreError::Io(e.error))?;
            crate::fsync_dir(&store_dir)?;
        }

        Ok(())
    }

    pub fn verify_version(&self) -> Result<(), StoreError> {
        let version_path = self.root.join("store").join(VERSION_FILE);
        let content = fs::read_to_string(&version_path)?;
        let ver: StoreVersion = serde_json::from_str(&content)?;

        if ver.format_version != STORE_FORMAT_VERSION {
            return Err(StoreError::VersionMismatch {
                expected: STORE_FORMAT_VERSION,
                found: ver.format_version,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_paths_are_correct() {
        let layout = StoreLayout::new("/tmp/karapace-test");
        assert_eq!(
            layout.objects_dir(),
            PathBuf::from("/tmp/karapace-test/store/objects")
        );
        assert_eq!(
            layout.layers_dir(),
            PathBuf::from("/tmp/karapace-test/store/layers")
        );
        assert_eq!(
            layout.metadata_dir(),
            PathBuf::from("/tmp/karapace-test/store/metadata")
        );
        assert_eq!(layout.env_dir(), PathBuf::from("/tmp/karapace-test/env"));
        assert_eq!(
            layout.env_path("abc123"),
            PathBuf::from("/tmp/karapace-test/env/abc123")
        );
        assert_eq!(
            layout.overlay_dir("abc123"),
            PathBuf::from("/tmp/karapace-test/env/abc123/overlay")
        );
    }

    #[test]
    fn initialize_creates_directories() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        assert!(layout.objects_dir().is_dir());
        assert!(layout.layers_dir().is_dir());
        assert!(layout.metadata_dir().is_dir());
        assert!(layout.env_dir().is_dir());
    }

    #[test]
    fn initialize_writes_version() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        layout.verify_version().unwrap();
    }

    #[test]
    fn initialize_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        layout.initialize().unwrap();
        layout.verify_version().unwrap();
    }
}
