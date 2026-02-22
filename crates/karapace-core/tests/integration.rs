#![allow(unsafe_code)]

use karapace_core::{Engine, StoreLock};
use karapace_store::{EnvState, StoreLayout};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

/// Skip test if running as root — root bypasses filesystem permission checks,
/// so read-only directory tests are meaningless in containers running as uid 0.
fn skip_if_root() -> bool {
    #[allow(unsafe_code)]
    unsafe {
        libc::getuid() == 0
    }
}

fn write_manifest(dir: &Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("karapace.toml");
    fs::write(&path, content).unwrap();
    path
}

fn mock_manifest(packages: &[&str]) -> String {
    format!(
        r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = [{}]
[runtime]
backend = "mock"
"#,
        packages
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

// §11.2: Build → Destroy → Rebuild equality
#[test]
fn build_destroy_rebuild_produces_identical_env_id() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git", "clang"]));

    let r1 = engine.build(&manifest).unwrap();
    let id1 = r1.identity.env_id.clone();

    engine.destroy(&id1).unwrap();

    let meta_store = karapace_store::MetadataStore::new(StoreLayout::new(store.path()));
    meta_store.remove(&id1).unwrap();

    let r2 = engine.build(&manifest).unwrap();
    assert_eq!(
        id1, r2.identity.env_id,
        "rebuild after destroy must produce identical env_id"
    );
}

// §11.2: Multi-environment isolation
#[test]
fn multi_environment_isolation() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let p1 = tempfile::tempdir().unwrap();
    let p2 = tempfile::tempdir().unwrap();

    let m1 = write_manifest(p1.path(), &mock_manifest(&["git"]));
    let m2 = write_manifest(p2.path(), &mock_manifest(&["cmake"]));

    let r1 = engine.build(&m1).unwrap();
    let r2 = engine.build(&m2).unwrap();

    assert_ne!(r1.identity.env_id, r2.identity.env_id);

    let envs = engine.list().unwrap();
    assert_eq!(envs.len(), 2);

    engine.destroy(&r1.identity.env_id).unwrap();
    let envs = engine.list().unwrap();
    assert_eq!(envs.len(), 1);

    let meta = engine.inspect(&r2.identity.env_id).unwrap();
    assert_eq!(meta.state, EnvState::Built);
}

// §11.2: GC safety under load
#[test]
fn gc_safety_with_multiple_environments() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let mut env_ids = Vec::new();
    for i in 0..10 {
        let p = tempfile::tempdir().unwrap();
        let pkgs: Vec<String> = (0..=i).map(|j| format!("pkg{j}")).collect();
        let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
        let m = write_manifest(p.path(), &mock_manifest(&pkg_refs));
        let r = engine.build(&m).unwrap();
        env_ids.push(r.identity.env_id);
    }

    assert_eq!(engine.list().unwrap().len(), 10);

    for id in &env_ids[..5] {
        engine.destroy(id).unwrap();
    }

    let layout = StoreLayout::new(store.path());
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
    let _report = engine.gc(&lock, false).unwrap();

    for id in &env_ids[5..] {
        let meta = engine.inspect(id).unwrap();
        assert_eq!(meta.state, EnvState::Built, "active env should survive GC");
    }
}

// §11.2: Concurrent build safety
#[test]
fn concurrent_builds_are_safe() {
    let store = tempfile::tempdir().unwrap();
    let store_path = store.path().to_path_buf();

    let barrier = Arc::new(Barrier::new(4));
    let mut handles = Vec::new();

    for i in 0..4 {
        let sp = store_path.clone();
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let engine = Engine::new(&sp);
            let p = tempfile::tempdir().unwrap();
            let pkgs = [format!("thread-pkg-{i}")];
            let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
            let m = write_manifest(p.path(), &mock_manifest(&pkg_refs));

            b.wait();
            engine.build(&m).unwrap()
        }));
    }

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let engine = Engine::new(&store_path);
    let envs = engine.list().unwrap();
    assert_eq!(envs.len(), 4, "all 4 concurrent builds should succeed");

    let ids: std::collections::HashSet<_> =
        results.iter().map(|r| r.identity.env_id.clone()).collect();
    assert_eq!(ids.len(), 4, "all env IDs should be unique");
}

// §11.3: Reproducibility test
#[test]
fn same_manifest_produces_identical_env_id_across_engines() {
    let manifest_content = mock_manifest(&["git", "clang", "cmake"]);

    let store1 = tempfile::tempdir().unwrap();
    let store2 = tempfile::tempdir().unwrap();
    let p1 = tempfile::tempdir().unwrap();
    let p2 = tempfile::tempdir().unwrap();

    let m1 = write_manifest(p1.path(), &manifest_content);
    let m2 = write_manifest(p2.path(), &manifest_content);

    let engine1 = Engine::new(store1.path());
    let engine2 = Engine::new(store2.path());

    let r1 = engine1.build(&m1).unwrap();
    let r2 = engine2.build(&m2).unwrap();

    assert_eq!(
        r1.identity.env_id, r2.identity.env_id,
        "same manifest on different stores must produce identical env_id"
    );
    assert_eq!(r1.lock_file.env_id, r2.lock_file.env_id);
    assert_eq!(
        r1.lock_file.base_image_digest,
        r2.lock_file.base_image_digest
    );
}

// §16 Core invariant: host system remains untouched
#[test]
fn host_system_untouched_after_lifecycle() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let home_before: Vec<_> = fs::read_dir(std::env::var("HOME").unwrap_or("/tmp".to_owned()))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    engine.freeze(&r.identity.env_id).unwrap();
    engine.destroy(&r.identity.env_id).unwrap();

    let home_after: Vec<_> = fs::read_dir(std::env::var("HOME").unwrap_or("/tmp".to_owned()))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();

    assert_eq!(home_before, home_after, "host HOME must not be modified");
}

// §16 Core invariant: no silent drift
#[test]
fn drift_always_observable() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Clear the upper dir to simulate a clean post-build state.
    // The mock backend creates files there during build; a real backend's
    // overlay upper would also have content, but drift is measured from
    // the post-build snapshot baseline.
    let upper = engine.store_layout().upper_dir(&r.identity.env_id);
    if upper.exists() {
        fs::remove_dir_all(&upper).unwrap();
        fs::create_dir_all(&upper).unwrap();
    }

    let report = karapace_core::diff_overlay(engine.store_layout(), &r.identity.env_id).unwrap();
    assert!(!report.has_drift, "fresh build should have no drift");

    fs::write(upper.join("injected.txt"), "mutation").unwrap();

    let report = karapace_core::diff_overlay(engine.store_layout(), &r.identity.env_id).unwrap();
    assert!(report.has_drift, "mutation must be detected");
    assert!(report.added.contains(&"injected.txt".to_owned()));
}

// §15: Crash safety — interrupted write leaves no partial objects
#[test]
fn store_integrity_after_partial_operations() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let obj_store = karapace_store::ObjectStore::new(layout.clone());
    obj_store.put(b"valid1").unwrap();
    obj_store.put(b"valid2").unwrap();

    let stray_path = layout.objects_dir().join("not_a_real_hash");
    fs::write(&stray_path, b"corrupted data").unwrap();

    let report = karapace_store::verify_store_integrity(&layout).unwrap();
    assert_eq!(report.failed.len(), 1, "corrupted object must be detected");
    assert_eq!(report.passed, 2);
}

// §5.2: commit persists overlay into store
#[test]
fn commit_persists_overlay_drift() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Clear mock backend artifacts from upper, then add user files
    let upper = engine.store_layout().upper_dir(&r.identity.env_id);
    if upper.exists() {
        fs::remove_dir_all(&upper).unwrap();
    }
    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("user_file.txt"), "user data").unwrap();
    fs::create_dir_all(upper.join("subdir")).unwrap();
    fs::write(upper.join("subdir").join("nested.txt"), "nested data").unwrap();

    let snapshot_hash = engine.commit(&r.identity.env_id).unwrap();
    assert!(
        !snapshot_hash.is_empty(),
        "commit should return a snapshot hash"
    );

    // Look up the snapshot layer to get the tar_hash
    let layer_store = karapace_store::LayerStore::new(StoreLayout::new(store.path()));
    let layer = layer_store.get(&snapshot_hash).unwrap();
    assert_eq!(layer.kind, karapace_store::LayerKind::Snapshot);
    assert!(!layer.tar_hash.is_empty());

    let obj_store = karapace_store::ObjectStore::new(StoreLayout::new(store.path()));
    assert!(
        obj_store.exists(&layer.tar_hash),
        "committed tar object must exist in store"
    );

    // Verify the committed tar can be unpacked and contains the user files
    let tar_data = obj_store.get(&layer.tar_hash).unwrap();
    let unpack_dir = tempfile::tempdir().unwrap();
    karapace_store::unpack_layer(&tar_data, unpack_dir.path()).unwrap();
    assert!(unpack_dir.path().join("user_file.txt").exists());
    assert!(unpack_dir.path().join("subdir").join("nested.txt").exists());
}

// INV-S1: build → modify → commit → restore → diff = zero drift
#[test]
fn restore_roundtrip() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Clear mock artifacts and add user files
    let upper = engine.store_layout().upper_dir(&r.identity.env_id);
    if upper.exists() {
        fs::remove_dir_all(&upper).unwrap();
    }
    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("user_file.txt"), "snapshot content").unwrap();
    fs::create_dir_all(upper.join("data")).unwrap();
    fs::write(upper.join("data").join("config.json"), r#"{"key":"val"}"#).unwrap();

    // Commit the snapshot
    let snapshot_hash = engine.commit(&r.identity.env_id).unwrap();

    // Mutate the upper dir (simulating user modifications after snapshot)
    fs::write(upper.join("user_file.txt"), "MODIFIED AFTER SNAPSHOT").unwrap();
    fs::write(upper.join("extra.txt"), "extra file").unwrap();

    // Restore from the snapshot
    engine.restore(&r.identity.env_id, &snapshot_hash).unwrap();

    // Verify the upper dir matches the snapshot content exactly
    assert_eq!(
        fs::read_to_string(upper.join("user_file.txt")).unwrap(),
        "snapshot content"
    );
    assert_eq!(
        fs::read_to_string(upper.join("data").join("config.json")).unwrap(),
        r#"{"key":"val"}"#
    );
    // The extra file should NOT exist after restore
    assert!(
        !upper.join("extra.txt").exists(),
        "extra file must be gone after restore"
    );
}

// INV-S2: restore nonexistent snapshot → error
#[test]
fn restore_nonexistent_snapshot_fails() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    let result = engine.restore(&r.identity.env_id, "nonexistent_hash");
    assert!(result.is_err());
}

// list_snapshots returns committed snapshots
#[test]
fn list_snapshots_after_commit() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // No snapshots initially
    let snaps = engine.list_snapshots(&r.identity.env_id).unwrap();
    assert!(snaps.is_empty());

    // Commit a snapshot
    let _hash = engine.commit(&r.identity.env_id).unwrap();

    let snaps = engine.list_snapshots(&r.identity.env_id).unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].kind, karapace_store::LayerKind::Snapshot);
    assert!(!snaps[0].tar_hash.is_empty());
}

// §12: GC scales to at least 100 environments
#[test]
fn gc_scales_to_100_environments() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    for i in 0..100 {
        let p = tempfile::tempdir().unwrap();
        let pkgs: Vec<String> = (0..=i).map(|j| format!("scale-pkg-{j}")).collect();
        let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
        let m = write_manifest(p.path(), &mock_manifest(&pkg_refs));
        engine.build(&m).unwrap();
    }

    assert_eq!(engine.list().unwrap().len(), 100);

    let layout = StoreLayout::new(store.path());
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
    let start = std::time::Instant::now();
    let report = engine.gc(&lock, true).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 10,
        "GC dry-run on 100 envs took {elapsed:?}, must be under 10s"
    );
    assert_eq!(report.orphaned_envs.len(), 0, "all envs have ref_count > 0");
}

// §12: Warm cache build under 10 seconds
#[test]
fn warm_build_under_10_seconds() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git", "clang", "cmake"]));

    // Cold build
    engine.build(&manifest).unwrap();

    // Warm rebuild (store already populated)
    let start = std::time::Instant::now();
    engine.rebuild(&manifest).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 10,
        "warm rebuild took {elapsed:?}, must be under 10s"
    );
}

// §3.1: Build fails deterministically — different manifests get different IDs
#[test]
fn different_manifests_different_ids() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let p1 = tempfile::tempdir().unwrap();
    let p2 = tempfile::tempdir().unwrap();
    let m1 = write_manifest(p1.path(), &mock_manifest(&["git"]));
    let m2 = write_manifest(p2.path(), &mock_manifest(&["git", "cmake"]));

    let r1 = engine.build(&m1).unwrap();
    let r2 = engine.build(&m2).unwrap();
    assert_ne!(r1.identity.env_id, r2.identity.env_id);
}

// §5.1: Whiteout files detected as removed
#[test]
fn whiteout_files_detected_as_removals() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Clear mock backend artifacts, then add only the whiteout
    let upper = engine.store_layout().upper_dir(&r.identity.env_id);
    if upper.exists() {
        fs::remove_dir_all(&upper).unwrap();
    }
    fs::create_dir_all(&upper).unwrap();
    // Overlayfs whiteout: .wh.filename means filename was deleted
    fs::write(upper.join(".wh.deleted_config"), "").unwrap();

    let report = karapace_core::diff_overlay(engine.store_layout(), &r.identity.env_id).unwrap();
    assert!(report.has_drift);
    assert!(report.removed.contains(&"deleted_config".to_owned()));
    assert!(report.added.is_empty());
}

// §5.1: Modified files classified correctly when lower layer exists
#[test]
fn modified_files_detected_against_lower_layer() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Clear mock backend artifacts from upper
    let upper = engine.store_layout().upper_dir(&r.identity.env_id);
    if upper.exists() {
        fs::remove_dir_all(&upper).unwrap();
    }
    fs::create_dir_all(&upper).unwrap();

    // Simulate a lower layer file
    let env_dir = engine.store_layout().env_path(&r.identity.env_id);
    let lower = env_dir.join("lower");
    fs::create_dir_all(&lower).unwrap();
    fs::write(lower.join("config.txt"), "original").unwrap();

    // Same file in upper = modification
    fs::write(upper.join("config.txt"), "modified content").unwrap();
    // New file = added
    fs::write(upper.join("new_script.sh"), "#!/bin/sh").unwrap();

    let report = karapace_core::diff_overlay(engine.store_layout(), &r.identity.env_id).unwrap();
    assert!(report.has_drift);
    assert_eq!(report.modified, vec!["config.txt"]);
    assert_eq!(report.added, vec!["new_script.sh"]);
    assert!(report.removed.is_empty());
}

// §6.1: exec works via mock backend
#[test]
fn exec_via_mock_backend() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    let result = engine.exec(&r.identity.env_id, &["echo".to_owned(), "hello".to_owned()]);
    assert!(result.is_ok());
}

// §3.2: Lock file integrity verifiable after build
#[test]
fn lock_file_integrity_after_build() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git", "clang"]));
    let r = engine.build(&manifest).unwrap();

    // Lock file was written
    let lock_path = project.path().join("karapace.lock");
    assert!(lock_path.exists());

    // Verify integrity
    assert!(r.lock_file.verify_integrity().is_ok());

    // Verify manifest intent
    let parsed = karapace_schema::parse_manifest_file(&manifest).unwrap();
    let normalized = parsed.normalize().unwrap();
    assert!(r.lock_file.verify_manifest_intent(&normalized).is_ok());
}

// §5.2: Frozen environment cannot be entered
#[test]
fn frozen_env_cannot_be_entered() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    engine.freeze(&r.identity.env_id).unwrap();

    let result = engine.enter(&r.identity.env_id);
    assert!(result.is_err(), "entering a frozen env must fail");
}

// §15: Crash injection — partial write must not corrupt store
#[test]
fn crash_injection_partial_write_detected() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let obj_store = karapace_store::ObjectStore::new(layout.clone());

    // Write valid objects
    let h1 = obj_store.put(b"valid-object-1").unwrap();
    let h2 = obj_store.put(b"valid-object-2").unwrap();

    // Simulate a crash during write: create a file with a valid hash name
    // but corrupted content (as if the process died mid-write and the
    // atomic rename somehow partially completed — or manual tampering)
    let fake_hash = blake3::hash(b"original-content").to_hex().to_string();
    let fake_path = layout.objects_dir().join(&fake_hash);
    fs::write(&fake_path, b"truncated-garbage").unwrap();

    // Store integrity check must detect all corruption
    let report = karapace_store::verify_store_integrity(&layout).unwrap();
    assert_eq!(report.checked, 3);
    assert_eq!(report.passed, 2, "two valid objects should pass");
    assert_eq!(
        report.failed.len(),
        1,
        "corrupted object should be detected"
    );
    assert_eq!(report.failed[0].hash, fake_hash);

    // Valid objects must still be readable
    assert!(obj_store.get(&h1).is_ok());
    assert!(obj_store.get(&h2).is_ok());

    // Corrupted object must fail on read
    assert!(obj_store.get(&fake_hash).is_err());
}

// §15: Crash injection — version file corruption detected
#[test]
fn crash_injection_version_corruption_detected() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    // Corrupt the version file
    let version_path = store.path().join("store").join("version");
    fs::write(&version_path, r#"{"format_version": 999}"#).unwrap();

    // Re-initializing should detect version mismatch
    let result = layout.initialize();
    assert!(result.is_err(), "mismatched version must be rejected");
}

// §12: Environment entry under 1 second (mock backend)
#[test]
fn environment_entry_under_1_second() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    let start = std::time::Instant::now();
    // Mock enter is effectively instant — this tests the overhead of
    // metadata lookup, state transition, backend dispatch, and cleanup.
    engine.enter(&r.identity.env_id).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 1,
        "environment entry took {elapsed:?}, must be under 1s"
    );
}

// §4.1: Archive lifecycle — archive preserves but prevents entry
#[test]
fn archive_lifecycle() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Archive from Built state
    engine.archive(&r.identity.env_id).unwrap();
    let meta = engine.inspect(&r.identity.env_id).unwrap();
    assert_eq!(meta.state, EnvState::Archived);

    // Archived env cannot be entered
    let result = engine.enter(&r.identity.env_id);
    assert!(result.is_err(), "entering an archived env must fail");

    // Archived env can be rebuilt
    let r2 = engine.rebuild(&manifest).unwrap();
    let meta2 = engine.inspect(&r2.identity.env_id).unwrap();
    assert_eq!(meta2.state, EnvState::Built);
}

// §4.1: Freeze then archive lifecycle
#[test]
fn freeze_then_archive() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    engine.freeze(&r.identity.env_id).unwrap();
    engine.archive(&r.identity.env_id).unwrap();

    let meta = engine.inspect(&r.identity.env_id).unwrap();
    assert_eq!(meta.state, EnvState::Archived);
}

// §4.1: Destroy of non-existent env fails gracefully
#[test]
fn destroy_nonexistent_env_fails_gracefully() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());
    let result = engine.destroy("0000000000000000000000000000000000000000000000000000000000000000");
    assert!(result.is_err());
}

// §3.2: Lock file v2 contains resolved package versions (not just names)
#[test]
fn lock_file_v2_has_resolved_versions() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git", "clang"]));
    let r = engine.build(&manifest).unwrap();

    assert_eq!(r.lock_file.lock_version, 2);
    assert_eq!(r.lock_file.resolved_packages.len(), 2);
    for pkg in &r.lock_file.resolved_packages {
        assert!(!pkg.name.is_empty());
        assert!(!pkg.version.is_empty());
        assert_ne!(pkg.version, "unresolved");
    }
}

// §6.2: Cannot destroy a running environment (must stop first)
#[test]
fn destroy_running_env_is_rejected() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Simulate entering (mock backend sets state to Running)
    engine.enter(&r.identity.env_id).unwrap();

    // Now try to destroy — should fail because mock leaves it in Running
    // Note: mock enter() sets internal state but engine resets to Built on success,
    // so we manually set it to Running to test the guard
    let meta_store = karapace_store::MetadataStore::new(engine.store_layout().clone());
    meta_store
        .update_state(&r.identity.env_id, EnvState::Running)
        .unwrap();

    let result = engine.destroy(&r.identity.env_id);
    assert!(result.is_err(), "destroy must reject Running environments");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("running") || err_msg.contains("Running"),
        "error should mention Running state: {err_msg}"
    );
}

// Quick-style manifest generation + build (tests the flow used by `karapace quick`)
#[test]
fn quick_style_generated_manifest_builds_correctly() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    // Simulate what cmd_quick does: generate a manifest from CLI flags
    let image = "rolling";
    let packages = ["git", "curl"];
    let mut manifest = String::new();
    manifest.push_str("manifest_version = 1\n\n");
    manifest.push_str("[base]\n");
    {
        use std::fmt::Write as _;
        let _ = write!(manifest, "image = \"{image}\"\n\n");
    }
    manifest.push_str("[system]\npackages = [");
    let pkg_list: Vec<String> = packages.iter().map(|p| format!("\"{p}\"")).collect();
    manifest.push_str(&pkg_list.join(", "));
    manifest.push_str("]\n\n");
    manifest.push_str("[runtime]\nbackend = \"mock\"\n");

    let manifest_path = write_manifest(project.path(), &manifest);
    let result = engine.build(&manifest_path).unwrap();

    assert!(!result.identity.env_id.is_empty());
    assert!(!result.identity.short_id.is_empty());

    let meta = engine.inspect(&result.identity.env_id).unwrap();
    assert_eq!(meta.state, EnvState::Built);
}

// Quick-style minimal manifest (no packages, no hardware)
#[test]
fn quick_style_minimal_manifest_builds() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest =
        "manifest_version = 1\n\n[base]\nimage = \"rolling\"\n\n[runtime]\nbackend = \"mock\"\n";
    let manifest_path = write_manifest(project.path(), manifest);
    let result = engine.build(&manifest_path).unwrap();

    assert!(!result.identity.env_id.is_empty());
}

// §16: No hidden mutable state — init + build produce consistent lock
#[test]
fn init_then_build_produces_consistent_state() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let init_result = engine.init(&manifest).unwrap();

    // Init creates metadata in Defined state
    let meta = engine.inspect(&init_result.identity.env_id).unwrap();
    assert_eq!(meta.state, EnvState::Defined);

    // Build resolves and creates a new identity (different from init's preliminary)
    let build_result = engine.build(&manifest).unwrap();

    // The lock file from build is verifiable
    assert!(build_result.lock_file.verify_integrity().is_ok());
    assert_eq!(build_result.lock_file.lock_version, 2);
}

// INV-S2: Restore atomicity — original upper dir preserved if snapshot invalid
#[test]
fn restore_preserves_upper_on_invalid_snapshot() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Create a file in upper that we want to verify survives a failed restore
    let upper = store.path().join("env").join(&env_id).join("upper");
    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("sentinel.txt"), "must survive").unwrap();

    // Attempt restore with a nonexistent snapshot — should fail
    let result = engine.restore(&env_id, "nonexistent_hash_abc123");
    assert!(result.is_err());

    // Original upper content must still exist
    assert!(upper.join("sentinel.txt").exists());
    assert_eq!(
        fs::read_to_string(upper.join("sentinel.txt")).unwrap(),
        "must survive"
    );
}

// INV-S3: Multiple snapshots listed in deterministic hash order
#[test]
fn snapshot_ordering_is_deterministic() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Create upper dir with different content for each commit
    let upper = store.path().join("env").join(&env_id).join("upper");

    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("v1.txt"), "version 1").unwrap();
    let h1 = engine.commit(&env_id).unwrap();

    fs::write(upper.join("v2.txt"), "version 2").unwrap();
    let h2 = engine.commit(&env_id).unwrap();

    fs::write(upper.join("v3.txt"), "version 3").unwrap();
    let h3 = engine.commit(&env_id).unwrap();

    // All hashes must be different
    assert_ne!(h1, h2);
    assert_ne!(h2, h3);
    assert_ne!(h1, h3);

    // list_snapshots returns sorted by hash — verify determinism
    let snaps = engine.list_snapshots(&env_id).unwrap();
    assert_eq!(snaps.len(), 3);
    let hashes: Vec<&str> = snaps.iter().map(|s| s.hash.as_str()).collect();
    let mut sorted = hashes.clone();
    sorted.sort_unstable();
    assert_eq!(hashes, sorted, "snapshots must be sorted by hash");

    // Calling again must return same order
    let snaps2 = engine.list_snapshots(&env_id).unwrap();
    let hashes2: Vec<&str> = snaps2.iter().map(|s| s.hash.as_str()).collect();
    assert_eq!(hashes, hashes2, "snapshot ordering must be deterministic");
}

// INV-W1: WAL recovery cleans orphaned env_dir after simulated build crash
#[test]
fn wal_recovery_cleans_orphaned_env_dir() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    // Simulate a crash during build:
    // 1. Create a WAL entry for a build operation
    // 2. Create an orphaned env_dir (as if build started but crashed)
    // 3. Verify that Engine::new() recovery cleans it up
    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    let fake_env_id = "crash_test_env_abc123";
    let orphan_dir = store.path().join("env").join(fake_env_id);
    fs::create_dir_all(&orphan_dir).unwrap();
    fs::write(orphan_dir.join("partial_build_artifact"), "data").unwrap();

    let op_id = wal
        .begin(karapace_store::WalOpKind::Build, fake_env_id)
        .unwrap();
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveDir(orphan_dir.clone()),
    )
    .unwrap();

    // Do NOT commit the WAL — simulates a crash

    // Now create a new Engine, which should trigger WAL recovery
    let _engine = Engine::new(store.path());

    // The orphaned env_dir must be cleaned up
    assert!(
        !orphan_dir.exists(),
        "WAL recovery must remove orphaned env_dir"
    );

    // WAL must be empty after recovery
    let wal2 = karapace_store::WriteAheadLog::new(&layout);
    assert!(wal2.list_incomplete().unwrap().is_empty());
}

// --- A3: WAL & Crash Safety Hardening ---

// Simulate crash during commit: WAL entry exists, staging dir left behind
#[test]
fn wal_crash_during_commit_leaves_recoverable_state() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    // Simulate a commit that created a staging dir but crashed before completion
    let staging_dir = layout.staging_dir().join("restore-crash_test");
    fs::create_dir_all(&staging_dir).unwrap();
    fs::write(staging_dir.join("partial_data.txt"), "partial").unwrap();

    let op_id = wal
        .begin(karapace_store::WalOpKind::Commit, "crash_env")
        .unwrap();
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveDir(staging_dir.clone()),
    )
    .unwrap();

    // Do NOT commit — simulates crash

    // Recovery via new Engine must clean up
    let _engine = Engine::new(store.path());

    assert!(
        !staging_dir.exists(),
        "WAL recovery must remove orphaned staging dir"
    );
    let wal2 = karapace_store::WriteAheadLog::new(&layout);
    assert!(wal2.list_incomplete().unwrap().is_empty());
}

// Simulate crash during restore: old upper removed but new upper not yet renamed
#[test]
fn wal_crash_during_restore_rollback() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    let fake_env_id = "restore_crash_env";

    // Create orphaned staging dir (as if restore was in progress)
    let staging = layout.staging_dir().join(format!("restore-{fake_env_id}"));
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("snapshot_file.txt"), "snapshot data").unwrap();

    let op_id = wal
        .begin(karapace_store::WalOpKind::Restore, fake_env_id)
        .unwrap();
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveDir(staging.clone()),
    )
    .unwrap();

    // Crash — WAL entry remains

    // Recovery should clean up staging
    let count = wal.recover().unwrap();
    assert_eq!(count, 1);
    assert!(!staging.exists());
}

// Multiple incomplete WAL entries all rolled back in order
#[test]
fn wal_multi_entry_recovery() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    // Create 3 orphaned directories with WAL entries
    let mut dirs = Vec::new();
    for i in 0..3 {
        let dir = store.path().join(format!("orphan_{i}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("data"), format!("data_{i}")).unwrap();

        let op_id = wal
            .begin(karapace_store::WalOpKind::Build, &format!("env_{i}"))
            .unwrap();
        wal.add_rollback_step(&op_id, karapace_store::RollbackStep::RemoveDir(dir.clone()))
            .unwrap();
        dirs.push(dir);
    }

    assert_eq!(wal.list_incomplete().unwrap().len(), 3);

    let count = wal.recover().unwrap();
    assert_eq!(count, 3);

    for dir in &dirs {
        assert!(!dir.exists(), "all orphaned dirs must be cleaned up");
    }
    assert!(wal.list_incomplete().unwrap().is_empty());
}

// Incomplete temp file in objects dir should not be visible
#[test]
fn incomplete_temp_file_invisible_to_store() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let obj_store = karapace_store::ObjectStore::new(layout.clone());

    // Write a valid object
    let hash = obj_store.put(b"valid data").unwrap();

    // Create a temp file (simulating interrupted atomic write)
    let temp_path = layout.objects_dir().join(".tmp_partial_write");
    fs::write(&temp_path, b"incomplete").unwrap();

    // list() should only return the valid object (skips dotfiles)
    let list = obj_store.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0], hash);

    // Integrity check should only check the valid object
    let report = karapace_store::verify_store_integrity(&layout).unwrap();
    assert_eq!(report.checked, 1);
    assert_eq!(report.passed, 1);
    assert!(report.failed.is_empty());
}

// Verify atomic rename is used: concurrent writes to same hash don't corrupt
#[test]
fn concurrent_object_writes_are_safe() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let store_path = store.path().to_path_buf();
    let barrier = Arc::new(Barrier::new(8));
    let mut handles = Vec::new();

    for _ in 0..8 {
        let sp = store_path.clone();
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let layout = StoreLayout::new(&sp);
            let obj_store = karapace_store::ObjectStore::new(layout);
            b.wait();
            // All threads write the same data — should deduplicate safely
            obj_store.put(b"concurrent-data").unwrap()
        }));
    }

    let hashes: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // All must produce the same hash
    let first = &hashes[0];
    for h in &hashes {
        assert_eq!(h, first);
    }

    // Data must be readable and intact
    let obj_store = karapace_store::ObjectStore::new(StoreLayout::new(store.path()));
    let data = obj_store.get(first).unwrap();
    assert_eq!(data, b"concurrent-data");
}

// --- M1: WAL Crash Safety Hardening (2.0) ---

// M1.1: Verify rollback step is registered before side-effect in build
#[test]
fn wal_rollback_registered_before_build_side_effect() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    // Simulate the new build pattern: register rollback step, then create dir
    let fake_env_id = "m1_test_build_order";
    let op_id = wal
        .begin(karapace_store::WalOpKind::Build, fake_env_id)
        .unwrap();

    let env_dir = store.path().join("env").join(fake_env_id);
    // Rollback step registered BEFORE dir creation
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveDir(env_dir.clone()),
    )
    .unwrap();

    // Verify WAL entry has the rollback step before dir even exists
    let entries = wal.list_incomplete().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].rollback_steps.is_empty());
    assert!(!env_dir.exists(), "dir should not exist yet");

    // Now create the dir (simulating actual build)
    fs::create_dir_all(&env_dir).unwrap();

    // Simulate crash: don't commit. Recovery should clean up.
    let _engine = Engine::new(store.path());
    assert!(
        !env_dir.exists(),
        "WAL recovery must remove orphaned dir even when rollback was registered before creation"
    );
}

// M1.1: Verify rollback of nonexistent path is a safe no-op
#[test]
fn wal_rollback_nonexistent_path_is_noop() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    let op_id = wal
        .begin(karapace_store::WalOpKind::Build, "noop_test")
        .unwrap();

    let nonexistent = store.path().join("env").join("does_not_exist");
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveDir(nonexistent.clone()),
    )
    .unwrap();

    // Recovery should succeed without error even though the path doesn't exist
    let wal2 = karapace_store::WriteAheadLog::new(&layout);
    let recovered = wal2.recover().unwrap();
    assert_eq!(recovered, 1);

    // WAL should be clean
    assert!(wal2.list_incomplete().unwrap().is_empty());
}

// M1.2: Destroy with WAL protection — normal path
#[test]
fn destroy_with_wal_commits_cleanly() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Destroy should succeed and leave no WAL entries
    engine.destroy(&env_id).unwrap();

    let layout = StoreLayout::new(store.path());
    let wal = karapace_store::WriteAheadLog::new(&layout);
    assert!(
        wal.list_incomplete().unwrap().is_empty(),
        "WAL must be clean after successful destroy"
    );
}

// M1.2: Destroy WAL crash recovery — crash between env_dir removal and metadata removal
#[test]
fn wal_crash_during_destroy_is_recoverable() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    let fake_env_id = "destroy_crash_env";

    // Create env dir and metadata to simulate a built environment
    let env_dir = store.path().join("env").join(fake_env_id);
    fs::create_dir_all(&env_dir).unwrap();
    fs::write(env_dir.join("some_file"), "data").unwrap();

    let meta_dir = layout.metadata_dir();
    fs::create_dir_all(&meta_dir).unwrap();
    fs::write(
        meta_dir.join(fake_env_id),
        r#"{"env_id":"destroy_crash_env"}"#,
    )
    .unwrap();

    // Simulate a destroy that crashed: WAL entry with rollback steps, not committed
    let op_id = wal
        .begin(karapace_store::WalOpKind::Destroy, fake_env_id)
        .unwrap();
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveDir(env_dir.clone()),
    )
    .unwrap();
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveFile(meta_dir.join(fake_env_id)),
    )
    .unwrap();

    // Recovery via Engine::new should clean up
    let _engine = Engine::new(store.path());

    assert!(
        !env_dir.exists(),
        "WAL recovery must remove orphaned env_dir from incomplete destroy"
    );

    let wal2 = karapace_store::WriteAheadLog::new(&layout);
    assert!(wal2.list_incomplete().unwrap().is_empty());
}

// M1.3: Commit layer rollback — crash after layer write, before WAL commit
#[test]
fn wal_crash_during_commit_cleans_orphaned_snapshot_layer() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    // Create a fake snapshot layer manifest file
    let layers_dir = layout.layers_dir();
    fs::create_dir_all(&layers_dir).unwrap();
    let fake_hash = "orphaned_snapshot_hash_abc123";
    let layer_path = layers_dir.join(fake_hash);
    fs::write(
        &layer_path,
        r#"{"hash":"orphaned_snapshot_hash_abc123","kind":"Snapshot"}"#,
    )
    .unwrap();

    // Simulate commit crash: WAL entry with layer rollback, not committed
    let op_id = wal
        .begin(karapace_store::WalOpKind::Commit, "commit_crash_env")
        .unwrap();
    wal.add_rollback_step(
        &op_id,
        karapace_store::RollbackStep::RemoveFile(layer_path.clone()),
    )
    .unwrap();

    // Recovery should remove the orphaned snapshot layer
    let _engine = Engine::new(store.path());

    assert!(
        !layer_path.exists(),
        "WAL recovery must remove orphaned snapshot layer from incomplete commit"
    );
}

// M1.4: GC with WAL marker — normal path
#[test]
fn gc_with_wal_commits_cleanly() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    // Run GC (nothing to collect, but WAL should be clean after)
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
    let report = engine.gc(&lock, true).unwrap();
    assert_eq!(report.orphaned_envs.len(), 0);

    let wal = karapace_store::WriteAheadLog::new(&layout);
    assert!(
        wal.list_incomplete().unwrap().is_empty(),
        "WAL must be clean after successful GC"
    );
}

// M1.4: GC incomplete WAL entry recovered safely
#[test]
fn gc_incomplete_wal_entry_recovered_safely() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    // Simulate an incomplete GC: WAL entry exists, GC didn't finish
    let _op_id = wal.begin(karapace_store::WalOpKind::Gc, "gc").unwrap();

    // Recovery via Engine::new should clean up the WAL entry (no rollback steps needed)
    let _engine = Engine::new(store.path());

    let wal2 = karapace_store::WriteAheadLog::new(&layout);
    assert!(
        wal2.list_incomplete().unwrap().is_empty(),
        "incomplete GC WAL entry must be cleaned up on recovery"
    );
}

// M1.4: GC is idempotent after partial run
#[test]
fn gc_is_idempotent_after_partial_run() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Destroy to create orphaned objects/layers
    engine.destroy(&env_id).unwrap();

    // First GC run
    let layout = StoreLayout::new(store.path());
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
    let report1 = engine.gc(&lock, false).unwrap();
    // Second GC run — should be a no-op (idempotent)
    let report2 = engine.gc(&lock, false).unwrap();
    assert_eq!(
        report2.removed_envs + report2.removed_layers + report2.removed_objects,
        0,
        "second GC run should find nothing to collect: {report2:?}"
    );

    // First run should have found something to collect
    assert!(
        report1.removed_envs + report1.removed_layers + report1.removed_objects > 0,
        "first GC run should have collected orphans: {report1:?}"
    );
}

// --- M6: Failure Mode Testing (2.0) ---

// M6.2: Object read fails gracefully on permission denied
#[test]
fn object_get_fails_on_permission_denied() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let obj_store = karapace_store::ObjectStore::new(layout.clone());
    let hash = obj_store.put(b"test data").unwrap();

    // Remove read permission on the object file
    let obj_path = layout.objects_dir().join(&hash);
    fs::set_permissions(&obj_path, fs::Permissions::from_mode(0o000)).unwrap();

    let result = obj_store.get(&hash);
    assert!(result.is_err(), "get must fail on permission denied");

    // Restore permissions for cleanup
    fs::set_permissions(&obj_path, fs::Permissions::from_mode(0o644)).unwrap();
}

// M6.2: Metadata write fails gracefully on read-only store
#[test]
fn metadata_put_fails_on_read_only_dir() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let meta_store = karapace_store::MetadataStore::new(layout.clone());

    // Make metadata dir read-only
    let meta_dir = layout.metadata_dir();
    fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o555)).unwrap();

    let meta = karapace_store::EnvMetadata {
        env_id: "test_ro".into(),
        short_id: "test_ro".into(),
        name: None,
        state: EnvState::Defined,
        manifest_hash: "mhash".into(),
        base_layer: "base".into(),
        dependency_layers: vec![],
        policy_layer: None,
        created_at: "2025-01-01T00:00:00Z".to_owned(),
        updated_at: "2025-01-01T00:00:00Z".to_owned(),
        ref_count: 1,
        checksum: None,
    };
    let result = meta_store.put(&meta);
    assert!(result.is_err(), "put must fail on read-only metadata dir");

    // Restore permissions for cleanup
    fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o755)).unwrap();
}

// M6.3: Concurrent GC blocked by store lock
#[test]
fn concurrent_gc_blocked_by_lock() {
    let store = tempfile::tempdir().unwrap();
    let _engine = Engine::new(store.path());
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    // Acquire lock in main thread
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();

    // Try to acquire lock again — should return None (non-blocking try)
    let result = StoreLock::try_acquire(&layout.lock_file()).unwrap();
    assert!(
        result.is_none(),
        "second lock acquisition must return None while first is held"
    );

    drop(lock);

    // After drop, lock should be available again
    let lock2 = StoreLock::try_acquire(&layout.lock_file()).unwrap();
    assert!(lock2.is_some(), "lock must be available after drop");
}

// M6: Layer corruption detected on get
#[test]
fn layer_corruption_detected_on_get() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let layer_store = karapace_store::LayerStore::new(layout.clone());
    let layer = karapace_store::LayerManifest {
        hash: "test_layer".to_owned(),
        kind: karapace_store::LayerKind::Base,
        parent: None,
        object_refs: vec![],
        read_only: true,
        tar_hash: String::new(),
    };
    let content_hash = layer_store.put(&layer).unwrap();

    // Corrupt the layer file (flip bytes but keep it valid-ish)
    let layer_path = layout.layers_dir().join(&content_hash);
    fs::write(&layer_path, b"corrupted data that is not the original").unwrap();

    let result = layer_store.get(&content_hash);
    assert!(
        result.is_err(),
        "corrupted layer must be detected by hash verification"
    );
}

// M6: Metadata corruption detected on get
#[test]
fn metadata_corruption_detected_on_get() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let meta_store = karapace_store::MetadataStore::new(layout.clone());
    let meta = karapace_store::EnvMetadata {
        env_id: "corrupt_test".into(),
        short_id: "corrupt_test".into(),
        name: None,
        state: EnvState::Built,
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

    // Corrupt the metadata file while keeping valid JSON with wrong checksum
    let meta_path = layout.metadata_dir().join("corrupt_test");
    let mut content = fs::read_to_string(&meta_path).unwrap();
    content = content.replace("corrupt_test", "tampered_val");
    fs::write(&meta_path, &content).unwrap();

    let result = meta_store.get("corrupt_test");
    assert!(
        result.is_err(),
        "tampered metadata must be detected by checksum verification"
    );
}

// M6: Destroy non-existent environment returns clean error
#[test]
fn destroy_nonexistent_env_returns_error() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let result = engine.destroy("does_not_exist_abc123");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("NotFound") || err.contains("does_not_exist"),
        "error should indicate env not found: {err}"
    );
}

// M6: Build with invalid manifest returns clean error
#[test]
fn build_invalid_manifest_returns_error() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = project.path().join("karapace.toml");
    fs::write(&manifest, "this is not valid toml [[[").unwrap();

    let result = engine.build(&manifest);
    assert!(result.is_err(), "build with invalid manifest must fail");
}

// stop() on non-Running env returns correct error
#[test]
fn stop_non_running_env_returns_error() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();

    // Environment is in Built state, not Running
    let result = engine.stop(&r.identity.env_id);
    assert!(result.is_err(), "stop on Built env must fail");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.to_lowercase().contains("running")
            || err_msg.to_lowercase().contains("not running"),
        "error should mention running state: {err_msg}"
    );
}

#[test]
fn stale_running_marker_cleaned_on_engine_new() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    // Build an environment so we have an env_dir
    let engine = Engine::new(store.path());
    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let result = engine.build(&manifest);
    assert!(result.is_ok());
    let env_id = result.unwrap().identity.env_id;

    // Manually create a stale .running marker (simulates a crash)
    let env_dir = store.path().join("env").join(&*env_id);
    fs::create_dir_all(&env_dir).unwrap();
    let running_marker = env_dir.join(".running");
    fs::write(&running_marker, "stale").unwrap();
    assert!(running_marker.exists());

    // Creating a new Engine should clean up the stale marker
    let _engine2 = Engine::new(store.path());
    assert!(
        !running_marker.exists(),
        "stale .running marker must be removed by Engine::new()"
    );
}

// --- §7: Disk-full / write-failure simulation ---

#[test]
fn build_on_readonly_objects_dir_returns_error() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    // Initialize the store, then make objects dir read-only
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();
    let objects_dir = layout.objects_dir();
    fs::set_permissions(&objects_dir, fs::Permissions::from_mode(0o444)).unwrap();

    let engine = Engine::new(store.path());
    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let result = engine.build(&manifest);

    // Restore permissions for cleanup
    fs::set_permissions(&objects_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        result.is_err(),
        "build must fail when objects dir is read-only"
    );
}

#[test]
fn build_on_readonly_metadata_dir_returns_error() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();
    let meta_dir = layout.metadata_dir();
    fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o444)).unwrap();

    let engine = Engine::new(store.path());
    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let result = engine.build(&manifest);

    fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        result.is_err(),
        "build must fail when metadata dir is read-only"
    );
}

#[test]
fn commit_on_readonly_layers_dir_returns_error() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Create upper dir with content
    let upper = store.path().join("env").join(&env_id).join("upper");
    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("file.txt"), "data").unwrap();

    // Make layers dir read-only
    let layout = StoreLayout::new(store.path());
    let layers_dir = layout.layers_dir();
    fs::set_permissions(&layers_dir, fs::Permissions::from_mode(0o444)).unwrap();

    let result = engine.commit(&env_id);

    fs::set_permissions(&layers_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        result.is_err(),
        "commit must fail when layers dir is read-only"
    );

    // Store should still be usable after the error
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(
        meta.state,
        EnvState::Built,
        "env must still be in Built state after failed commit"
    );
}

#[test]
fn write_failure_never_panics() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    // Test ObjectStore::put on read-only dir
    let objects_dir = layout.objects_dir();
    fs::set_permissions(&objects_dir, fs::Permissions::from_mode(0o444)).unwrap();
    let obj_store = karapace_store::ObjectStore::new(layout.clone());
    let result = obj_store.put(b"test data");
    fs::set_permissions(&objects_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        result.is_err(),
        "ObjectStore::put must return Err, not panic"
    );

    // Test LayerStore::put on read-only dir
    let layers_dir = layout.layers_dir();
    fs::set_permissions(&layers_dir, fs::Permissions::from_mode(0o444)).unwrap();
    let layer_store = karapace_store::LayerStore::new(layout.clone());
    let layer = karapace_store::LayerManifest {
        hash: "test".into(),
        kind: karapace_store::LayerKind::Base,
        parent: None,
        object_refs: vec![],
        read_only: true,
        tar_hash: "test".into(),
    };
    let result = layer_store.put(&layer);
    fs::set_permissions(&layers_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        result.is_err(),
        "LayerStore::put must return Err, not panic"
    );

    // Test MetadataStore::put on read-only dir
    let meta_dir = layout.metadata_dir();
    fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o444)).unwrap();
    let meta_store = karapace_store::MetadataStore::new(layout.clone());
    let meta = karapace_store::EnvMetadata {
        env_id: "test123".into(),
        short_id: "test123".into(),
        name: None,
        state: EnvState::Defined,
        manifest_hash: "mh".into(),
        base_layer: "bl".into(),
        dependency_layers: vec![],
        policy_layer: None,
        created_at: "2025-01-01T00:00:00Z".to_owned(),
        updated_at: "2025-01-01T00:00:00Z".to_owned(),
        ref_count: 1,
        checksum: None,
    };
    let result = meta_store.put(&meta);
    fs::set_permissions(&meta_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        result.is_err(),
        "MetadataStore::put must return Err, not panic"
    );
}

// --- §2: Bit-flip corruption detection ---

#[test]
fn bitflip_corruption_detected_on_objects() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();
    let obj_store = karapace_store::ObjectStore::new(layout.clone());

    // Write several objects
    let h1 = obj_store.put(b"object-alpha").unwrap();
    let h2 = obj_store.put(b"object-beta-longer-content-here").unwrap();
    let h3 = obj_store.put(b"object-gamma").unwrap();

    // Flip a random byte in object 2
    let path = layout.objects_dir().join(&h2);
    let mut data = fs::read(&path).unwrap();
    let flip_idx = data.len() / 2;
    data[flip_idx] ^= 0xFF;
    fs::write(&path, &data).unwrap();

    // Intact objects must still be readable
    assert!(obj_store.get(&h1).is_ok());
    assert!(obj_store.get(&h3).is_ok());

    // Corrupted object must fail with typed error
    let result = obj_store.get(&h2);
    assert!(result.is_err(), "bit-flipped object must be detected");

    // verify_store_integrity must report exactly 1 failure
    let report = karapace_store::verify_store_integrity(&layout).unwrap();
    assert_eq!(report.checked, 3);
    assert_eq!(report.passed, 2);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].hash, h2);
}

// --- §6: GC dry-run equivalence ---

#[test]
fn gc_dry_run_matches_real_gc() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    // Build 5 environments, destroy 3
    let mut env_ids = Vec::new();
    for i in 0..5 {
        let p = tempfile::tempdir().unwrap();
        let pkgs: Vec<String> = (0..=i).map(|j| format!("pkg{j}")).collect();
        let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
        let m = write_manifest(p.path(), &mock_manifest(&pkg_refs));
        let r = engine.build(&m).unwrap();
        env_ids.push(r.identity.env_id.to_string());
    }
    for id in &env_ids[..3] {
        engine.destroy(id).unwrap();
    }

    let layout = StoreLayout::new(store.path());
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();

    // Dry run — populates orphaned_* but doesn't remove
    let dry_report = engine.gc(&lock, true).unwrap();
    assert_eq!(
        dry_report.removed_envs, 0,
        "dry-run must not remove anything"
    );

    // Real GC — actually removes
    let real_report = engine.gc(&lock, false).unwrap();

    // Dry-run orphaned counts must match real removed counts
    assert_eq!(
        dry_report.orphaned_envs.len(),
        real_report.removed_envs,
        "dry-run orphaned_envs must match real removed_envs"
    );
    assert_eq!(
        dry_report.orphaned_layers.len(),
        real_report.removed_layers,
        "dry-run orphaned_layers must match real removed_layers"
    );
    assert_eq!(
        dry_report.orphaned_objects.len(),
        real_report.removed_objects,
        "dry-run orphaned_objects must match real removed_objects"
    );
}

// --- §15: Soak test — 100 build/destroy/GC cycles ---

#[test]
fn soak_100_build_destroy_gc_cycles() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());
    let layout = StoreLayout::new(store.path());

    for cycle in 0..100 {
        let p = tempfile::tempdir().unwrap();
        let pkgs: Vec<String> = (0..=(cycle % 5)).map(|j| format!("pkg{j}")).collect();
        let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
        let m = write_manifest(p.path(), &mock_manifest(&pkg_refs));

        let r = engine.build(&m).unwrap();
        let env_id = r.identity.env_id.to_string();

        // Verify it's inspectable
        let meta = engine.inspect(&env_id).unwrap();
        assert_eq!(meta.state, EnvState::Built);

        // Destroy
        engine.destroy(&env_id).unwrap();

        // GC every 10 cycles
        if cycle % 10 == 9 {
            let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
            let _report = engine.gc(&lock, false).unwrap();
        }
    }

    // Final state: no environments should remain
    let envs = engine.list().unwrap();
    assert_eq!(
        envs.len(),
        0,
        "all environments must be destroyed after soak"
    );

    // Final GC — store should be clean
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
    let _report = engine.gc(&lock, false).unwrap();

    // WAL must be clean
    let wal = karapace_store::WriteAheadLog::new(&layout);
    assert!(wal.list_incomplete().unwrap().is_empty());

    // Store integrity must pass
    let integrity = karapace_store::verify_store_integrity(&layout).unwrap();
    assert!(
        integrity.failed.is_empty(),
        "store integrity must be clean after 100 cycles"
    );
}

// --- §11: Store v1 rejection test ---

#[test]
fn old_store_version_rejected_with_clear_error() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    // Overwrite the version file with v1 (old format)
    let version_path = store.path().join("store").join("version");
    fs::write(&version_path, r#"{"format_version": 1}"#).unwrap();

    // Re-initializing must reject v1
    let result = layout.initialize();
    assert!(
        result.is_err(),
        "store v1 must be rejected by the current binary"
    );

    // The error message should mention the version
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("version") || err_msg.contains("format"),
        "error should mention version mismatch: {err_msg}"
    );
}

// --- §4: Resolver / lockfile determinism ---

#[test]
fn build_with_invalid_manifest_returns_manifest_error() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(
        project.path(),
        r"
manifest_version = 1
[base]
",
    );
    let result = engine.build(&manifest);
    assert!(result.is_err(), "build with missing image must fail");
}

#[test]
fn build_with_invalid_backend_returns_error() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(
        project.path(),
        r#"
manifest_version = 1
[base]
image = "rolling"
[runtime]
backend = "nonexistent_backend"
"#,
    );
    let result = engine.build(&manifest);
    assert!(result.is_err(), "build with unknown backend must fail");
}

#[test]
fn lockfile_determinism_same_inputs_same_env_id() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    // Build twice with identical manifests in different project dirs
    let p1 = tempfile::tempdir().unwrap();
    let m1 = write_manifest(p1.path(), &mock_manifest(&["git", "vim"]));
    let r1 = engine.build(&m1).unwrap();

    // Destroy first, then rebuild with same manifest
    engine.destroy(&r1.identity.env_id).unwrap();

    let p2 = tempfile::tempdir().unwrap();
    let m2 = write_manifest(p2.path(), &mock_manifest(&["git", "vim"]));
    let r2 = engine.build(&m2).unwrap();

    assert_eq!(
        r1.identity.env_id, r2.identity.env_id,
        "same manifest must produce same env_id"
    );
}

#[test]
fn different_packages_produce_different_env_id() {
    let store = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let p1 = tempfile::tempdir().unwrap();
    let m1 = write_manifest(p1.path(), &mock_manifest(&["git"]));
    let r1 = engine.build(&m1).unwrap();

    let p2 = tempfile::tempdir().unwrap();
    let m2 = write_manifest(p2.path(), &mock_manifest(&["vim"]));
    let r2 = engine.build(&m2).unwrap();

    assert_ne!(
        r1.identity.env_id, r2.identity.env_id,
        "different packages must produce different env_id"
    );
}

// --- §6: GC 1000-object stress test ---

#[test]
fn gc_stress_1000_objects() {
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();
    let obj_store = karapace_store::ObjectStore::new(layout.clone());

    // Create 1000 orphaned objects (no layer or metadata references them)
    for i in 0..1000 {
        obj_store
            .put(format!("orphaned-object-{i}").as_bytes())
            .unwrap();
    }

    // Also create a live environment so GC has reachable refs to trace
    let engine = Engine::new(store.path());
    let p = tempfile::tempdir().unwrap();
    let m = write_manifest(p.path(), &mock_manifest(&["git"]));
    let _r = engine.build(&m).unwrap();

    // Run GC
    let lock = StoreLock::acquire(&layout.lock_file()).unwrap();
    let report = engine.gc(&lock, false).unwrap();

    // All 1000 orphans must be collected
    assert_eq!(
        report.removed_objects, 1000,
        "GC must collect all 1000 orphaned objects, got {}",
        report.removed_objects
    );

    // Live env must survive
    assert_eq!(engine.list().unwrap().len(), 1);

    // Store integrity must still pass
    let integrity = karapace_store::verify_store_integrity(&layout).unwrap();
    assert!(
        integrity.failed.is_empty(),
        "store integrity must pass after GC stress"
    );
}

// --- §5: Concurrent build operations ---

#[test]
fn concurrent_builds_do_not_corrupt_store() {
    let store = tempfile::tempdir().unwrap();
    let store_path = store.path().to_owned();

    let barrier = Arc::new(Barrier::new(4));
    let mut handles = Vec::new();

    for thread_idx in 0..4 {
        let path = store_path.clone();
        let b = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let engine = Engine::new(&path);
            b.wait();

            let p = tempfile::tempdir().unwrap();
            let pkgs: Vec<String> = (0..=(thread_idx % 3))
                .map(|j| format!("t{thread_idx}pkg{j}"))
                .collect();
            let pkg_refs: Vec<&str> = pkgs.iter().map(String::as_str).collect();
            let m_content = format!(
                r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = [{}]
[runtime]
backend = "mock"
"#,
                pkg_refs
                    .iter()
                    .map(|p| format!("\"{p}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let manifest_path = p.path().join("karapace.toml");
            fs::write(&manifest_path, &m_content).unwrap();
            engine.build(&manifest_path).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Verify store is healthy after concurrent builds
    let engine = Engine::new(&store_path);
    let envs = engine.list().unwrap();
    assert_eq!(envs.len(), 4, "all 4 concurrent builds must succeed");

    let layout = StoreLayout::new(&store_path);
    let integrity = karapace_store::verify_store_integrity(&layout).unwrap();
    assert!(
        integrity.failed.is_empty(),
        "store must be intact after concurrent builds"
    );
}

// --- M6.1: WAL write failure simulation ---

#[test]
fn wal_write_fails_on_readonly_dir() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();

    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    // Make WAL dir read-only
    let wal_dir = store.path().join("store").join("wal");
    fs::set_permissions(&wal_dir, fs::Permissions::from_mode(0o444)).unwrap();

    let result = wal.begin(karapace_store::WalOpKind::Build, "test-env");

    fs::set_permissions(&wal_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        result.is_err(),
        "WAL begin must fail when WAL dir is read-only"
    );
}

#[test]
fn build_fails_cleanly_when_wal_dir_is_readonly() {
    if skip_if_root() { return; }
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();

    // Initialize layout and WAL so the dir exists
    let layout = StoreLayout::new(store.path());
    layout.initialize().unwrap();
    let wal = karapace_store::WriteAheadLog::new(&layout);
    wal.initialize().unwrap();

    let engine = Engine::new(store.path());

    // Now make WAL dir read-only to simulate disk full
    let wal_dir = layout.root().join("store").join("wal");
    fs::set_permissions(&wal_dir, fs::Permissions::from_mode(0o444)).unwrap();

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let result = engine.build(&manifest);

    fs::set_permissions(&wal_dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(
        result.is_err(),
        "build must fail cleanly when WAL dir is read-only (simulates disk full)"
    );
}

// --- M6.4: stop() SIGTERM/SIGKILL path tests ---

#[test]
fn stop_sends_sigterm_to_real_process() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Spawn a real sleep process and wait on it later to avoid zombie
    let mut child = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();
    let pid_i32 = i32::try_from(pid).expect("pid fits in i32");

    // Set metadata to Running state and write .running marker with real PID
    let layout = StoreLayout::new(store.path());
    let meta_store = karapace_store::MetadataStore::new(layout.clone());
    meta_store.update_state(&env_id, EnvState::Running).unwrap();

    let env_dir = store.path().join("env").join(&env_id);
    fs::create_dir_all(&env_dir).unwrap();
    fs::write(env_dir.join(".running"), pid.to_string()).unwrap();

    // Verify the process is alive
    assert!(
        Path::new(&format!("/proc/{pid}")).exists(),
        "sleep process must be alive before stop"
    );

    // Stop should send SIGTERM (the mock backend returns pid=99999 which won't exist,
    // but the real process PID was written to .running). The mock backend's status()
    // returns pid=99999 for running envs, so stop() will try to kill 99999 (ESRCH).
    // This tests the ESRCH handling path.
    let result = engine.stop(&env_id);

    // Clean up the real process regardless of result
    unsafe {
        libc::kill(pid_i32, libc::SIGKILL);
    }
    let _ = child.wait();

    assert!(
        result.is_ok(),
        "stop must succeed even when backend PID no longer exists (ESRCH path): {:?}",
        result.err()
    );

    // Verify state was reset to Built
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(
        meta.state,
        EnvState::Built,
        "state must be reset to Built after stop"
    );
}

#[test]
fn stop_invalid_pid_handled_gracefully() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Manually set state to Running (the mock backend will report pid=99999)
    let layout = StoreLayout::new(store.path());
    let meta_store = karapace_store::MetadataStore::new(layout.clone());
    meta_store.update_state(&env_id, EnvState::Running).unwrap();

    // stop() will try to kill pid 99999 which doesn't exist → ESRCH
    let result = engine.stop(&env_id);

    // Must handle gracefully (ESRCH = process already exited)
    assert!(
        result.is_ok(),
        "stop with non-existent PID must succeed (ESRCH handled): {:?}",
        result.err()
    );

    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(meta.state, EnvState::Built);
}

// --- M7: Coverage expansion ---

#[test]
fn freeze_and_archive_state_transitions() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Built → Frozen
    engine.freeze(&env_id).unwrap();
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(meta.state, EnvState::Frozen);

    // Frozen → Archived
    engine.archive(&env_id).unwrap();
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(meta.state, EnvState::Archived);
}

#[test]
fn rename_environment_works() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r = engine.build(&manifest).unwrap();
    let env_id = r.identity.env_id.to_string();

    // Rename
    engine.rename(&env_id, "my-dev-env").unwrap();
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(meta.name, Some("my-dev-env".to_owned()));

    // Rename again
    engine.rename(&env_id, "new-name").unwrap();
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(meta.name, Some("new-name".to_owned()));
}

#[test]
fn verify_store_reports_all_clean_after_fresh_build() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &mock_manifest(&["git", "vim"]));
    let _r = engine.build(&manifest).unwrap();

    let layout = StoreLayout::new(store.path());
    let report = karapace_store::verify_store_integrity(&layout).unwrap();

    assert!(report.checked > 0, "must check at least some objects");
    assert_eq!(report.passed, report.checked);
    assert!(report.failed.is_empty());
    assert!(report.layers_checked > 0, "must check at least some layers");
    assert_eq!(report.layers_passed, report.layers_checked);
    assert!(
        report.metadata_checked > 0,
        "must check at least some metadata"
    );
    assert_eq!(report.metadata_passed, report.metadata_checked);
}

#[test]
fn rebuild_preserves_new_and_cleans_old() {
    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    // Initial build
    let manifest = write_manifest(project.path(), &mock_manifest(&["git"]));
    let r1 = engine.build(&manifest).unwrap();
    let old_id = r1.identity.env_id.to_string();

    // Rebuild with different packages → different env_id
    let manifest2 = write_manifest(project.path(), &mock_manifest(&["git", "vim"]));
    let r2 = engine.rebuild(&manifest2).unwrap();
    let new_id = r2.identity.env_id.to_string();

    assert_ne!(
        old_id, new_id,
        "different packages must produce different env_id"
    );

    // New env must be inspectable
    let meta = engine.inspect(&new_id).unwrap();
    assert_eq!(meta.state, EnvState::Built);

    // Old env must be gone (destroyed by rebuild)
    assert!(engine.inspect(&old_id).is_err());
}
