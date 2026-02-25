use super::{json_pretty, resolve_env_id, resolve_env_id_pretty, EXIT_SUCCESS};
use karapace_core::Engine;
use karapace_store::{LayerStore, StoreLayout};
use std::path::Path;

pub fn run(engine: &Engine, store_path: &Path, env_id: &str, json: bool) -> Result<u8, String> {
    let _layout = StoreLayout::new(store_path);

    let resolved = if json {
        resolve_env_id(engine, env_id)?
    } else {
        resolve_env_id_pretty(engine, env_id)?
    };
    let snapshots = engine
        .list_snapshots(&resolved)
        .map_err(|e| e.to_string())?;

    if json {
        let mut entries = Vec::new();
        for s in &snapshots {
            let restore_hash = LayerStore::compute_hash(s).map_err(|e| e.to_string())?;
            entries.push(serde_json::json!({
                "hash": s.hash,
                "restore_hash": restore_hash,
                "tar_hash": s.tar_hash,
                "parent": s.parent,
            }));
        }
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
            let restore_hash = LayerStore::compute_hash(s).map_err(|e| e.to_string())?;
            println!("  {} (tar: {})", restore_hash, &s.tar_hash[..12]);
        }
    }
    Ok(EXIT_SUCCESS)
}
