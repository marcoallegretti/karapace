//! CLI subprocess integration tests.
//!
//! These tests invoke the `karapace` binary as a subprocess and verify
//! exit codes, stdout content, and JSON output stability.

use std::process::Command;

fn karapace_bin() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_karapace"));
    // Skip runtime prerequisite checks — mock backend does not need user namespaces
    cmd.env("KARAPACE_SKIP_PREREQS", "1");
    cmd
}

fn write_minimal_manifest(dir: &std::path::Path, base_image: &str) -> std::path::PathBuf {
    let path = dir.join("karapace.toml");
    std::fs::write(
        &path,
        format!(
            r#"manifest_version = 1

[base]
image = "{base_image}"

[runtime]
backend = "mock"
"#
        ),
    )
    .unwrap();
    path
}

fn temp_store() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write_test_manifest(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("karapace.toml");
    std::fs::write(
        &path,
        r#"manifest_version = 1

[base]
image = "rolling"

[system]
packages = ["git"]

[runtime]
backend = "mock"
"#,
    )
    .unwrap();
    path
}

// A5: CLI Validation — version flag
#[test]
fn cli_version_exits_zero() {
    let output = karapace_bin().arg("--version").output().unwrap();
    assert!(output.status.success(), "karapace --version must exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("karapace"),
        "version output must contain 'karapace': {stdout}"
    );
}

// A5: CLI Validation — help flag
#[test]
fn cli_help_exits_zero() {
    let output = karapace_bin().arg("--help").output().unwrap();
    assert!(output.status.success(), "karapace --help must exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("build"), "help must list 'build' command");
    assert!(
        stdout.contains("destroy"),
        "help must list 'destroy' command"
    );
}

// A5: CLI Validation — build with mock backend
#[test]
fn cli_build_succeeds_with_mock() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "build",
            &manifest.to_string_lossy(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "build must exit 0. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_pin_check_fails_when_unpinned() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_minimal_manifest(project.path(), "rolling");

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "pin",
            &manifest.to_string_lossy(),
            "--check",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "pin --check must fail for unpinned base.image"
    );
}

#[test]
fn cli_pin_check_succeeds_when_pinned_url() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_minimal_manifest(project.path(), "https://example.invalid/rootfs.tar.xz");

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "pin",
            &manifest.to_string_lossy(),
            "--check",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "pin --check must exit 0 when already pinned"
    );
}

#[test]
fn cli_build_offline_fails_fast_with_packages() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "build",
            &manifest.to_string_lossy(),
            "--offline",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "build --offline with packages must fail fast"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("offline mode") && stderr.contains("system packages"),
        "stderr must mention offline + system packages, got: {stderr}"
    );
}

#[test]
fn cli_snapshots_restore_hash_matches_commit() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_minimal_manifest(project.path(), "rolling");

    let build_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "build",
            &manifest.to_string_lossy(),
            "--name",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(build_out.status.success());

    let commit_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "commit",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(
        commit_out.status.success(),
        "commit must exit 0. stderr: {}",
        String::from_utf8_lossy(&commit_out.stderr)
    );
    let commit_stdout = String::from_utf8_lossy(&commit_out.stdout);
    let commit_json: serde_json::Value = serde_json::from_str(&commit_stdout)
        .unwrap_or_else(|e| panic!("commit --json must produce valid JSON: {e}\n{commit_stdout}"));
    let commit_hash = commit_json["snapshot_hash"].as_str().unwrap().to_owned();

    let snaps_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "snapshots",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(
        snaps_out.status.success(),
        "snapshots must exit 0. stderr: {}",
        String::from_utf8_lossy(&snaps_out.stderr)
    );
    let snaps_stdout = String::from_utf8_lossy(&snaps_out.stdout);
    let snaps_json: serde_json::Value = serde_json::from_str(&snaps_stdout).unwrap_or_else(|e| {
        panic!("snapshots --json must produce valid JSON: {e}\nstdout: {snaps_stdout}")
    });
    let restore_hash = snaps_json["snapshots"][0]["restore_hash"].as_str().unwrap();
    assert_eq!(restore_hash, commit_hash);

    let restore_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "restore",
            "demo",
            restore_hash,
        ])
        .output()
        .unwrap();
    assert!(
        restore_out.status.success(),
        "restore must exit 0. stderr: {}",
        String::from_utf8_lossy(&restore_out.stderr)
    );
}

// A5: CLI Validation — list with JSON output
#[test]
fn cli_list_json_output_stable() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());

    // Build first
    let build_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "build",
            &manifest.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(build_out.status.success());

    // List with --json
    let output = karapace_bin()
        .args(["--store", &store.path().to_string_lossy(), "--json", "list"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "list --json must exit 0. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Must be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("list --json must produce valid JSON: {e}\nstdout: {stdout}"));
    assert!(parsed.is_array(), "list output must be a JSON array");
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 1, "should have exactly 1 environment");
    // Verify expected fields exist
    assert!(arr[0]["env_id"].is_string());
    assert!(arr[0]["state"].is_string());
}

// A5: CLI Validation — inspect with JSON output
#[test]
fn cli_inspect_json_output_stable() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());

    // Build first
    let build_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "build",
            &manifest.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(build_out.status.success());

    // Parse build JSON to get env_id
    let build_stdout = String::from_utf8_lossy(&build_out.stdout);
    let build_json: serde_json::Value = serde_json::from_str(&build_stdout)
        .unwrap_or_else(|e| panic!("build --json must produce valid JSON: {e}\n{build_stdout}"));
    let env_id = build_json["env_id"].as_str().unwrap();

    // Inspect
    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "inspect",
            env_id,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let inspect_json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("inspect --json must produce valid JSON: {e}\n{stdout}"));
    assert_eq!(inspect_json["env_id"].as_str().unwrap(), env_id);
    assert_eq!(inspect_json["state"].as_str().unwrap(), "Built");
}

// A5: CLI Validation — destroy succeeds
#[test]
fn cli_destroy_succeeds() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());

    // Build
    let build_out = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "build",
            &manifest.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(build_out.status.success());
    let build_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&build_out.stdout)).unwrap();
    let env_id = build_json["env_id"].as_str().unwrap();

    // Destroy
    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "destroy",
            env_id,
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "destroy must exit 0. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// A5: CLI Validation — build with nonexistent manifest fails
#[test]
fn cli_build_nonexistent_manifest_fails() {
    let store = temp_store();

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "build",
            "/tmp/nonexistent_karapace_manifest_12345.toml",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "build with missing manifest must fail"
    );
}

// A5: CLI Validation — gc on empty store succeeds
#[test]
fn cli_gc_empty_store_succeeds() {
    let store = temp_store();

    // Initialize store first via a build
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());
    let _ = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "build",
            &manifest.to_string_lossy(),
        ])
        .output()
        .unwrap();

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "gc",
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "gc --dry-run must exit 0. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// A5: CLI Validation — verify-store on clean store
#[test]
fn cli_verify_store_clean() {
    let store = temp_store();
    let project = tempfile::tempdir().unwrap();
    let manifest = write_test_manifest(project.path());

    // Build to populate store
    let _ = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "build",
            &manifest.to_string_lossy(),
        ])
        .output()
        .unwrap();

    let output = karapace_bin()
        .args([
            "--store",
            &store.path().to_string_lossy(),
            "--json",
            "verify-store",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "verify-store must exit 0. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("verify-store --json must produce valid JSON: {e}\n{stdout}"));
    assert_eq!(json["failed"].as_u64().unwrap(), 0);
}
