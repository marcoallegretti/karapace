use super::{json_pretty, spin_fail, spin_ok, spinner, EXIT_SUCCESS};
use karapace_core::{Engine, StoreLock};
use karapace_store::StoreLayout;
use std::path::Path;

pub fn run(
    engine: &Engine,
    store_path: &Path,
    manifest: &Path,
    name: Option<&str>,
    json: bool,
) -> Result<u8, String> {
    let layout = StoreLayout::new(store_path);
    let _lock = StoreLock::acquire(&layout.lock_file()).map_err(|e| format!("store lock: {e}"))?;

    let pb = if json {
        None
    } else {
        Some(spinner("building environment..."))
    };
    let result = match engine.build(manifest) {
        Ok(r) => {
            if let Some(ref pb) = pb {
                spin_ok(pb, "environment built");
            }
            r
        }
        Err(e) => {
            if let Some(ref pb) = pb {
                spin_fail(pb, "build failed");
            }
            return Err(e.to_string());
        }
    };
    if let Some(n) = name {
        engine
            .set_name(&result.identity.env_id, Some(n.to_owned()))
            .map_err(|e| e.to_string())?;
    }
    if json {
        let payload = serde_json::json!({
            "env_id": result.identity.env_id,
            "short_id": result.identity.short_id,
            "name": name,
            "status": "built"
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        if let Some(n) = name {
            println!("built environment '{}' ({})", n, result.identity.short_id);
        } else {
            println!("built environment {}", result.identity.short_id);
        }
        println!("env_id: {}", result.identity.env_id);
    }
    Ok(EXIT_SUCCESS)
}
