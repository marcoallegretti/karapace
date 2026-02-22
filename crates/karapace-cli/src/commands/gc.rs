use super::{json_pretty, EXIT_SUCCESS};
use karapace_core::{Engine, StoreLock};
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(engine: &Engine, store_path: &Path, dry_run: bool, json: bool) -> Result<u8, String> {
    let layout = StoreLayout::new(store_path);
    let lock = StoreLock::acquire(&layout.lock_file()).map_err(|e| format!("store lock: {e}"))?;

    let report = engine.gc(&lock, dry_run).map_err(|e| e.to_string())?;
    if json {
        let payload = serde_json::json!({
            "dry_run": dry_run,
            "orphaned_envs": report.orphaned_envs,
            "orphaned_layers": report.orphaned_layers,
            "orphaned_objects": report.orphaned_objects,
            "removed_envs": report.removed_envs,
            "removed_layers": report.removed_layers,
            "removed_objects": report.removed_objects,
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        let prefix = if dry_run { "would remove" } else { "removed" };
        println!(
            "gc: {prefix} {} envs, {} layers, {} objects",
            report.removed_envs, report.removed_layers, report.removed_objects
        );
        if dry_run && !report.orphaned_envs.is_empty() {
            println!("orphaned envs: {:?}", report.orphaned_envs);
        }
    }
    Ok(EXIT_SUCCESS)
}
