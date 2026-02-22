use crate::layout::StoreLayout;
use crate::{fsync_dir, StoreError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::NamedTempFile;
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LayerKind {
    Base,
    Dependency,
    Policy,
    Snapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerManifest {
    pub hash: String,
    pub kind: LayerKind,
    pub parent: Option<String>,
    pub object_refs: Vec<String>,
    pub read_only: bool,
    /// blake3 hash of the tar archive containing this layer's filesystem content.
    /// Empty for legacy (v1) synthetic layers.
    #[serde(default)]
    pub tar_hash: String,
}

pub struct LayerStore {
    layout: StoreLayout,
}

impl LayerStore {
    pub fn new(layout: StoreLayout) -> Self {
        Self { layout }
    }

    /// Compute the content hash that `put()` would use for this manifest,
    /// without writing anything to disk.
    pub fn compute_hash(manifest: &LayerManifest) -> Result<String, StoreError> {
        let content = serde_json::to_string_pretty(manifest)?;
        Ok(blake3::hash(content.as_bytes()).to_hex().to_string())
    }

    /// Store a layer manifest. Returns the content hash (blake3 of serialized JSON),
    /// which is used as the filename. Idempotent — existing layers are skipped.
    pub fn put(&self, manifest: &LayerManifest) -> Result<String, StoreError> {
        let content = serde_json::to_string_pretty(manifest)?;
        let hash = blake3::hash(content.as_bytes()).to_hex().to_string();
        let dest = self.layout.layers_dir().join(&hash);

        if dest.exists() {
            return Ok(hash);
        }

        let dir = self.layout.layers_dir();
        let mut tmp = NamedTempFile::new_in(&dir)?;
        tmp.write_all(content.as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(&dest).map_err(|e| StoreError::Io(e.error))?;
        fsync_dir(&dir)?;

        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> Result<LayerManifest, StoreError> {
        let path = self.layout.layers_dir().join(hash);
        if !path.exists() {
            return Err(StoreError::LayerNotFound(hash.to_owned()));
        }
        let content = fs::read_to_string(&path)?;

        // Verify integrity: content hash must match filename
        let actual = blake3::hash(content.as_bytes());
        let actual_hex = actual.to_hex();
        if actual_hex.as_str() != hash {
            return Err(StoreError::IntegrityFailure {
                hash: hash.to_owned(),
                expected: hash.to_owned(),
                actual: actual_hex.to_string(),
            });
        }

        let manifest: LayerManifest = serde_json::from_str(&content)?;
        Ok(manifest)
    }

    pub fn exists(&self, hash: &str) -> bool {
        self.layout.layers_dir().join(hash).exists()
    }

    pub fn remove(&self, hash: &str) -> Result<(), StoreError> {
        let path = self.layout.layers_dir().join(hash);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<String>, StoreError> {
        let dir = self.layout.layers_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut hashes = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if let Some(name) = entry.file_name().to_str() {
                if !name.starts_with('.') {
                    hashes.push(name.to_owned());
                }
            }
        }
        hashes.sort();
        Ok(hashes)
    }
}

/// Create a deterministic tar archive from a directory.
///
/// Phase 1 supports regular files, directories, and symlinks.
/// Device nodes, sockets, FIFOs, and extended attributes are skipped with warnings.
///
/// Determinism guarantees:
/// - Entries sorted lexicographically by relative path
/// - All timestamps set to 0 (Unix epoch)
/// - All ownership set to 0:0 (root:root)
/// - Permissions preserved as-is from source
pub fn pack_layer(source_dir: &Path) -> Result<Vec<u8>, StoreError> {
    let mut entries = collect_entries(source_dir, source_dir)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut ar = tar::Builder::new(Vec::new());
    ar.follow_symlinks(false);

    for (rel_path, full_path) in &entries {
        let ft = match full_path.symlink_metadata() {
            Ok(m) => m.file_type(),
            Err(e) => {
                warn!("skipping {}: metadata error: {e}", rel_path);
                continue;
            }
        };

        if ft.is_file() {
            append_file(&mut ar, rel_path, full_path)?;
        } else if ft.is_dir() {
            append_dir(&mut ar, rel_path, full_path)?;
        } else if ft.is_symlink() {
            append_symlink(&mut ar, rel_path, full_path)?;
        } else {
            warn!("skipping unsupported file type: {rel_path}");
        }
    }

    let data = ar.into_inner()?;
    Ok(data)
}

/// Extract a tar archive to a target directory.
pub fn unpack_layer(tar_data: &[u8], target_dir: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(target_dir)?;
    let mut ar = tar::Archive::new(tar_data);
    ar.set_preserve_permissions(true);
    ar.set_preserve_mtime(false);
    ar.set_unpack_xattrs(false);
    ar.unpack(target_dir)?;
    Ok(())
}

/// Recursively collect (relative_path, full_path) pairs from a directory tree.
fn collect_entries(
    root: &Path,
    current: &Path,
) -> Result<Vec<(String, std::path::PathBuf)>, StoreError> {
    let mut result = Vec::new();
    if !current.exists() {
        return Ok(result);
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let full = entry.path();
        let rel = full
            .strip_prefix(root)
            .map_err(|e| StoreError::Io(std::io::Error::other(format!("path strip: {e}"))))?
            .to_string_lossy()
            .to_string();

        let meta = full.symlink_metadata()?;
        if meta.is_dir() {
            result.push((rel.clone(), full.clone()));
            result.extend(collect_entries(root, &full)?);
        } else {
            result.push((rel, full));
        }
    }
    Ok(result)
}

fn make_header(full_path: &Path, entry_type: tar::EntryType) -> Result<tar::Header, StoreError> {
    let meta = full_path.symlink_metadata()?;
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mode(meta.permissions().mode());
    Ok(header)
}

fn append_file(
    ar: &mut tar::Builder<Vec<u8>>,
    rel_path: &str,
    full_path: &Path,
) -> Result<(), StoreError> {
    let data = fs::read(full_path)?;
    let mut header = make_header(full_path, tar::EntryType::Regular)?;
    header.set_size(data.len() as u64);
    header.set_cksum();
    ar.append_data(&mut header, rel_path, data.as_slice())?;
    Ok(())
}

fn append_dir(
    ar: &mut tar::Builder<Vec<u8>>,
    rel_path: &str,
    full_path: &Path,
) -> Result<(), StoreError> {
    let mut header = make_header(full_path, tar::EntryType::Directory)?;
    header.set_size(0);
    header.set_cksum();
    let path = if rel_path.ends_with('/') {
        rel_path.to_owned()
    } else {
        format!("{rel_path}/")
    };
    ar.append_data(&mut header, &path, &[] as &[u8])?;
    Ok(())
}

fn append_symlink(
    ar: &mut tar::Builder<Vec<u8>>,
    rel_path: &str,
    full_path: &Path,
) -> Result<(), StoreError> {
    let target = fs::read_link(full_path)?;
    let mut header = make_header(full_path, tar::EntryType::Symlink)?;
    header.set_size(0);
    header.set_cksum();
    ar.append_link(&mut header, rel_path, &target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_layer_store() -> (tempfile::TempDir, LayerStore) {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        (dir, LayerStore::new(layout))
    }

    fn sample_layer() -> LayerManifest {
        LayerManifest {
            hash: "abc123def456".to_owned(),
            kind: LayerKind::Base,
            parent: None,
            object_refs: vec!["obj1".to_owned(), "obj2".to_owned()],
            read_only: true,
            tar_hash: String::new(),
        }
    }

    #[test]
    fn put_and_get_roundtrip() {
        let (_dir, store) = test_layer_store();
        let layer = sample_layer();
        let content_hash = store.put(&layer).unwrap();
        let retrieved = store.get(&content_hash).unwrap();
        assert_eq!(layer, retrieved);
    }

    #[test]
    fn put_is_idempotent() {
        let (_dir, store) = test_layer_store();
        let layer = sample_layer();
        store.put(&layer).unwrap();
        store.put(&layer).unwrap();
    }

    #[test]
    fn get_nonexistent_fails() {
        let (_dir, store) = test_layer_store();
        assert!(store.get("nonexistent").is_err());
    }

    #[test]
    fn list_layers() {
        let (_dir, store) = test_layer_store();
        let content_hash = store.put(&sample_layer()).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], content_hash);
    }

    #[test]
    fn deserialize_without_tar_hash_defaults_empty() {
        let json = r#"{
            "hash": "h1",
            "kind": "Base",
            "parent": null,
            "object_refs": [],
            "read_only": true
        }"#;
        let m: LayerManifest = serde_json::from_str(json).unwrap();
        assert!(m.tar_hash.is_empty());
    }

    // --- pack/unpack tests ---

    fn create_fixture_dir(dir: &Path) {
        // Regular files
        fs::write(dir.join("hello.txt"), "hello world").unwrap();
        fs::write(dir.join("binary.bin"), [0u8, 1, 2, 255]).unwrap();

        // Subdirectory with files
        fs::create_dir_all(dir.join("subdir")).unwrap();
        fs::write(dir.join("subdir").join("nested.txt"), "nested content").unwrap();

        // Empty directory
        fs::create_dir_all(dir.join("empty_dir")).unwrap();

        // Symlink
        std::os::unix::fs::symlink("hello.txt", dir.join("link_to_hello")).unwrap();
    }

    #[test]
    fn pack_unpack_roundtrip() {
        let src = tempfile::tempdir().unwrap();
        create_fixture_dir(src.path());

        let tar_data = pack_layer(src.path()).unwrap();
        assert!(!tar_data.is_empty());

        let dst = tempfile::tempdir().unwrap();
        unpack_layer(&tar_data, dst.path()).unwrap();

        // Verify regular files
        assert_eq!(
            fs::read_to_string(dst.path().join("hello.txt")).unwrap(),
            "hello world"
        );
        assert_eq!(
            fs::read(dst.path().join("binary.bin")).unwrap(),
            &[0u8, 1, 2, 255]
        );

        // Verify nested file
        assert_eq!(
            fs::read_to_string(dst.path().join("subdir").join("nested.txt")).unwrap(),
            "nested content"
        );

        // Verify empty directory
        assert!(dst.path().join("empty_dir").is_dir());

        // Verify symlink
        let link = dst.path().join("link_to_hello");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap().to_string_lossy(), "hello.txt");
    }

    #[test]
    fn pack_is_deterministic() {
        let src = tempfile::tempdir().unwrap();
        create_fixture_dir(src.path());

        let tar1 = pack_layer(src.path()).unwrap();
        let tar2 = pack_layer(src.path()).unwrap();
        assert_eq!(tar1, tar2, "pack_layer must be deterministic");
    }

    #[test]
    fn pack_deterministic_hash() {
        let src = tempfile::tempdir().unwrap();
        create_fixture_dir(src.path());

        let tar1 = pack_layer(src.path()).unwrap();
        let tar2 = pack_layer(src.path()).unwrap();
        let h1 = blake3::hash(&tar1).to_hex().to_string();
        let h2 = blake3::hash(&tar2).to_hex().to_string();
        assert_eq!(h1, h2);
    }

    #[test]
    fn pack_empty_dir() {
        let src = tempfile::tempdir().unwrap();
        let tar_data = pack_layer(src.path()).unwrap();
        // Empty directory produces a valid (possibly empty) tar
        let dst = tempfile::tempdir().unwrap();
        unpack_layer(&tar_data, dst.path()).unwrap();
    }

    #[test]
    fn pack_different_content_different_hash() {
        let src1 = tempfile::tempdir().unwrap();
        fs::write(src1.path().join("a.txt"), "aaa").unwrap();
        let tar1 = pack_layer(src1.path()).unwrap();

        let src2 = tempfile::tempdir().unwrap();
        fs::write(src2.path().join("a.txt"), "bbb").unwrap();
        let tar2 = pack_layer(src2.path()).unwrap();

        let h1 = blake3::hash(&tar1).to_hex().to_string();
        let h2 = blake3::hash(&tar2).to_hex().to_string();
        assert_ne!(h1, h2);
    }

    #[test]
    fn unpack_nonexistent_target_created() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("f.txt"), "data").unwrap();
        let tar_data = pack_layer(src.path()).unwrap();

        let base = tempfile::tempdir().unwrap();
        let target = base.path().join("new_subdir");
        assert!(!target.exists());
        unpack_layer(&tar_data, &target).unwrap();
        assert!(target.join("f.txt").exists());
    }

    // --- A2: Layer Integrity Hardening ---

    #[test]
    fn layer_tar_hash_verified_on_restore() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("data.txt"), "layer content").unwrap();

        let tar_data = pack_layer(src.path()).unwrap();
        let tar_hash = blake3::hash(&tar_data).to_hex().to_string();

        // Store the tar in object store
        let store_dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(store_dir.path());
        layout.initialize().unwrap();
        let obj_store = crate::ObjectStore::new(layout.clone());
        let stored_hash = obj_store.put(&tar_data).unwrap();

        // Verify stored hash matches computed hash
        assert_eq!(stored_hash, tar_hash);

        // Retrieve and verify integrity
        let retrieved = obj_store.get(&stored_hash).unwrap();
        let retrieved_hash = blake3::hash(&retrieved).to_hex().to_string();
        assert_eq!(retrieved_hash, tar_hash);

        // Unpack and verify content
        let dst = tempfile::tempdir().unwrap();
        unpack_layer(&retrieved, dst.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dst.path().join("data.txt")).unwrap(),
            "layer content"
        );
    }

    #[test]
    fn corrupt_layer_file_detected_on_read() {
        let (dir, store) = test_layer_store();
        let layer = sample_layer();
        let content_hash = store.put(&layer).unwrap();

        // Corrupt the layer file on disk
        let layer_path = StoreLayout::new(dir.path())
            .layers_dir()
            .join(&content_hash);
        fs::write(&layer_path, b"this is not valid JSON").unwrap();

        // get() must fail with an integrity error (hash mismatch)
        let result = store.get(&content_hash);
        assert!(
            result.is_err(),
            "corrupted layer manifest must fail on read"
        );
    }

    #[test]
    fn layer_manifest_hash_matches_content() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("file.txt"), "content").unwrap();
        let tar_data = pack_layer(src.path()).unwrap();
        let tar_hash = blake3::hash(&tar_data).to_hex().to_string();

        let layer = LayerManifest {
            hash: tar_hash.clone(),
            kind: LayerKind::Base,
            parent: None,
            object_refs: vec![tar_hash.clone()],
            read_only: true,
            tar_hash: tar_hash.clone(),
        };

        // Verify tar_hash in manifest matches actual content hash
        assert_eq!(layer.tar_hash, blake3::hash(&tar_data).to_hex().to_string());
        // Verify object_refs include the tar
        assert!(layer.object_refs.contains(&tar_hash));
    }

    #[test]
    fn partial_tar_write_detected() {
        let src = tempfile::tempdir().unwrap();
        fs::write(src.path().join("data.txt"), "real data").unwrap();
        let tar_data = pack_layer(src.path()).unwrap();

        // Simulate partial write: truncate the tar data
        let truncated = &tar_data[..tar_data.len() / 2];

        // Store the truncated data under the hash of the full data
        let store_dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(store_dir.path());
        layout.initialize().unwrap();
        let obj_store = crate::ObjectStore::new(layout);

        // Write the full data first to get the correct hash
        let correct_hash = obj_store.put(&tar_data).unwrap();

        // Now corrupt it with truncated data
        let obj_path = store_dir
            .path()
            .join("store")
            .join("objects")
            .join(&correct_hash);
        fs::write(&obj_path, truncated).unwrap();

        // Reading must detect integrity failure
        let result = obj_store.get(&correct_hash);
        assert!(
            result.is_err(),
            "truncated object must be detected as corrupt"
        );
    }

    #[test]
    fn compute_hash_matches_put() {
        let (_dir, store) = test_layer_store();
        let layer = sample_layer();
        let predicted = LayerStore::compute_hash(&layer).unwrap();
        let stored = store.put(&layer).unwrap();
        assert_eq!(predicted, stored, "compute_hash() must match put() hash");
    }

    #[test]
    fn corrupt_tar_data_fails_unpack() {
        // Garbage data should fail to unpack
        let garbage = b"this is not a tar archive at all";
        let dst = tempfile::tempdir().unwrap();
        let result = unpack_layer(garbage, dst.path());
        // tar::Archive may produce an empty archive or an error — both are acceptable
        // as long as no valid files are produced from garbage input
        if result.is_ok() {
            // If it "succeeded", verify no files were created
            let entries: Vec<_> = fs::read_dir(dst.path())
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            assert!(
                entries.is_empty(),
                "garbage tar data must not produce files"
            );
        }
    }
}
