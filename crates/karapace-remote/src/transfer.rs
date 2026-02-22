use crate::{BlobKind, Registry, RegistryEntry, RemoteBackend, RemoteError};
use karapace_store::{LayerStore, MetadataStore, ObjectStore, StoreLayout};

/// Result of a push operation.
#[derive(Debug)]
pub struct PushResult {
    pub objects_pushed: usize,
    pub layers_pushed: usize,
    pub objects_skipped: usize,
    pub layers_skipped: usize,
}

/// Result of a pull operation.
#[derive(Debug)]
pub struct PullResult {
    pub objects_pulled: usize,
    pub layers_pulled: usize,
    pub objects_skipped: usize,
    pub layers_skipped: usize,
}

/// Push an environment (metadata + layers + objects) to a remote store.
/// Optionally publish it under a registry key (e.g. `"my-env@latest"`).
pub fn push_env(
    layout: &StoreLayout,
    env_id: &str,
    backend: &dyn RemoteBackend,
    registry_key: Option<&str>,
) -> Result<PushResult, RemoteError> {
    let meta_store = MetadataStore::new(layout.clone());
    let layer_store = LayerStore::new(layout.clone());
    let object_store = ObjectStore::new(layout.clone());

    // 1. Read metadata
    let meta = meta_store.get(env_id)?;
    let meta_json =
        serde_json::to_vec_pretty(&meta).map_err(|e| RemoteError::Serialization(e.to_string()))?;

    // 2. Collect all layer hashes (base + deps)
    let mut layer_hashes = vec![meta.base_layer.clone()];
    layer_hashes.extend(meta.dependency_layers.iter().cloned());

    // 3. Collect all object hashes from layers + manifest
    let mut object_hashes = Vec::new();
    if !meta.manifest_hash.is_empty() {
        object_hashes.push(meta.manifest_hash.to_string());
    }
    for lh in &layer_hashes {
        let layer = layer_store.get(lh)?;
        object_hashes.extend(layer.object_refs.iter().cloned());
    }
    object_hashes.sort();
    object_hashes.dedup();

    // 4. Push objects (skip existing)
    let mut objects_pushed = 0;
    let mut objects_skipped = 0;
    for hash in &object_hashes {
        if backend.has_blob(BlobKind::Object, hash)? {
            objects_skipped += 1;
            continue;
        }
        let data = object_store.get(hash)?;
        backend.put_blob(BlobKind::Object, hash, &data)?;
        objects_pushed += 1;
    }

    // 5. Push layers (skip existing)
    let mut layers_pushed = 0;
    let mut layers_skipped = 0;
    for lh in &layer_hashes {
        if backend.has_blob(BlobKind::Layer, lh)? {
            layers_skipped += 1;
            continue;
        }
        let layer = layer_store.get(lh)?;
        let data = serde_json::to_vec_pretty(&layer)
            .map_err(|e| RemoteError::Serialization(e.to_string()))?;
        backend.put_blob(BlobKind::Layer, lh, &data)?;
        layers_pushed += 1;
    }

    // 6. Push metadata
    backend.put_blob(BlobKind::Metadata, env_id, &meta_json)?;

    // 7. Update registry if key provided
    if let Some(key) = registry_key {
        let mut registry = match backend.get_registry() {
            Ok(data) => Registry::from_bytes(&data).unwrap_or_default(),
            Err(_) => Registry::new(),
        };
        registry.publish(
            key,
            RegistryEntry {
                env_id: meta.env_id.to_string(),
                short_id: meta.short_id.to_string(),
                name: meta.name.clone(),
                pushed_at: chrono::Utc::now().to_rfc3339(),
            },
        );
        let reg_bytes = registry.to_bytes()?;
        backend.put_registry(&reg_bytes)?;
    }

    Ok(PushResult {
        objects_pushed,
        layers_pushed,
        objects_skipped,
        layers_skipped,
    })
}

/// Pull an environment from a remote store into the local store.
pub fn pull_env(
    layout: &StoreLayout,
    env_id: &str,
    backend: &dyn RemoteBackend,
) -> Result<PullResult, RemoteError> {
    let meta_store = MetadataStore::new(layout.clone());
    let layer_store = LayerStore::new(layout.clone());
    let object_store = ObjectStore::new(layout.clone());

    // 1. Download metadata and verify checksum if present
    let meta_bytes = backend.get_blob(BlobKind::Metadata, env_id)?;
    let meta: karapace_store::EnvMetadata = serde_json::from_slice(&meta_bytes)
        .map_err(|e| RemoteError::Serialization(format!("invalid metadata: {e}")))?;
    if let Some(ref expected) = meta.checksum {
        let mut copy = meta.clone();
        copy.checksum = None;
        let json = serde_json::to_string_pretty(&copy)
            .map_err(|e| RemoteError::Serialization(e.to_string()))?;
        let actual = blake3::hash(json.as_bytes()).to_hex().to_string();
        if actual != *expected {
            return Err(RemoteError::IntegrityFailure {
                key: format!("metadata:{env_id}"),
                expected: expected.clone(),
                actual,
            });
        }
    }

    // 2. Collect layer hashes
    let mut layer_hashes = vec![meta.base_layer.clone()];
    layer_hashes.extend(meta.dependency_layers.iter().cloned());

    // 3. Download layers (skip existing)
    let mut layers_pulled = 0;
    let mut layers_skipped = 0;
    let mut object_hashes = Vec::new();
    if !meta.manifest_hash.is_empty() {
        object_hashes.push(meta.manifest_hash.to_string());
    }
    for lh in &layer_hashes {
        if layer_store.exists(lh) {
            let layer = layer_store.get(lh)?;
            object_hashes.extend(layer.object_refs.iter().cloned());
            layers_skipped += 1;
            continue;
        }
        let data = backend.get_blob(BlobKind::Layer, lh)?;
        let layer: karapace_store::LayerManifest = serde_json::from_slice(&data)
            .map_err(|e| RemoteError::Serialization(format!("invalid layer: {e}")))?;
        object_hashes.extend(layer.object_refs.iter().cloned());
        let stored_hash = layer_store.put(&layer)?;
        if stored_hash != **lh {
            return Err(RemoteError::IntegrityFailure {
                key: lh.to_string(),
                expected: lh.to_string(),
                actual: stored_hash,
            });
        }
        layers_pulled += 1;
    }
    object_hashes.sort();
    object_hashes.dedup();

    // 4. Download objects (skip existing, verify blake3 integrity)
    let mut objects_pulled = 0;
    let mut objects_skipped = 0;
    for hash in &object_hashes {
        if object_store.exists(hash) {
            objects_skipped += 1;
            continue;
        }
        let data = backend.get_blob(BlobKind::Object, hash)?;
        let actual = blake3::hash(&data).to_hex().to_string();
        if actual != *hash {
            return Err(RemoteError::IntegrityFailure {
                key: hash.clone(),
                expected: hash.clone(),
                actual,
            });
        }
        object_store.put(&data)?;
        objects_pulled += 1;
    }

    // 5. Store metadata locally
    meta_store.put(&meta)?;

    Ok(PullResult {
        objects_pulled,
        layers_pulled,
        objects_skipped,
        layers_skipped,
    })
}

/// Resolve a registry reference (e.g. "my-env@latest") to an env_id using the remote registry.
pub fn resolve_ref(backend: &dyn RemoteBackend, reference: &str) -> Result<String, RemoteError> {
    let reg_bytes = backend.get_registry()?;
    let registry = Registry::from_bytes(&reg_bytes)?;
    let (name, tag) = crate::registry::parse_ref(reference);
    let key = format!("{name}@{tag}");
    let entry = registry
        .lookup(&key)
        .ok_or_else(|| RemoteError::NotFound(format!("registry key '{key}' not found")))?;
    Ok(entry.env_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory mock remote backend for testing.
    struct MockRemote {
        blobs: Mutex<HashMap<String, Vec<u8>>>,
        registry: Mutex<Option<Vec<u8>>>,
    }

    impl MockRemote {
        fn new() -> Self {
            Self {
                blobs: Mutex::new(HashMap::new()),
                registry: Mutex::new(None),
            }
        }

        fn blob_key(kind: BlobKind, key: &str) -> String {
            format!("{kind:?}/{key}")
        }
    }

    impl RemoteBackend for MockRemote {
        fn put_blob(&self, kind: BlobKind, key: &str, data: &[u8]) -> Result<(), RemoteError> {
            self.blobs
                .lock()
                .unwrap()
                .insert(Self::blob_key(kind, key), data.to_vec());
            Ok(())
        }

        fn get_blob(&self, kind: BlobKind, key: &str) -> Result<Vec<u8>, RemoteError> {
            self.blobs
                .lock()
                .unwrap()
                .get(&Self::blob_key(kind, key))
                .cloned()
                .ok_or_else(|| RemoteError::NotFound(key.to_owned()))
        }

        fn has_blob(&self, kind: BlobKind, key: &str) -> Result<bool, RemoteError> {
            Ok(self
                .blobs
                .lock()
                .unwrap()
                .contains_key(&Self::blob_key(kind, key)))
        }

        fn list_blobs(&self, kind: BlobKind) -> Result<Vec<String>, RemoteError> {
            let prefix = format!("{kind:?}/");
            let blobs = self.blobs.lock().unwrap();
            Ok(blobs
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .map(|k| k[prefix.len()..].to_owned())
                .collect())
        }

        fn put_registry(&self, data: &[u8]) -> Result<(), RemoteError> {
            *self.registry.lock().unwrap() = Some(data.to_vec());
            Ok(())
        }

        fn get_registry(&self) -> Result<Vec<u8>, RemoteError> {
            self.registry
                .lock()
                .unwrap()
                .clone()
                .ok_or_else(|| RemoteError::NotFound("registry".to_owned()))
        }
    }

    fn setup_local_env(dir: &std::path::Path) -> (StoreLayout, String) {
        let layout = StoreLayout::new(dir);
        layout.initialize().unwrap();

        let obj_store = ObjectStore::new(layout.clone());
        let layer_store = LayerStore::new(layout.clone());
        let meta_store = MetadataStore::new(layout.clone());

        // Create a test object (layer content)
        let obj_hash = obj_store.put(b"test data content").unwrap();

        // Create a manifest object (environment manifest)
        let manifest_hash = obj_store.put(b"{\"manifest\": \"test\"}").unwrap();

        // Create a base layer referencing the object
        let layer = karapace_store::LayerManifest {
            hash: "layer_hash_001".to_owned(),
            kind: karapace_store::LayerKind::Base,
            parent: None,
            object_refs: vec![obj_hash],
            read_only: true,
            tar_hash: String::new(),
        };
        let layer_content_hash = layer_store.put(&layer).unwrap();

        // Create environment metadata
        let meta = karapace_store::EnvMetadata {
            env_id: "env_abc123".into(),
            short_id: "env_abc123".into(),
            name: Some("test-env".to_owned()),
            state: karapace_store::EnvState::Built,
            base_layer: layer_content_hash.into(),
            dependency_layers: vec![],
            policy_layer: None,
            manifest_hash: manifest_hash.into(),
            ref_count: 1,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        (layout, "env_abc123".to_owned())
    }

    #[test]
    fn push_and_pull_roundtrip() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());

        let remote = MockRemote::new();

        // Push
        let push_result = push_env(&src_layout, &env_id, &remote, Some("test-env@latest")).unwrap();
        assert_eq!(push_result.objects_pushed, 2); // layer content + manifest
        assert_eq!(push_result.layers_pushed, 1);
        assert_eq!(push_result.objects_skipped, 0);

        // Pull into a fresh store
        let dst_dir = tempfile::tempdir().unwrap();
        let dst_layout = StoreLayout::new(dst_dir.path());
        dst_layout.initialize().unwrap();

        let pull_result = pull_env(&dst_layout, &env_id, &remote).unwrap();
        assert_eq!(pull_result.objects_pulled, 2); // layer content + manifest
        assert_eq!(pull_result.layers_pulled, 1);

        // Verify metadata exists in destination
        let dst_meta = MetadataStore::new(dst_layout);
        let meta = dst_meta.get(&env_id).unwrap();
        assert_eq!(meta.name, Some("test-env".to_owned()));
    }

    #[test]
    fn push_skips_existing_blobs() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let remote = MockRemote::new();

        // Push once
        push_env(&src_layout, &env_id, &remote, None).unwrap();

        // Push again — should skip everything
        let result = push_env(&src_layout, &env_id, &remote, None).unwrap();
        assert_eq!(result.objects_skipped, 2); // layer content + manifest
        assert_eq!(result.layers_skipped, 1);
        assert_eq!(result.objects_pushed, 0);
        assert_eq!(result.layers_pushed, 0);
    }

    #[test]
    fn resolve_ref_from_registry() {
        let remote = MockRemote::new();

        // Manually push a registry
        let mut reg = Registry::new();
        reg.publish(
            "my-env@latest",
            RegistryEntry {
                env_id: "hash_xyz".to_owned(),
                short_id: "hash_xyz".to_owned(),
                name: None,
                pushed_at: "t".to_owned(),
            },
        );
        remote.put_registry(&reg.to_bytes().unwrap()).unwrap();

        let resolved = resolve_ref(&remote, "my-env@latest").unwrap();
        assert_eq!(resolved, "hash_xyz");

        // Without @tag → defaults to @latest
        let resolved2 = resolve_ref(&remote, "my-env").unwrap();
        assert_eq!(resolved2, "hash_xyz");
    }

    #[test]
    fn pull_nonexistent_env_fails() {
        let remote = MockRemote::new();
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();

        let result = pull_env(&layout, "nonexistent_env", &remote);
        assert!(result.is_err());
    }

    #[test]
    fn resolve_ref_not_found_fails() {
        let remote = MockRemote::new();
        let mut reg = Registry::new();
        reg.publish(
            "other@latest",
            RegistryEntry {
                env_id: "xyz".to_owned(),
                short_id: "xyz".to_owned(),
                name: None,
                pushed_at: "t".to_owned(),
            },
        );
        remote.put_registry(&reg.to_bytes().unwrap()).unwrap();

        let result = resolve_ref(&remote, "missing-env@latest");
        assert!(result.is_err());
    }

    #[test]
    fn pull_skips_existing_objects() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let remote = MockRemote::new();

        push_env(&src_layout, &env_id, &remote, None).unwrap();

        // Pull into destination that already has the objects
        let dst_dir = tempfile::tempdir().unwrap();
        let dst_layout = StoreLayout::new(dst_dir.path());
        dst_layout.initialize().unwrap();

        // First pull
        pull_env(&dst_layout, &env_id, &remote).unwrap();

        // Second pull — should skip existing
        let result = pull_env(&dst_layout, &env_id, &remote).unwrap();
        assert_eq!(result.objects_skipped, 2); // layer content + manifest
        assert_eq!(result.layers_skipped, 1);
        assert_eq!(result.objects_pulled, 0);
        assert_eq!(result.layers_pulled, 0);
    }

    #[test]
    fn push_result_fields_correct() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let remote = MockRemote::new();

        let result = push_env(&src_layout, &env_id, &remote, None).unwrap();
        assert!(result.objects_pushed > 0 || result.objects_skipped > 0);
        assert!(result.layers_pushed > 0 || result.layers_skipped > 0);
    }

    #[test]
    fn pull_transfers_manifest_object() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let remote = MockRemote::new();

        push_env(&src_layout, &env_id, &remote, None).unwrap();

        // Pull into a fresh store
        let dst_dir = tempfile::tempdir().unwrap();
        let dst_layout = StoreLayout::new(dst_dir.path());
        dst_layout.initialize().unwrap();

        pull_env(&dst_layout, &env_id, &remote).unwrap();

        // Verify the manifest object is accessible in the destination store
        let dst_meta = MetadataStore::new(dst_layout.clone());
        let meta = dst_meta.get(&env_id).unwrap();
        let dst_obj = ObjectStore::new(dst_layout);
        let manifest_data = dst_obj.get(&meta.manifest_hash);
        assert!(
            manifest_data.is_ok(),
            "manifest object must be available after pull: {:?}",
            manifest_data.err()
        );
    }

    #[test]
    fn pull_detects_tampered_metadata_checksum() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let remote = MockRemote::new();

        // Push to populate the remote
        push_env(&src_layout, &env_id, &remote, None).unwrap();

        // Tamper with the metadata blob on the remote: change the name field
        // but leave the checksum intact (so it mismatches)
        let meta_bytes = remote.get_blob(BlobKind::Metadata, &env_id).unwrap();
        let mut meta: serde_json::Value = serde_json::from_slice(&meta_bytes).unwrap();
        meta["name"] = serde_json::Value::String("tampered".into());
        let tampered = serde_json::to_string_pretty(&meta).unwrap();
        remote
            .put_blob(BlobKind::Metadata, &env_id, tampered.as_bytes())
            .unwrap();

        // Pull into a fresh store — should fail with integrity error
        let dst_dir = tempfile::tempdir().unwrap();
        let dst_layout = StoreLayout::new(dst_dir.path());
        dst_layout.initialize().unwrap();

        let result = pull_env(&dst_layout, &env_id, &remote);
        assert!(
            result.is_err(),
            "pull must fail when metadata checksum is tampered"
        );
    }

    #[test]
    fn push_with_tag_publishes_registry() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let remote = MockRemote::new();

        push_env(&src_layout, &env_id, &remote, Some("my-app@v1")).unwrap();

        // Verify registry was published
        let reg_bytes = remote.get_registry().unwrap();
        let reg = Registry::from_bytes(&reg_bytes).unwrap();
        let entry = reg.lookup("my-app@v1").unwrap();
        assert_eq!(entry.env_id, env_id);
    }

    // --- §7: Network failure simulation ---

    /// Mock remote that fails on the Nth put_blob call.
    struct FailOnPutRemote {
        inner: MockRemote,
        call_count: Mutex<usize>,
        fail_on: usize,
    }

    impl FailOnPutRemote {
        fn new(fail_on: usize) -> Self {
            Self {
                inner: MockRemote::new(),
                call_count: Mutex::new(0),
                fail_on,
            }
        }
    }

    impl RemoteBackend for FailOnPutRemote {
        fn put_blob(&self, kind: BlobKind, key: &str, data: &[u8]) -> Result<(), RemoteError> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            if *count >= self.fail_on {
                return Err(RemoteError::Http("simulated network failure".to_owned()));
            }
            drop(count);
            self.inner.put_blob(kind, key, data)
        }

        fn get_blob(&self, kind: BlobKind, key: &str) -> Result<Vec<u8>, RemoteError> {
            self.inner.get_blob(kind, key)
        }

        fn has_blob(&self, kind: BlobKind, key: &str) -> Result<bool, RemoteError> {
            self.inner.has_blob(kind, key)
        }

        fn list_blobs(&self, kind: BlobKind) -> Result<Vec<String>, RemoteError> {
            self.inner.list_blobs(kind)
        }

        fn put_registry(&self, data: &[u8]) -> Result<(), RemoteError> {
            self.inner.put_registry(data)
        }

        fn get_registry(&self) -> Result<Vec<u8>, RemoteError> {
            self.inner.get_registry()
        }
    }

    /// Mock remote that returns garbage on get_blob.
    struct CorruptGetRemote {
        inner: MockRemote,
    }

    impl CorruptGetRemote {
        fn new() -> Self {
            Self {
                inner: MockRemote::new(),
            }
        }
    }

    impl RemoteBackend for CorruptGetRemote {
        fn put_blob(&self, kind: BlobKind, key: &str, data: &[u8]) -> Result<(), RemoteError> {
            self.inner.put_blob(kind, key, data)
        }

        fn get_blob(&self, kind: BlobKind, key: &str) -> Result<Vec<u8>, RemoteError> {
            // Return corrupted data for objects (not metadata/layers which are JSON)
            if matches!(kind, BlobKind::Object) {
                let real = self.inner.get_blob(kind, key)?;
                let mut corrupted = real;
                if !corrupted.is_empty() {
                    corrupted[0] ^= 0xFF;
                }
                Ok(corrupted)
            } else {
                self.inner.get_blob(kind, key)
            }
        }

        fn has_blob(&self, kind: BlobKind, key: &str) -> Result<bool, RemoteError> {
            self.inner.has_blob(kind, key)
        }

        fn list_blobs(&self, kind: BlobKind) -> Result<Vec<String>, RemoteError> {
            self.inner.list_blobs(kind)
        }

        fn put_registry(&self, data: &[u8]) -> Result<(), RemoteError> {
            self.inner.put_registry(data)
        }

        fn get_registry(&self) -> Result<Vec<u8>, RemoteError> {
            self.inner.get_registry()
        }
    }

    #[test]
    fn push_fails_on_network_error() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());

        // Fail on the very first put_blob call
        let remote = FailOnPutRemote::new(1);
        let result = push_env(&src_layout, &env_id, &remote, None);
        assert!(
            result.is_err(),
            "push must fail when network error occurs during upload"
        );
    }

    #[test]
    fn pull_detects_corrupted_remote_object() {
        let src_dir = tempfile::tempdir().unwrap();
        let (src_layout, env_id) = setup_local_env(src_dir.path());
        let corrupt_remote = CorruptGetRemote::new();

        // Push via the inner (uncorrupted) remote first
        push_env(&src_layout, &env_id, &corrupt_remote.inner, None).unwrap();

        // Pull via the corrupting remote — objects will have flipped bytes
        let dst_dir = tempfile::tempdir().unwrap();
        let dst_layout = StoreLayout::new(dst_dir.path());
        dst_layout.initialize().unwrap();

        let result = pull_env(&dst_layout, &env_id, &corrupt_remote);
        assert!(
            result.is_err(),
            "pull must fail when remote returns corrupted object data"
        );
    }

    #[test]
    fn large_object_push_pull_roundtrip() {
        let src_dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(src_dir.path());
        layout.initialize().unwrap();

        let obj_store = ObjectStore::new(layout.clone());
        let layer_store = LayerStore::new(layout.clone());
        let meta_store = MetadataStore::new(layout.clone());

        // Create a 1MB object (simulating a large layer tar)
        let large_data: Vec<u8> = (0..1_048_576u32).map(|i| (i % 256) as u8).collect();
        let obj_hash = obj_store.put(&large_data).unwrap();

        let manifest_hash = obj_store.put(b"{\"manifest\": \"large\"}").unwrap();

        let layer = karapace_store::LayerManifest {
            hash: "large_layer".to_owned(),
            kind: karapace_store::LayerKind::Base,
            parent: None,
            object_refs: vec![obj_hash],
            read_only: true,
            tar_hash: String::new(),
        };
        let layer_hash = layer_store.put(&layer).unwrap();

        let meta = karapace_store::EnvMetadata {
            env_id: "large_env".into(),
            short_id: "large_env".into(),
            name: None,
            state: karapace_store::EnvState::Built,
            base_layer: layer_hash.into(),
            dependency_layers: vec![],
            policy_layer: None,
            manifest_hash: manifest_hash.into(),
            ref_count: 1,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let remote = MockRemote::new();
        push_env(&layout, "large_env", &remote, None).unwrap();

        // Pull into fresh store
        let dst_dir = tempfile::tempdir().unwrap();
        let dst_layout = StoreLayout::new(dst_dir.path());
        dst_layout.initialize().unwrap();

        let result = pull_env(&dst_layout, "large_env", &remote).unwrap();
        assert_eq!(result.objects_pulled, 2);

        // Verify the large object survived the roundtrip
        let dst_obj = ObjectStore::new(dst_layout);
        let pulled = dst_obj.get(&meta.manifest_hash).unwrap();
        assert_eq!(pulled, b"{\"manifest\": \"large\"}");
    }
}
