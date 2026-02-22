use super::{json_pretty, resolve_env_id, EXIT_SUCCESS};
use karapace_core::Engine;
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(engine: &Engine, store_path: &Path, env_id: &str, json: bool) -> Result<u8, String> {
    let _layout = StoreLayout::new(store_path);

    let resolved = resolve_env_id(engine, env_id)?;
    let snapshots = engine
        .list_snapshots(&resolved)
        .map_err(|e| e.to_string())?;

    if json {
        let entries: Vec<_> = snapshots
            .iter()
            .map(|s| {
                serde_json::json!({
                    "hash": s.hash,
                    "tar_hash": s.tar_hash,
                    "parent": s.parent,
                })
            })
            .collect();
        let payload = serde_json::json!({
            "env_id": resolved,
            "snapshots": entries,
        });
        println!("{}", json_pretty(&payload)?);
    } else if snapshots.is_empty() {
        println!("no snapshots for {env_id}");
    } else {
        println!("snapshots for {env_id}:");
        for s in &snapshots {
            println!("  {} (tar: {})", &s.hash[..12], &s.tar_hash[..12]);
        }
    }
    Ok(EXIT_SUCCESS)
}
