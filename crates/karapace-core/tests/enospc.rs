//! IG-M4: True disk-full (ENOSPC) simulation tests.
//!
//! These tests mount a tiny tmpfs to trigger real ENOSPC conditions.
//! They require root (or equivalent) to mount tmpfs, so they are ignored
//! by default and run in CI with: `sudo -E cargo test --test enospc -- --ignored`

use std::path::{Path, PathBuf};
use std::process::Command;

/// Mount a tmpfs of the given size (in KB) at `path`.
/// Returns true if successful. Requires root.
fn mount_tiny_tmpfs(path: &Path, size_kb: u64) -> bool {
    std::fs::create_dir_all(path).unwrap();
    let status = Command::new("mount")
        .args(["-t", "tmpfs", "-o", &format!("size={size_kb}k"), "tmpfs"])
        .arg(path)
        .status();
    matches!(status, Ok(s) if s.success())
}

/// Unmount the tmpfs at `path`.
fn unmount(path: &Path) {
    let _ = Command::new("umount").arg(path).status();
}

/// RAII guard that unmounts on drop.
struct TmpfsGuard {
    path: PathBuf,
}

impl TmpfsGuard {
    fn mount(path: &Path, size_kb: u64) -> Option<Self> {
        if mount_tiny_tmpfs(path, size_kb) {
            Some(Self {
                path: path.to_path_buf(),
            })
        } else {
            None
        }
    }
}

impl Drop for TmpfsGuard {
    fn drop(&mut self) {
        unmount(&self.path);
    }
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_object_put_returns_io_error() {
    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("tiny");
    let _guard = TmpfsGuard::mount(&mount_point, 64)
        .expect("failed to mount tmpfs — are you running as root?");

    let layout = karapace_store::StoreLayout::new(&mount_point);
    layout.initialize().unwrap();
    let obj_store = karapace_store::ObjectStore::new(layout);

    // Write objects until we hit ENOSPC
    let mut hit_error = false;
    for i in 0..10_000 {
        let data = format!("object-data-{i}-padding-to-fill-disk-quickly").repeat(10);
        match obj_store.put(data.as_bytes()) {
            Ok(_) => {}
            Err(e) => {
                let msg = format!("{e}");
                eprintln!("ENOSPC triggered at object {i}: {msg}");
                hit_error = true;
                // Must be an Io error, never a panic
                assert!(
                    matches!(e, karapace_store::StoreError::Io(_)),
                    "expected StoreError::Io, got: {e}"
                );
                break;
            }
        }
    }
    assert!(
        hit_error,
        "should have hit ENOSPC within 10000 objects on 64KB tmpfs"
    );
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_build_fails_cleanly() {
    use karapace_core::Engine;
    use karapace_store::StoreLayout;

    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("tiny");
    let _guard = TmpfsGuard::mount(&mount_point, 64)
        .expect("failed to mount tmpfs — are you running as root?");

    let layout = StoreLayout::new(&mount_point);
    layout.initialize().unwrap();

    let manifest = r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["curl", "git", "vim", "wget", "htop"]
"#;

    let manifest_path = mount_point.join("karapace.toml");
    std::fs::write(&manifest_path, manifest).unwrap();

    let engine = Engine::new(&mount_point);
    let result = engine.build(&manifest_path);

    // Build must fail (ENOSPC), not panic
    assert!(result.is_err(), "build on 64KB tmpfs must fail");

    // WAL must have no incomplete entries after error cleanup
    let wal = karapace_store::WriteAheadLog::new(&layout);
    let incomplete = wal.list_incomplete().unwrap_or_default();
    assert!(
        incomplete.is_empty(),
        "WAL must be clean after failed build, found {} incomplete entries",
        incomplete.len()
    );

    // No orphaned env directories
    let env_dir = layout.env_dir();
    if env_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&env_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            entries.is_empty(),
            "no orphaned env dirs after failed build, found: {:?}",
            entries
                .iter()
                .map(std::fs::DirEntry::file_name)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_wal_write_fails() {
    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("tiny");
    let _guard = TmpfsGuard::mount(&mount_point, 4)
        .expect("failed to mount tmpfs — are you running as root?");

    // Create minimal store structure
    let store_dir = mount_point.join("store");
    std::fs::create_dir_all(store_dir.join("wal")).unwrap();

    // Fill the tmpfs with dummy data until nearly full
    for i in 0..100 {
        let path = mount_point.join(format!("filler_{i}"));
        if std::fs::write(&path, [0u8; 512]).is_err() {
            break;
        }
    }

    let layout = karapace_store::StoreLayout::new(&mount_point);
    let wal = karapace_store::WriteAheadLog::new(&layout);

    // WAL begin should fail due to ENOSPC
    let result = wal.begin(karapace_store::WalOpKind::Build, "test_env");
    assert!(
        result.is_err(),
        "WAL begin on full disk must fail, not panic"
    );
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_commit_fails_cleanly() {
    use karapace_core::Engine;
    use karapace_store::StoreLayout;

    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("medium");
    // 256KB — enough for build, but commit with large upper should fail
    let _guard = TmpfsGuard::mount(&mount_point, 256)
        .expect("failed to mount tmpfs — are you running as root?");

    let layout = StoreLayout::new(&mount_point);
    layout.initialize().unwrap();

    let manifest = r#"
manifest_version = 1
[base]
image = "rolling"
"#;
    let manifest_path = mount_point.join("karapace.toml");
    std::fs::write(&manifest_path, manifest).unwrap();

    let engine = Engine::new(&mount_point);

    // Build must succeed on 256KB — if it doesn't, the test setup is wrong
    let build_result = engine.build(&manifest_path);
    assert!(
        build_result.is_ok(),
        "build on 256KB tmpfs must succeed for commit test to be valid: {:?}",
        build_result.err()
    );
    let env_id = build_result.unwrap().identity.env_id;

    // Write enough data to the upper dir to fill the disk
    let upper = layout.upper_dir(&env_id);
    std::fs::create_dir_all(&upper).unwrap();
    let mut filled = false;
    for i in 0..500 {
        let path = upper.join(format!("bigfile_{i}"));
        if std::fs::write(&path, [0xAB; 1024]).is_err() {
            filled = true;
            break;
        }
    }
    assert!(
        filled,
        "must fill disk before commit — 256KB tmpfs should be exhaustible"
    );

    // Commit MUST fail due to ENOSPC during layer packing
    let commit_result = engine.commit(&env_id);
    assert!(
        commit_result.is_err(),
        "commit on full disk MUST fail — test is invalid if it succeeds"
    );

    // Verify env state is still Built (not corrupted)
    let meta = karapace_store::MetadataStore::new(layout.clone())
        .get(&env_id)
        .unwrap();
    assert_eq!(
        meta.state,
        karapace_store::EnvState::Built,
        "env state must remain Built after failed commit"
    );

    // No partial commit artifacts
    let layers_dir = layout.layers_dir();
    if layers_dir.exists() {
        let staging = layout.staging_dir();
        if staging.exists() {
            let staging_entries: Vec<_> = std::fs::read_dir(&staging)
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            assert!(
                staging_entries.is_empty(),
                "no partial staging artifacts after failed commit: {:?}",
                staging_entries
                    .iter()
                    .map(std::fs::DirEntry::file_name)
                    .collect::<Vec<_>>()
            );
        }
    }
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_recovery_after_freeing_space() {
    use karapace_store::{ObjectStore, StoreLayout};

    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("recov");
    let _guard = TmpfsGuard::mount(&mount_point, 128)
        .expect("failed to mount tmpfs — are you running as root?");

    let layout = StoreLayout::new(&mount_point);
    layout.initialize().unwrap();
    let obj_store = ObjectStore::new(layout);

    // Fill with objects
    let mut hashes = Vec::new();
    for i in 0..500 {
        let data = format!("fill-data-{i}").repeat(5);
        match obj_store.put(data.as_bytes()) {
            Ok(h) => hashes.push(h),
            Err(_) => break,
        }
    }
    assert!(!hashes.is_empty(), "should have stored at least one object");

    // Attempt one more write — MUST fail (disk full)
    let big_data = [0xCD; 4096];
    let err_result = obj_store.put(&big_data);
    assert!(
        err_result.is_err(),
        "128KB tmpfs must be full after filling — test setup invalid if write succeeds"
    );

    // Delete half the objects to free space
    let objects_dir = mount_point.join("store").join("objects");
    let half = hashes.len() / 2;
    for h in &hashes[..half] {
        let _ = std::fs::remove_file(objects_dir.join(h));
    }

    // Now writes should succeed again
    let recovery_result = obj_store.put(b"recovery data after freeing space");
    assert!(
        recovery_result.is_ok(),
        "write must succeed after freeing space: {:?}",
        recovery_result.err()
    );
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_layer_put_fails_cleanly() {
    use karapace_store::{LayerKind, LayerManifest, LayerStore, StoreLayout};

    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("tiny_layer");
    let _guard = TmpfsGuard::mount(&mount_point, 8)
        .expect("failed to mount tmpfs — are you running as root?");

    let layout = StoreLayout::new(&mount_point);
    layout.initialize().unwrap();

    // Fill the tmpfs
    for i in 0..200 {
        let path = mount_point.join(format!("filler_{i}"));
        if std::fs::write(&path, [0u8; 256]).is_err() {
            break;
        }
    }

    let layer_store = LayerStore::new(layout.clone());
    let manifest = LayerManifest {
        hash: "test_layer_enospc".to_owned(),
        kind: LayerKind::Base,
        parent: None,
        object_refs: vec!["obj1".to_owned(), "obj2".to_owned()],
        read_only: true,
        tar_hash: String::new(),
    };

    let result = layer_store.put(&manifest);
    assert!(
        result.is_err(),
        "layer put on full disk MUST fail, not succeed"
    );
    assert!(
        matches!(
            result.as_ref().unwrap_err(),
            karapace_store::StoreError::Io(_)
        ),
        "expected StoreError::Io, got: {:?}",
        result.unwrap_err()
    );
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_metadata_put_fails_cleanly() {
    use karapace_store::{EnvMetadata, EnvState, MetadataStore, StoreLayout};

    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("tiny_meta");
    let _guard = TmpfsGuard::mount(&mount_point, 8)
        .expect("failed to mount tmpfs — are you running as root?");

    let layout = StoreLayout::new(&mount_point);
    layout.initialize().unwrap();

    // Fill the tmpfs
    for i in 0..200 {
        let path = mount_point.join(format!("filler_{i}"));
        if std::fs::write(&path, [0u8; 256]).is_err() {
            break;
        }
    }

    let meta_store = MetadataStore::new(layout);
    let meta = EnvMetadata {
        env_id: "enospc_test_env".into(),
        short_id: "enospc_test".into(),
        name: Some("enospc-test".to_owned()),
        state: EnvState::Built,
        base_layer: "fake_layer".into(),
        dependency_layers: vec![],
        policy_layer: None,
        manifest_hash: "fake_hash".into(),
        ref_count: 1,
        created_at: "2025-01-01T00:00:00Z".to_owned(),
        updated_at: "2025-01-01T00:00:00Z".to_owned(),
        checksum: None,
    };

    let result = meta_store.put(&meta);
    assert!(
        result.is_err(),
        "metadata put on full disk MUST fail, not succeed"
    );
    assert!(
        matches!(
            result.as_ref().unwrap_err(),
            karapace_store::StoreError::Io(_)
        ),
        "expected StoreError::Io, got: {:?}",
        result.unwrap_err()
    );
}

#[test]
#[ignore = "requires root for tmpfs mount"]
fn enospc_version_file_write_fails() {
    use karapace_store::StoreLayout;

    let base = tempfile::tempdir().unwrap();
    let mount_point = base.path().join("tiny_ver");
    // Very small: just enough for dirs but not for version file after fill
    let _guard = TmpfsGuard::mount(&mount_point, 4)
        .expect("failed to mount tmpfs — are you running as root?");

    // Manually create minimal dirs (initialize writes version file, we want it to fail)
    let store_dir = mount_point.join("store");
    std::fs::create_dir_all(store_dir.join("objects")).unwrap();
    std::fs::create_dir_all(store_dir.join("layers")).unwrap();
    std::fs::create_dir_all(store_dir.join("metadata")).unwrap();
    std::fs::create_dir_all(store_dir.join("staging")).unwrap();
    std::fs::create_dir_all(mount_point.join("env")).unwrap();

    // Fill the tmpfs completely
    for i in 0..200 {
        let path = mount_point.join(format!("filler_{i}"));
        if std::fs::write(&path, [0u8; 256]).is_err() {
            break;
        }
    }

    // Now try to initialize (which writes the version file) — must fail
    let layout = StoreLayout::new(&mount_point);
    let result = layout.initialize();
    assert!(
        result.is_err(),
        "StoreLayout::initialize on full disk MUST fail when writing version file"
    );
}
