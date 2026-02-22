use crate::layers::LayerStore;
use crate::layout::StoreLayout;
use crate::metadata::MetadataStore;
use crate::objects::ObjectStore;
use crate::StoreError;

#[derive(Debug, Default)]
pub struct IntegrityReport {
    pub checked: usize,
    pub passed: usize,
    pub failed: Vec<IntegrityFailure>,
    pub layers_checked: usize,
    pub layers_passed: usize,
    pub metadata_checked: usize,
    pub metadata_passed: usize,
}

#[derive(Debug)]
pub struct IntegrityFailure {
    pub hash: String,
    pub reason: String,
}

pub fn verify_store_integrity(layout: &StoreLayout) -> Result<IntegrityReport, StoreError> {
    let object_store = ObjectStore::new(layout.clone());
    let layer_store = LayerStore::new(layout.clone());
    let meta_store = MetadataStore::new(layout.clone());

    let all_objects = object_store.list()?;
    let all_layers = layer_store.list()?;
    let all_meta = meta_store.list()?;

    let mut report = IntegrityReport {
        checked: all_objects.len(),
        layers_checked: all_layers.len(),
        metadata_checked: all_meta.len(),
        ..Default::default()
    };

    // Verify objects (blake3 content-addressed)
    for hash in &all_objects {
        match object_store.get(hash) {
            Ok(_) => report.passed += 1,
            Err(StoreError::IntegrityFailure { actual, .. }) => {
                report.failed.push(IntegrityFailure {
                    hash: hash.clone(),
                    reason: format!("object hash mismatch: got {actual}"),
                });
            }
            Err(e) => {
                report.failed.push(IntegrityFailure {
                    hash: hash.clone(),
                    reason: format!("object read error: {e}"),
                });
            }
        }
    }

    // Verify layers (blake3 content-addressed)
    for hash in &all_layers {
        match layer_store.get(hash) {
            Ok(_) => report.layers_passed += 1,
            Err(StoreError::IntegrityFailure { actual, .. }) => {
                report.failed.push(IntegrityFailure {
                    hash: hash.clone(),
                    reason: format!("layer hash mismatch: got {actual}"),
                });
            }
            Err(e) => {
                report.failed.push(IntegrityFailure {
                    hash: hash.clone(),
                    reason: format!("layer read error: {e}"),
                });
            }
        }
    }

    // Verify metadata (embedded checksum)
    for meta in &all_meta {
        match meta_store.get(&meta.env_id) {
            Ok(_) => report.metadata_passed += 1,
            Err(StoreError::IntegrityFailure { actual, .. }) => {
                report.failed.push(IntegrityFailure {
                    hash: meta.env_id.to_string(),
                    reason: format!("metadata checksum mismatch: got {actual}"),
                });
            }
            Err(e) => {
                report.failed.push(IntegrityFailure {
                    hash: meta.env_id.to_string(),
                    reason: format!("metadata read error: {e}"),
                });
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_store_passes_integrity() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let obj_store = ObjectStore::new(layout.clone());
        obj_store.put(b"data1").unwrap();
        obj_store.put(b"data2").unwrap();

        let report = verify_store_integrity(&layout).unwrap();
        assert_eq!(report.checked, 2);
        assert_eq!(report.passed, 2);
        assert!(report.failed.is_empty());
    }

    #[test]
    fn corrupted_object_detected() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let obj_store = ObjectStore::new(layout.clone());
        let hash = obj_store.put(b"original").unwrap();

        std::fs::write(layout.objects_dir().join(&hash), b"corrupted").unwrap();

        let report = verify_store_integrity(&layout).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].hash, hash);
    }

    #[test]
    fn verify_store_checks_layers() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let layer_store = LayerStore::new(layout.clone());
        let layer = crate::LayerManifest {
            hash: "test".to_owned(),
            kind: crate::LayerKind::Base,
            parent: None,
            object_refs: vec![],
            read_only: true,
            tar_hash: String::new(),
        };
        layer_store.put(&layer).unwrap();

        let report = verify_store_integrity(&layout).unwrap();
        assert_eq!(report.layers_checked, 1);
        assert_eq!(report.layers_passed, 1);
    }

    #[test]
    fn verify_store_detects_corrupt_layer() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let layer_store = LayerStore::new(layout.clone());
        let layer = crate::LayerManifest {
            hash: "test".to_owned(),
            kind: crate::LayerKind::Base,
            parent: None,
            object_refs: vec![],
            read_only: true,
            tar_hash: String::new(),
        };
        let hash = layer_store.put(&layer).unwrap();

        // Corrupt the layer file
        std::fs::write(layout.layers_dir().join(&hash), b"corrupted").unwrap();

        let report = verify_store_integrity(&layout).unwrap();
        assert_eq!(report.layers_checked, 1);
        assert_eq!(report.layers_passed, 0);
        assert!(!report.failed.is_empty());
    }

    #[test]
    fn verify_store_checks_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let meta_store = MetadataStore::new(layout.clone());
        let meta = crate::EnvMetadata {
            env_id: "test_env".into(),
            short_id: "test_env".into(),
            name: None,
            state: crate::EnvState::Built,
            manifest_hash: "mhash".into(),
            base_layer: "base".into(),
            dependency_layers: vec![],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 1,
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let report = verify_store_integrity(&layout).unwrap();
        assert_eq!(report.metadata_checked, 1);
        assert_eq!(report.metadata_passed, 1);
    }

    #[test]
    fn empty_store_passes() {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let report = verify_store_integrity(&layout).unwrap();
        assert_eq!(report.checked, 0);
        assert_eq!(report.layers_checked, 0);
        assert_eq!(report.metadata_checked, 0);
        assert!(report.failed.is_empty());
    }
}
