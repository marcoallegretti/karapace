use super::{resolve_env_id_pretty, EXIT_SUCCESS};
use karapace_core::{Engine, StoreLock};
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(engine: &Engine, store_path: &Path, env_id: &str) -> Result<u8, String> {
    let layout = StoreLayout::new(store_path);
    let _lock = StoreLock::acquire(&layout.lock_file()).map_err(|e| format!("store lock: {e}"))?;

    let resolved = resolve_env_id_pretty(engine, env_id)?;
    engine.stop(&resolved).map_err(|e| e.to_string())?;
    println!("stopped environment {env_id}");
    Ok(EXIT_SUCCESS)
}
