use super::{EXIT_FAILURE, EXIT_SUCCESS};
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(store_path: &Path, json_output: bool) -> Result<u8, String> {
    let mut checks: Vec<Check> = Vec::new();
    let mut all_pass = true;

    check_prereqs(&mut checks, &mut all_pass);

    let layout = StoreLayout::new(store_path);
    if store_path.join("store").exists() {
        checks.push(Check::pass("store_exists", "Store directory exists"));
        check_store(&layout, &mut checks, &mut all_pass);
        check_disk_space(store_path, &mut checks);
    } else {
        checks.push(Check::info(
            "store_exists",
            "Store not initialized (will be created on first build)",
        ));
    }

    print_results(&checks, all_pass, json_output)
}

fn check_prereqs(checks: &mut Vec<Check>, all_pass: &mut bool) {
    let missing = karapace_runtime::check_namespace_prereqs();
    if missing.is_empty() {
        checks.push(Check::pass(
            "runtime_prereqs",
            "Runtime prerequisites satisfied",
        ));
    } else {
        *all_pass = false;
        checks.push(Check::fail(
            "runtime_prereqs",
            &format!(
                "Missing prerequisites: {}",
                karapace_runtime::format_missing(&missing)
            ),
        ));
    }
}

fn check_store(layout: &StoreLayout, checks: &mut Vec<Check>, all_pass: &mut bool) {
    // Version
    match layout.initialize() {
        Ok(()) => checks.push(Check::pass("store_version", "Store format version valid")),
        Err(e) => {
            *all_pass = false;
            checks.push(Check::fail(
                "store_version",
                &format!("Store version check failed: {e}"),
            ));
        }
    }

    // Integrity
    match karapace_store::verify_store_integrity(layout) {
        Ok(report) if report.failed.is_empty() => {
            checks.push(Check::pass(
                "store_integrity",
                &format!("Store integrity OK ({} objects checked)", report.checked),
            ));
        }
        Ok(report) => {
            *all_pass = false;
            checks.push(Check::fail(
                "store_integrity",
                &format!(
                    "{} of {} objects corrupted",
                    report.failed.len(),
                    report.checked
                ),
            ));
        }
        Err(e) => {
            *all_pass = false;
            checks.push(Check::fail(
                "store_integrity",
                &format!("Integrity check failed: {e}"),
            ));
        }
    }

    // WAL
    let wal = karapace_store::WriteAheadLog::new(layout);
    match wal.list_incomplete() {
        Ok(entries) if entries.is_empty() => {
            checks.push(Check::pass(
                "wal_clean",
                "WAL is clean (no incomplete entries)",
            ));
        }
        Ok(entries) => {
            checks.push(Check::warn(
                "wal_clean",
                &format!(
                    "WAL has {} incomplete entries (will recover on next start)",
                    entries.len()
                ),
            ));
        }
        Err(e) => checks.push(Check::warn("wal_clean", &format!("Cannot read WAL: {e}"))),
    }

    // Lock
    match karapace_core::StoreLock::try_acquire(&layout.lock_file()) {
        Ok(Some(_)) => checks.push(Check::pass("store_lock", "Store lock is free")),
        Ok(None) => checks.push(Check::warn(
            "store_lock",
            "Store lock is held by another process",
        )),
        Err(e) => {
            *all_pass = false;
            checks.push(Check::fail(
                "store_lock",
                &format!("Cannot check store lock: {e}"),
            ));
        }
    }

    // Environments
    let meta_store = karapace_store::MetadataStore::new(layout.clone());
    match meta_store.list() {
        Ok(envs) => {
            let running = envs
                .iter()
                .filter(|e| e.state == karapace_store::EnvState::Running)
                .count();
            checks.push(Check::info(
                "environments",
                &format!("{} environments ({running} running)", envs.len()),
            ));
        }
        Err(e) => checks.push(Check::warn(
            "environments",
            &format!("Cannot list environments: {e}"),
        )),
    }
}

fn print_results(checks: &[Check], all_pass: bool, json_output: bool) -> Result<u8, String> {
    if json_output {
        let json = serde_json::json!({
            "healthy": all_pass,
            "checks": checks.iter().map(|c| serde_json::json!({
                "name": c.name,
                "status": c.status,
                "message": c.message,
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?
        );
    } else {
        println!("Karapace Doctor\n");
        for check in checks {
            let icon = match check.status.as_str() {
                "pass" => "✓",
                "fail" => "✗",
                "warn" => "⚠",
                _ => "ℹ",
            };
            println!("  {icon} {}", check.message);
        }
        println!();
        if all_pass {
            println!("All checks passed.");
        } else {
            println!("Some checks failed. See above for details.");
        }
    }
    Ok(if all_pass { EXIT_SUCCESS } else { EXIT_FAILURE })
}

struct Check {
    name: String,
    status: String,
    message: String,
}

impl Check {
    fn pass(name: &str, message: &str) -> Self {
        Self {
            name: name.to_owned(),
            status: "pass".to_owned(),
            message: message.to_owned(),
        }
    }

    fn fail(name: &str, message: &str) -> Self {
        Self {
            name: name.to_owned(),
            status: "fail".to_owned(),
            message: message.to_owned(),
        }
    }

    fn warn(name: &str, message: &str) -> Self {
        Self {
            name: name.to_owned(),
            status: "warn".to_owned(),
            message: message.to_owned(),
        }
    }

    fn info(name: &str, message: &str) -> Self {
        Self {
            name: name.to_owned(),
            status: "info".to_owned(),
            message: message.to_owned(),
        }
    }
}

fn check_disk_space(store_path: &Path, checks: &mut Vec<Check>) {
    let Ok(c_path) = std::ffi::CString::new(store_path.to_string_lossy().as_bytes()) else {
        return;
    };

    // SAFETY: zeroed statvfs is a valid initial state for the struct.
    #[allow(unsafe_code, clippy::undocumented_unsafe_blocks)]
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: statvfs with a valid, NUL-terminated path and a properly
    // zeroed output struct is well-defined. The struct is stack-allocated
    // and only read after the call succeeds (ret == 0).
    #[allow(unsafe_code, clippy::undocumented_unsafe_blocks)]
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &raw mut stat) };
    if ret != 0 {
        return;
    }

    let avail_bytes = stat.f_bavail * stat.f_frsize;
    let avail_mb = avail_bytes / (1024 * 1024);

    if avail_mb < 100 {
        checks.push(Check::fail(
            "disk_space",
            &format!("Low disk space: {avail_mb} MB available"),
        ));
    } else if avail_mb < 1024 {
        checks.push(Check::warn(
            "disk_space",
            &format!("Disk space: {avail_mb} MB available (consider freeing space)"),
        ));
    } else {
        let free_gb = avail_mb / 1024;
        checks.push(Check::pass(
            "disk_space",
            &format!("Disk space: {free_gb} GB available"),
        ));
    }
}
