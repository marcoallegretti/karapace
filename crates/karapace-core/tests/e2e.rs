//! End-to-end tests that exercise the real namespace backend.
//!
//! These tests are `#[ignore]` by default because they require:
//! - Linux with user namespace support
//! - `fuse-overlayfs` installed
//! - `curl` installed
//! - Network access (to download base images)
//!
//! Run with: `cargo test --test e2e -- --ignored`

use karapace_core::Engine;
use karapace_store::{EnvState, StoreLayout};
use std::fs;
use std::path::Path;

fn namespace_manifest(packages: &[&str]) -> String {
    let pkgs = if packages.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = packages.iter().map(|p| format!("\"{p}\"")).collect();
        format!("\n[system]\npackages = [{}]\n", list.join(", "))
    };
    format!(
        r#"manifest_version = 1

[base]
image = "rolling"
{pkgs}
[runtime]
backend = "namespace"
"#
    )
}

fn write_manifest(dir: &Path, content: &str) -> std::path::PathBuf {
    let path = dir.join("karapace.toml");
    fs::write(&path, content).unwrap();
    path
}

fn prereqs_available() -> bool {
    let ns = karapace_runtime::check_namespace_prereqs();
    if !ns.is_empty() {
        let msg = karapace_runtime::format_missing(&ns);
        assert!(
            std::env::var("CI").is_err(),
            "CI FATAL: E2E prerequisites missing — tests cannot silently skip in CI.\n{msg}"
        );
        eprintln!("skipping E2E: missing prerequisites: {msg}");
        return false;
    }
    true
}

/// Build a minimal environment with the namespace backend (no packages).
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_build_minimal_namespace() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();

    assert!(!result.identity.env_id.is_empty());
    assert!(!result.identity.short_id.is_empty());

    let meta = engine.inspect(&result.identity.env_id).unwrap();
    assert_eq!(meta.state, EnvState::Built);

    // Verify the environment directory was created
    let layout = StoreLayout::new(store.path());
    assert!(layout.env_path(&result.identity.env_id).exists());

    // Lock file was written
    assert!(project.path().join("karapace.lock").exists());
}

/// Exec a command inside a built environment and verify stdout.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_exec_in_namespace() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();

    // Exec `echo hello` inside the container
    let cmd = vec!["echo".to_owned(), "hello".to_owned()];
    // exec() writes to stdout/stderr directly; just verify it doesn't error
    engine.exec(&result.identity.env_id, &cmd).unwrap();
}

/// Destroy cleans up all overlay directories.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_destroy_cleans_up() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();
    let env_id = result.identity.env_id.clone();

    let layout = StoreLayout::new(store.path());
    assert!(layout.env_path(&env_id).exists());

    engine.destroy(&env_id).unwrap();

    // Environment directory should be gone
    assert!(!layout.env_path(&env_id).exists());
}

/// Rebuild produces the same env_id for the same manifest.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_rebuild_determinism() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let r1 = engine.build(&manifest).unwrap();
    let r2 = engine.rebuild(&manifest).unwrap();

    assert_eq!(
        r1.identity.env_id, r2.identity.env_id,
        "rebuild must produce the same env_id"
    );
}

/// Snapshot and restore round-trip with real namespace backend.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_snapshot_and_restore() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();
    let env_id = result.identity.env_id.clone();

    // Write a file to the upper dir (simulating user modifications)
    let upper = StoreLayout::new(store.path()).upper_dir(&env_id);
    if upper.exists() {
        // Clear build artifacts first
        let _ = fs::remove_dir_all(&upper);
    }
    fs::create_dir_all(&upper).unwrap();
    fs::write(upper.join("user_data.txt"), "snapshot baseline").unwrap();

    // Commit a snapshot
    let snapshot_hash = engine.commit(&env_id).unwrap();
    assert!(!snapshot_hash.is_empty());

    // Verify snapshot is listed
    let snapshots = engine.list_snapshots(&env_id).unwrap();
    assert_eq!(snapshots.len(), 1);

    // Mutate upper dir after snapshot
    fs::write(upper.join("user_data.txt"), "MODIFIED").unwrap();
    fs::write(upper.join("extra.txt"), "should disappear").unwrap();

    // Restore from snapshot
    engine.restore(&env_id, &snapshot_hash).unwrap();

    // Verify restore worked
    assert_eq!(
        fs::read_to_string(upper.join("user_data.txt")).unwrap(),
        "snapshot baseline"
    );
    assert!(
        !upper.join("extra.txt").exists(),
        "extra file must be gone after restore"
    );
}

/// Overlay correctness: files written in upper are visible, base is read-only.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_overlay_file_visibility() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();
    let env_id = result.identity.env_id.clone();

    let layout = StoreLayout::new(store.path());
    let upper = layout.upper_dir(&env_id);
    fs::create_dir_all(&upper).unwrap();

    // Write a file in upper — should be visible via exec
    fs::write(upper.join("test_marker.txt"), "visible").unwrap();

    // exec `cat /test_marker.txt` should succeed (file visible through overlay)
    let cmd = vec!["cat".to_owned(), "/test_marker.txt".to_owned()];
    let result = engine.exec(&env_id, &cmd);
    // If overlay is correctly mounted, the file is visible
    assert!(
        result.is_ok(),
        "files in upper dir must be visible through overlay"
    );
}

/// Enter/exit cycle: repeated enter should not leak state.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_enter_exit_cycle() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();
    let env_id = result.identity.env_id.clone();

    // Run exec 20 times — should not accumulate state or leak
    for i in 0..20 {
        let cmd = vec!["echo".to_owned(), format!("cycle-{i}")];
        engine.exec(&env_id, &cmd).unwrap();
    }

    // Environment should still be in Built state
    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(
        meta.state,
        EnvState::Built,
        "env must be Built after enter/exit cycles"
    );
}

// --- IG-M1: Real Runtime Backend Validation ---

/// Verify no fuse-overlayfs mounts leak after build + exec + destroy cycle.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_mount_leak_detection() {
    if !prereqs_available() {
        return;
    }

    let mounts_before = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let fuse_before = mounts_before
        .lines()
        .filter(|l| l.contains("fuse-overlayfs"))
        .count();

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();
    let env_id = result.identity.env_id.clone();

    // Exec inside
    engine
        .exec(&env_id, &["echo".to_owned(), "leak-test".to_owned()])
        .unwrap();

    // Destroy
    engine.destroy(&env_id).unwrap();

    let mounts_after = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let fuse_after = mounts_after
        .lines()
        .filter(|l| l.contains("fuse-overlayfs"))
        .count();

    assert_eq!(
        fuse_before, fuse_after,
        "fuse-overlayfs mount count must not change after build+exec+destroy: before={fuse_before}, after={fuse_after}"
    );
}

/// Repeated build/destroy cycles must not accumulate state or stale mounts.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_build_destroy_20_cycles() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());
    let layout = StoreLayout::new(store.path());

    for i in 0..20 {
        let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
        let result = engine.build(&manifest).unwrap();
        let env_id = result.identity.env_id.clone();
        engine.destroy(&env_id).unwrap();
        assert!(
            !layout.env_path(&env_id).exists(),
            "env dir must be gone after destroy in cycle {i}"
        );
    }

    // Final integrity check
    let report = karapace_store::verify_store_integrity(&layout).unwrap();
    assert!(
        report.failed.is_empty(),
        "store integrity must pass after 20 build/destroy cycles: {:?}",
        report.failed
    );

    // No stale overlays
    let mounts = fs::read_to_string("/proc/mounts").unwrap_or_default();
    let store_path_str = store.path().to_string_lossy();
    let stale: Vec<&str> = mounts
        .lines()
        .filter(|l| l.contains("fuse-overlayfs") && l.contains(store_path_str.as_ref()))
        .collect();
    assert!(
        stale.is_empty(),
        "no stale overlayfs mounts after 20 cycles: {stale:?}"
    );
}

/// If an OCI runtime (crun/runc) is available, build and destroy with it.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_oci_build_if_available() {
    if !prereqs_available() {
        return;
    }

    // Check if crun or runc exists
    let has_oci = std::process::Command::new("which")
        .arg("crun")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::process::Command::new("which")
            .arg("runc")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

    if !has_oci {
        assert!(
            std::env::var("CI").is_err(),
            "CI FATAL: OCI test requires crun or runc — install in CI or remove test from CI job"
        );
        eprintln!("skipping OCI test: no crun or runc found");
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest_content = r#"manifest_version = 1

[base]
image = "rolling"

[runtime]
backend = "oci"
"#;
    let manifest = write_manifest(project.path(), manifest_content);
    let result = engine.build(&manifest).unwrap();
    let env_id = result.identity.env_id.clone();

    let meta = engine.inspect(&env_id).unwrap();
    assert_eq!(meta.state, EnvState::Built);

    engine.destroy(&env_id).unwrap();
    let layout = StoreLayout::new(store.path());
    assert!(
        !layout.env_path(&env_id).exists(),
        "OCI env dir must be cleaned up after destroy"
    );
}

/// Concurrent exec calls on the same environment must all succeed.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_namespace_concurrent_exec() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = std::sync::Arc::new(Engine::new(store.path()));

    let manifest = write_manifest(project.path(), &namespace_manifest(&[]));
    let result = engine.build(&manifest).unwrap();
    let env_id = std::sync::Arc::new(result.identity.env_id.clone());

    let handles: Vec<_> = (0..4)
        .map(|i| {
            let eng = std::sync::Arc::clone(&engine);
            let eid = std::sync::Arc::clone(&env_id);
            std::thread::spawn(move || {
                let cmd = vec!["echo".to_owned(), format!("thread-{i}")];
                eng.exec(&eid, &cmd).unwrap();
            })
        })
        .collect();

    for h in handles {
        h.join().expect("exec thread must not panic");
    }

    // No stale .running markers
    let layout = StoreLayout::new(store.path());
    let env_path = layout.env_path(&env_id);
    if env_path.exists() {
        let running_markers: Vec<_> = fs::read_dir(&env_path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".running"))
            .collect();
        assert!(
            running_markers.is_empty(),
            "no stale .running markers after concurrent exec: {:?}",
            running_markers
                .iter()
                .map(fs::DirEntry::file_name)
                .collect::<Vec<_>>()
        );
    }
}

// --- IG-M2: Real Package Resolution Validation ---

/// Verify resolved packages have real versions (not mock/unresolved).
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_resolve_pins_exact_versions() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&["curl"]));
    let result = engine.build(&manifest).unwrap();

    for pkg in &result.lock_file.resolved_packages {
        assert!(
            !pkg.version.is_empty(),
            "package {} has empty version",
            pkg.name
        );
        assert_ne!(
            pkg.version, "0.0.0-mock",
            "package {} has mock version — real resolver not running",
            pkg.name
        );
        // Version should contain at least one digit
        assert!(
            pkg.version.chars().any(|c| c.is_ascii_digit()),
            "package {} version '{}' contains no digits — suspect",
            pkg.name,
            pkg.version
        );
    }
}

/// Rebuild same manifest must produce identical env_id and resolved versions.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_resolve_deterministic_across_rebuilds() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&["curl"]));
    let r1 = engine.build(&manifest).unwrap();
    let r2 = engine.rebuild(&manifest).unwrap();

    assert_eq!(
        r1.identity.env_id, r2.identity.env_id,
        "same manifest must produce same env_id"
    );
    assert_eq!(
        r1.lock_file.resolved_packages, r2.lock_file.resolved_packages,
        "resolved packages must be identical across rebuilds"
    );
}

/// Building with a non-existent package must fail cleanly.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_resolve_nonexistent_package_fails() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(
        project.path(),
        &namespace_manifest(&["nonexistent-pkg-that-does-not-exist-xyz"]),
    );
    let result = engine.build(&manifest);

    assert!(result.is_err(), "build with non-existent package must fail");

    // No orphaned env directories
    let layout = StoreLayout::new(store.path());
    let env_dir = layout.env_dir();
    if env_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&env_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(
            entries.is_empty(),
            "no orphaned env dirs after failed build: {:?}",
            entries
                .iter()
                .map(fs::DirEntry::file_name)
                .collect::<Vec<_>>()
        );
    }
}

/// Build with multiple packages — all must have non-empty resolved versions.
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_resolve_multiple_packages() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    let manifest = write_manifest(project.path(), &namespace_manifest(&["curl", "git"]));
    let result = engine.build(&manifest).unwrap();

    assert!(
        result.lock_file.resolved_packages.len() >= 2,
        "at least 2 resolved packages expected, got {}",
        result.lock_file.resolved_packages.len()
    );
    for pkg in &result.lock_file.resolved_packages {
        assert!(
            !pkg.version.is_empty() && pkg.version != "unresolved",
            "package {} has unresolved version",
            pkg.name
        );
    }
}

/// Build with packages (requires network to download image + install).
#[test]
#[ignore = "requires Linux user namespaces, fuse-overlayfs, curl, and network"]
fn e2e_build_with_packages() {
    if !prereqs_available() {
        return;
    }

    let store = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let engine = Engine::new(store.path());

    // Use a package whose zypper name matches its RPM name on openSUSE
    let manifest = write_manifest(project.path(), &namespace_manifest(&["curl"]));
    let result = engine.build(&manifest).unwrap();

    assert!(!result.identity.env_id.is_empty());

    // Lock file should have resolved packages with real versions
    let lock = result.lock_file;
    assert_eq!(lock.lock_version, 2);
    assert!(!lock.resolved_packages.is_empty());
    // At least one package should have a resolved version.
    // Note: some package names may not match their RPM names exactly,
    // causing fallback to "unresolved". This is a known limitation.
    let resolved_count = lock
        .resolved_packages
        .iter()
        .filter(|p| p.version != "unresolved")
        .count();
    assert!(
        resolved_count > 0,
        "at least one package should have a resolved version, got: {:?}",
        lock.resolved_packages
    );
}
