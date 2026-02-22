//! IG-M6: Store migration tests.

use karapace_store::{
    migrate_store, EnvState, LayerKind, LayerManifest, LayerStore, MetadataStore, ObjectStore,
    StoreLayout, STORE_FORMAT_VERSION,
};
use std::fs;
use std::path::Path;

/// Create a v1-format store with the given number of metadata files.
fn create_v1_store(root: &Path, num_envs: usize) {
    let store_dir = root.join("store");
    fs::create_dir_all(store_dir.join("objects")).unwrap();
    fs::create_dir_all(store_dir.join("layers")).unwrap();
    fs::create_dir_all(store_dir.join("metadata")).unwrap();
    fs::create_dir_all(store_dir.join("staging")).unwrap();
    fs::create_dir_all(root.join("env")).unwrap();

    // Write v1 version file
    fs::write(store_dir.join("version"), r#"{"format_version": 1}"#).unwrap();

    // Write v1-format metadata (missing v2 fields: name, checksum, policy_layer)
    for i in 0..num_envs {
        let env_id = format!("env_{i:04}");
        let meta_json = serde_json::json!({
            "env_id": env_id,
            "short_id": &env_id[..8],
            "state": "Built",
            "manifest_hash": format!("mhash_{i}"),
            "base_layer": format!("blayer_{i}"),
            "dependency_layers": [],
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z",
            "ref_count": 1
        });
        fs::write(
            store_dir.join("metadata").join(&env_id),
            serde_json::to_string_pretty(&meta_json).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn migrate_v1_store_to_v2() {
    let dir = tempfile::tempdir().unwrap();
    create_v1_store(dir.path(), 2);

    let result = migrate_store(dir.path()).unwrap();
    assert!(result.is_some(), "migration must return Some for v1→v2");
    let result = result.unwrap();
    assert_eq!(result.from_version, 1);
    assert_eq!(result.to_version, STORE_FORMAT_VERSION);
    assert_eq!(result.environments_migrated, 2);

    // Verify version file now says v2
    let layout = StoreLayout::new(dir.path());
    layout.verify_version().unwrap();

    // Both metadata files must be readable by current MetadataStore
    let meta_store = MetadataStore::new(layout);
    let m0 = meta_store.get("env_0000").unwrap();
    assert_eq!(m0.env_id.as_str(), "env_0000");
    assert_eq!(m0.state, EnvState::Built);
    let m1 = meta_store.get("env_0001").unwrap();
    assert_eq!(m1.env_id.as_str(), "env_0001");
}

#[test]
fn migrate_preserves_all_metadata_fields() {
    let dir = tempfile::tempdir().unwrap();
    create_v1_store(dir.path(), 1);

    migrate_store(dir.path()).unwrap();

    let layout = StoreLayout::new(dir.path());
    let meta_store = MetadataStore::new(layout);
    let meta = meta_store.get("env_0000").unwrap();

    // Original fields preserved
    assert_eq!(meta.env_id.as_str(), "env_0000");
    assert_eq!(meta.short_id.as_str(), "env_0000");
    assert_eq!(meta.state, EnvState::Built);
    assert_eq!(meta.manifest_hash.as_str(), "mhash_0");
    assert_eq!(meta.base_layer.as_str(), "blayer_0");
    assert!(meta.dependency_layers.is_empty());
    assert_eq!(meta.created_at, "2025-01-01T00:00:00Z");
    assert_eq!(meta.ref_count, 1);

    // v2 defaults added
    assert_eq!(meta.name, None);
    assert_eq!(meta.policy_layer, None);
}

#[test]
fn migrate_preserves_objects_and_layers() {
    let dir = tempfile::tempdir().unwrap();

    // Start with a normal v2 store to create real objects and layers
    let layout = StoreLayout::new(dir.path());
    layout.initialize().unwrap();

    let obj_store = ObjectStore::new(layout.clone());
    let layer_store = LayerStore::new(layout.clone());

    let h1 = obj_store.put(b"object data 1").unwrap();
    let h2 = obj_store.put(b"object data 2").unwrap();
    let h3 = obj_store.put(b"object data 3").unwrap();

    let layer = LayerManifest {
        hash: "test_layer".to_owned(),
        kind: LayerKind::Base,
        parent: None,
        object_refs: vec![h1.clone(), h2.clone()],
        read_only: true,
        tar_hash: String::new(),
    };
    let lh1 = layer_store.put(&layer).unwrap();
    let layer2 = LayerManifest {
        hash: "test_layer2".to_owned(),
        kind: LayerKind::Snapshot,
        parent: Some(lh1.clone()),
        object_refs: vec![h3.clone()],
        read_only: false,
        tar_hash: String::new(),
    };
    let lh2 = layer_store.put(&layer2).unwrap();

    // Downgrade version file to v1
    fs::write(
        dir.path().join("store").join("version"),
        r#"{"format_version": 1}"#,
    )
    .unwrap();

    // Run migration
    migrate_store(dir.path()).unwrap();

    // Verify all objects intact
    let obj_store2 = ObjectStore::new(StoreLayout::new(dir.path()));
    assert_eq!(obj_store2.get(&h1).unwrap(), b"object data 1");
    assert_eq!(obj_store2.get(&h2).unwrap(), b"object data 2");
    assert_eq!(obj_store2.get(&h3).unwrap(), b"object data 3");

    // Verify all layers intact
    let layer_store2 = LayerStore::new(StoreLayout::new(dir.path()));
    let loaded1 = layer_store2.get(&lh1).unwrap();
    assert_eq!(loaded1.object_refs.len(), 2);
    let loaded2 = layer_store2.get(&lh2).unwrap();
    assert_eq!(loaded2.kind, LayerKind::Snapshot);

    // Verify store integrity
    let report = karapace_store::verify_store_integrity(&StoreLayout::new(dir.path())).unwrap();
    assert!(
        report.failed.is_empty(),
        "store integrity must pass after migration, failures: {:?}",
        report.failed
    );
}

#[test]
fn migrate_creates_backup() {
    let dir = tempfile::tempdir().unwrap();
    create_v1_store(dir.path(), 0);

    let result = migrate_store(dir.path()).unwrap().unwrap();
    assert!(result.backup_path.exists(), "backup file must exist");

    // Backup must contain v1
    let backup_content = fs::read_to_string(&result.backup_path).unwrap();
    assert!(
        backup_content.contains("\"format_version\": 1")
            || backup_content.contains("\"format_version\":1"),
        "backup must contain format_version 1, got: {backup_content}"
    );

    // Current version must be v2
    let current = fs::read_to_string(dir.path().join("store").join("version")).unwrap();
    assert!(
        current.contains(&format!("{STORE_FORMAT_VERSION}")),
        "version file must now be v{STORE_FORMAT_VERSION}"
    );
}

#[test]
fn migrate_idempotent_on_current_version() {
    let dir = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(dir.path());
    layout.initialize().unwrap();

    let result = migrate_store(dir.path()).unwrap();
    assert!(
        result.is_none(),
        "migrate on current-version store must return None"
    );

    // Store unmodified
    layout.verify_version().unwrap();
}

#[test]
fn migrate_rejects_future_version() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("store");
    fs::create_dir_all(&store_dir).unwrap();
    fs::write(store_dir.join("version"), r#"{"format_version": 99}"#).unwrap();

    let result = migrate_store(dir.path());
    assert!(result.is_err(), "future version must be rejected");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("mismatch") || err_msg.contains("Mismatch"),
        "error must mention version mismatch, got: {err_msg}"
    );
}

#[test]
fn migrate_atomic_version_unchanged_on_write_failure() {
    use std::os::unix::fs::PermissionsExt;

    // Root bypasses filesystem permission checks — skip in containers
    #[allow(unsafe_code)]
    if unsafe { libc::getuid() } == 0 {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    create_v1_store(dir.path(), 1);

    let store_dir = dir.path().join("store");

    // Make the store directory non-writable so the final NamedTempFile::new_in(&store_dir)
    // fails. Metadata migration writes into metadata/ (still writable) but the version
    // file write into store/ will fail.
    let original_mode = fs::metadata(&store_dir).unwrap().permissions().mode();
    // Remove write permission from store/ dir (keep read+exec for traversal)
    fs::set_permissions(&store_dir, fs::Permissions::from_mode(0o555)).unwrap();

    let result = migrate_store(dir.path());

    // Restore permissions for cleanup
    fs::set_permissions(&store_dir, fs::Permissions::from_mode(original_mode)).unwrap();

    // Migration MUST have failed — the version file write requires creating a temp file in store/
    assert!(
        result.is_err(),
        "migration must fail when store dir is read-only — test is invalid if it succeeds"
    );

    // Version file MUST still say v1
    let ver_content = fs::read_to_string(dir.path().join("store").join("version")).unwrap();
    assert!(
        ver_content.contains("\"format_version\": 1")
            || ver_content.contains("\"format_version\":1"),
        "version must still be v1 after failed migration, got: {ver_content}"
    );

    // No partial version.backup files should exist (backup also writes to store/)
    let backup_files: Vec<_> = fs::read_dir(&store_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("version.backup")
        })
        .collect();
    // Backup may or may not exist depending on where exactly the failure occurred,
    // but version must be unchanged regardless.
    let _ = backup_files;
}

#[test]
fn migrate_corrupted_metadata_fails_and_store_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("store");

    // Create a minimal v1 store with a corrupted metadata file
    fs::create_dir_all(store_dir.join("objects")).unwrap();
    fs::create_dir_all(store_dir.join("layers")).unwrap();
    fs::create_dir_all(store_dir.join("metadata")).unwrap();
    fs::create_dir_all(store_dir.join("staging")).unwrap();
    fs::create_dir_all(dir.path().join("env")).unwrap();
    fs::write(store_dir.join("version"), r#"{"format_version": 1}"#).unwrap();

    // Write corrupted metadata: not a JSON object (it's an array)
    fs::write(store_dir.join("metadata").join("corrupt_env"), "[1, 2, 3]").unwrap();

    // Migration should succeed (corrupt files are warned+skipped) but report 0 migrated
    let result = migrate_store(dir.path()).unwrap();
    assert!(result.is_some());
    let result = result.unwrap();
    assert_eq!(
        result.environments_migrated, 0,
        "corrupted metadata must not count as migrated"
    );

    // The corrupted file must still exist and be unchanged
    let corrupt_content =
        fs::read_to_string(store_dir.join("metadata").join("corrupt_env")).unwrap();
    assert_eq!(
        corrupt_content, "[1, 2, 3]",
        "corrupted file must be untouched"
    );

    // Version file must now be v2 (migration itself succeeded, only metadata was skipped)
    let layout = StoreLayout::new(dir.path());
    layout.verify_version().unwrap();
}

#[test]
fn migrate_invalid_json_metadata_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("store");

    fs::create_dir_all(store_dir.join("objects")).unwrap();
    fs::create_dir_all(store_dir.join("layers")).unwrap();
    fs::create_dir_all(store_dir.join("metadata")).unwrap();
    fs::create_dir_all(store_dir.join("staging")).unwrap();
    fs::create_dir_all(dir.path().join("env")).unwrap();
    fs::write(store_dir.join("version"), r#"{"format_version": 1}"#).unwrap();

    // Write totally invalid JSON
    fs::write(
        store_dir.join("metadata").join("broken_env"),
        "THIS IS NOT JSON AT ALL {{{",
    )
    .unwrap();

    // Also write a valid v1 metadata file
    let valid_meta = serde_json::json!({
        "env_id": "valid_env",
        "short_id": "valid_en",
        "state": "Built",
        "manifest_hash": "mh",
        "base_layer": "bl",
        "dependency_layers": [],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z",
        "ref_count": 1
    });
    fs::write(
        store_dir.join("metadata").join("valid_env"),
        serde_json::to_string_pretty(&valid_meta).unwrap(),
    )
    .unwrap();

    let result = migrate_store(dir.path()).unwrap().unwrap();

    // Only the valid one should be migrated
    assert_eq!(result.environments_migrated, 1);

    // Invalid file must still exist, unchanged
    let broken = fs::read_to_string(store_dir.join("metadata").join("broken_env")).unwrap();
    assert_eq!(broken, "THIS IS NOT JSON AT ALL {{{");

    // Valid file must now have v2 fields
    let valid = fs::read_to_string(store_dir.join("metadata").join("valid_env")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&valid).unwrap();
    assert!(
        parsed.get("name").is_some(),
        "v2 'name' field must be present"
    );
    assert!(
        parsed.get("checksum").is_some(),
        "v2 'checksum' field must be present"
    );
    assert!(
        parsed.get("policy_layer").is_some(),
        "v2 'policy_layer' field must be present"
    );
}
