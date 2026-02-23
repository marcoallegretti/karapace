use super::{resolve_env_id_pretty, EXIT_SUCCESS};
use karapace_core::{Engine, StoreLock};
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(engine: &Engine, store_path: &Path, env_id: &str, new_name: &str) -> Result<u8, String> {
    let layout = StoreLayout::new(store_path);
    let _lock = StoreLock::acquire(&layout.lock_file()).map_err(|e| format!("store lock: {e}"))?;

    let resolved = resolve_env_id_pretty(engine, env_id)?;
    engine
        .rename(&resolved, new_name)
        .map_err(|e| e.to_string())?;
    println!("renamed {} → '{}'", &resolved[..12], new_name);
    Ok(EXIT_SUCCESS)
}
