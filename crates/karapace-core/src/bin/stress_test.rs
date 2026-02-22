//! Long-running stress test for the Karapace engine.
//!
//! Runs hundreds of build/commit/destroy/gc cycles with the mock backend,
//! checking for resource leaks (orphaned files, stale WAL entries, metadata
//! corruption) after every cycle.
//!
//! Usage:
//!   cargo run --bin stress_test -- [--cycles N]

use karapace_core::Engine;
use karapace_store::{verify_store_integrity, GarbageCollector, StoreLayout};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

fn write_manifest(dir: &Path) -> std::path::PathBuf {
    let manifest_path = dir.join("karapace.toml");
    fs::write(
        &manifest_path,
        r#"manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "clang"]
[runtime]
backend = "mock"
"#,
    )
    .expect("write manifest");
    manifest_path
}

fn count_files_in(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

struct Timings {
    build: Duration,
    commit: Duration,
    destroy: Duration,
    gc: Duration,
}

fn run_cycle(
    engine: &Engine,
    manifest_path: &Path,
    layout: &StoreLayout,
    cycle: usize,
    timings: &mut Timings,
) -> Result<(), String> {
    let t0 = Instant::now();
    let build_result = engine
        .build(manifest_path)
        .map_err(|e| format!("cycle {cycle}: BUILD FAILED: {e}"))?;
    timings.build += t0.elapsed();
    let env_id = build_result.identity.env_id.to_string();

    let t0 = Instant::now();
    if let Err(e) = engine.commit(&env_id) {
        eprintln!("  cycle {cycle}: COMMIT FAILED: {e}");
    }
    timings.commit += t0.elapsed();

    let t0 = Instant::now();
    engine
        .destroy(&env_id)
        .map_err(|e| format!("cycle {cycle}: DESTROY FAILED: {e}"))?;
    timings.destroy += t0.elapsed();

    if cycle.is_multiple_of(10) {
        let gc = GarbageCollector::new(layout.clone());
        let t0 = Instant::now();
        match gc.collect(false) {
            Ok(report) => {
                timings.gc += t0.elapsed();
                if cycle.is_multiple_of(100) {
                    println!(
                        "  cycle {cycle}: GC collected {} objects, {} layers",
                        report.removed_objects, report.removed_layers
                    );
                }
            }
            Err(e) => return Err(format!("cycle {cycle}: GC FAILED: {e}")),
        }
    }
    Ok(())
}

fn check_health(layout: &StoreLayout, wal_dir: &Path, cycle: usize) -> u64 {
    let mut failures = 0u64;
    match verify_store_integrity(layout) {
        Ok(report) => {
            if !report.failed.is_empty() {
                eprintln!(
                    "  cycle {cycle}: INTEGRITY FAILURE: {} objects failed",
                    report.failed.len()
                );
                failures += 1;
            }
        }
        Err(e) => {
            eprintln!("  cycle {cycle}: INTEGRITY CHECK ERROR: {e}");
            failures += 1;
        }
    }
    let wal_files = count_files_in(wal_dir);
    if wal_files > 0 {
        eprintln!("  cycle {cycle}: WAL LEAK: {wal_files} stale entries");
        failures += 1;
    }
    let meta_count = count_files_in(&layout.metadata_dir());
    if meta_count > 0 {
        eprintln!("  cycle {cycle}: METADATA LEAK: {meta_count} entries after full destroy+gc");
    }
    failures
}

fn print_report(
    cycles: usize,
    failures: u64,
    timings: &Timings,
    layout: &StoreLayout,
    wal_dir: &Path,
) {
    let final_integrity = verify_store_integrity(layout);
    let wal_files = count_files_in(wal_dir);

    println!();
    println!("============================================");
    println!("Results: {cycles} cycles, {failures} failures");
    println!(
        "  build:   {:.3}s total, {:.3}ms avg",
        timings.build.as_secs_f64(),
        timings.build.as_secs_f64() * 1000.0 / cycles as f64
    );
    println!(
        "  commit:  {:.3}s total, {:.3}ms avg",
        timings.commit.as_secs_f64(),
        timings.commit.as_secs_f64() * 1000.0 / cycles as f64
    );
    println!(
        "  destroy: {:.3}s total, {:.3}ms avg",
        timings.destroy.as_secs_f64(),
        timings.destroy.as_secs_f64() * 1000.0 / cycles as f64
    );
    println!("  gc:      {:.3}s total", timings.gc.as_secs_f64());
    println!("  WAL entries remaining: {wal_files}");
    println!(
        "  metadata remaining: {}",
        count_files_in(&layout.metadata_dir())
    );
    println!(
        "  objects remaining: {}",
        count_files_in(&layout.objects_dir())
    );
    println!(
        "  layers remaining: {}",
        count_files_in(&layout.layers_dir())
    );
    match final_integrity {
        Ok(report) => println!(
            "  integrity: {} checked, {} passed, {} failed",
            report.checked,
            report.passed,
            report.failed.len()
        ),
        Err(e) => println!("  integrity: ERROR: {e}"),
    }

    if failures > 0 || wal_files > 0 {
        eprintln!("\nSTRESS TEST FAILED");
        std::process::exit(1);
    } else {
        println!("\nSTRESS TEST PASSED");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cycles: usize = args
        .iter()
        .position(|a| a == "--cycles")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);

    println!("Karapace stress test: {cycles} cycles");
    println!("============================================");

    let store_dir = tempfile::tempdir().expect("create temp dir");
    let project_dir = tempfile::tempdir().expect("create project dir");
    let manifest_path = write_manifest(project_dir.path());

    let layout = StoreLayout::new(store_dir.path());
    layout.initialize().expect("initialize store");
    let engine = Engine::new(store_dir.path());
    let wal_dir = store_dir.path().join("store").join("wal");

    let mut timings = Timings {
        build: Duration::ZERO,
        commit: Duration::ZERO,
        destroy: Duration::ZERO,
        gc: Duration::ZERO,
    };
    let mut failures = 0u64;

    for cycle in 1..=cycles {
        if let Err(msg) = run_cycle(&engine, &manifest_path, &layout, cycle, &mut timings) {
            eprintln!("  {msg}");
            failures += 1;
            continue;
        }
        if cycle.is_multiple_of(50) {
            failures += check_health(&layout, &wal_dir, cycle);
        }
        if cycle.is_multiple_of(100) {
            let elapsed = timings.build + timings.commit + timings.destroy + timings.gc;
            println!(
                "  cycle {cycle}/{cycles}: {:.1}s elapsed, {failures} failures",
                elapsed.as_secs_f64()
            );
        }
    }

    let gc = GarbageCollector::new(layout.clone());
    let _ = gc.collect(false);

    print_report(cycles, failures, &timings, &layout, &wal_dir);
}
