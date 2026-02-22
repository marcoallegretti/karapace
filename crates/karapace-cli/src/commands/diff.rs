use super::{json_pretty, resolve_env_id, EXIT_SUCCESS};
use karapace_core::Engine;

pub fn run(engine: &Engine, env_id: &str, json: bool) -> Result<u8, String> {
    let resolved = resolve_env_id(engine, env_id)?;
    let report =
        karapace_core::diff_overlay(engine.store_layout(), &resolved).map_err(|e| e.to_string())?;

    if json {
        println!("{}", json_pretty(&report)?);
    } else if report.has_drift {
        println!("drift detected in environment {env_id}:");
        for f in &report.added {
            println!("  + {f}");
        }
        for f in &report.modified {
            println!("  ~ {f}");
        }
        for f in &report.removed {
            println!("  - {f}");
        }
    } else {
        println!("no drift detected in environment {env_id}");
    }
    Ok(EXIT_SUCCESS)
}
