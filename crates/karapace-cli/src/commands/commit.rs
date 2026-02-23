use super::{json_pretty, resolve_env_id, resolve_env_id_pretty, EXIT_SUCCESS};
use karapace_core::{Engine, StoreLock};
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(engine: &Engine, store_path: &Path, env_id: &str, json: bool) -> Result<u8, String> {
    let layout = StoreLayout::new(store_path);
    let _lock = StoreLock::acquire(&layout.lock_file()).map_err(|e| format!("store lock: {e}"))?;

    let resolved = if json {
        resolve_env_id(engine, env_id)?
    } else {
        resolve_env_id_pretty(engine, env_id)?
    };
    let tar_hash = engine.commit(&resolved).map_err(|e| e.to_string())?;
    if json {
        let payload = serde_json::json!({
            "env_id": resolved,
            "snapshot_hash": tar_hash,
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        println!("committed snapshot {tar_hash} for {env_id}");
    }
    Ok(EXIT_SUCCESS)
}
