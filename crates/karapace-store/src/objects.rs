use crate::layout::StoreLayout;
use crate::{fsync_dir, StoreError};
use std::fs;
use std::io::Write;
use tempfile::NamedTempFile;

/// Content-addressable object store backed by blake3 hashing.
///
/// Objects are stored as files named by their blake3 hash. Writes are atomic
/// via `NamedTempFile`, and reads verify integrity by recomputing the hash.
pub struct ObjectStore {
    layout: StoreLayout,
}

impl ObjectStore {
    pub fn new(layout: StoreLayout) -> Self {
        Self { layout }
    }

    /// Store data and return its blake3 hash. Idempotent — existing objects are skipped.
    pub fn put(&self, data: &[u8]) -> Result<String, StoreError> {
        let hash = blake3::hash(data).to_hex().to_string();
        let dest = self.layout.objects_dir().join(&hash);

        if dest.exists() {
            return Ok(hash);
        }

        let dir = self.layout.objects_dir();
        let mut tmp = NamedTempFile::new_in(&dir)?;
        tmp.write_all(data)?;
        tmp.as_file().sync_all()?;
        tmp.persist(&dest).map_err(|e| StoreError::Io(e.error))?;
        fsync_dir(&dir)?;

        Ok(hash)
    }

    /// Retrieve data by hash, verifying integrity on read.
    pub fn get(&self, hash: &str) -> Result<Vec<u8>, StoreError> {
        let path = self.layout.objects_dir().join(hash);
        if !path.exists() {
            return Err(StoreError::ObjectNotFound(hash.to_owned()));
        }
        let data = fs::read(&path)?;

        let actual = blake3::hash(&data);
        let actual_hex = actual.to_hex();
        if actual_hex.as_str() != hash {
            return Err(StoreError::IntegrityFailure {
                hash: hash.to_owned(),
                expected: hash.to_owned(),
                actual: actual_hex.to_string(),
            });
        }

        Ok(data)
    }

    pub fn exists(&self, hash: &str) -> bool {
        self.layout.objects_dir().join(hash).exists()
    }

    pub fn remove(&self, hash: &str) -> Result<(), StoreError> {
        let path = self.layout.objects_dir().join(hash);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<String>, StoreError> {
        let dir = self.layout.objects_dir();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> (tempfile::TempDir, ObjectStore) {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        let store = ObjectStore::new(layout);
        (dir, store)
    }

    #[test]
    fn put_and_get_roundtrip() {
        let (_dir, store) = test_store();
        let data = b"hello karapace";
        let hash = store.put(data).unwrap();
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn put_is_idempotent() {
        let (_dir, store) = test_store();
        let data = b"hello";
        let h1 = store.put(data).unwrap();
        let h2 = store.put(data).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn get_nonexistent_fails() {
        let (_dir, store) = test_store();
        assert!(store.get("nonexistent").is_err());
    }

    #[test]
    fn integrity_check_on_read() {
        let (dir, store) = test_store();
        let data = b"test data";
        let hash = store.put(data).unwrap();

        let obj_path = StoreLayout::new(dir.path()).objects_dir().join(&hash);
        fs::write(&obj_path, b"corrupted").unwrap();

        assert!(store.get(&hash).is_err());
    }

    #[test]
    fn list_objects() {
        let (_dir, store) = test_store();
        store.put(b"aaa").unwrap();
        store.put(b"bbb").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn remove_object() {
        let (_dir, store) = test_store();
        let hash = store.put(b"data").unwrap();
        assert!(store.exists(&hash));
        store.remove(&hash).unwrap();
        assert!(!store.exists(&hash));
    }

    #[test]
    fn put_empty_data() {
        let (_dir, store) = test_store();
        let hash = store.put(b"").unwrap();
        let retrieved = store.get(&hash).unwrap();
        assert!(retrieved.is_empty());
    }

    #[test]
    fn put_large_data() {
        let (_dir, store) = test_store();
        let data = vec![0xABu8; 1024 * 64]; // 64KB
        let hash = store.put(&data).unwrap();
        let retrieved = store.get(&hash).unwrap();
        assert_eq!(retrieved.len(), 1024 * 64);
    }

    #[test]
    fn list_empty_store() {
        let (_dir, store) = test_store();
        let list = store.list().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn remove_nonexistent_is_ok() {
        let (_dir, store) = test_store();
        assert!(store.remove("nonexistent").is_ok());
    }

    #[test]
    fn exists_nonexistent_is_false() {
        let (_dir, store) = test_store();
        assert!(!store.exists("nonexistent"));
    }

    #[test]
    fn hash_is_deterministic() {
        let (_dir, store) = test_store();
        let h1 = store.put(b"deterministic").unwrap();
        let h2 = store.put(b"deterministic").unwrap();
        assert_eq!(h1, h2);
        // Different data should produce different hash
        let h3 = store.put(b"different").unwrap();
        assert_ne!(h1, h3);
    }
}
