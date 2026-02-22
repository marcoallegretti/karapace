use super::{json_pretty, make_remote_backend, spin_fail, spin_ok, spinner, EXIT_SUCCESS};
use karapace_core::Engine;

pub fn run(
    engine: &Engine,
    reference: &str,
    remote_url: Option<&str>,
    json: bool,
) -> Result<u8, String> {
    let backend = make_remote_backend(remote_url)?;

    // Resolve reference: try as registry ref first, fall back to raw env_id
    let env_id = match Engine::resolve_remote_ref(&backend, reference) {
        Ok(id) => id,
        Err(_) => reference.to_owned(),
    };

    let pb = spinner("pulling environment…");
    let result = engine.pull(&env_id, &backend).map_err(|e| {
        spin_fail(&pb, "pull failed");
        e.to_string()
    })?;
    spin_ok(&pb, "pull complete");

    if json {
        let payload = serde_json::json!({
            "env_id": env_id,
            "objects_pulled": result.objects_pulled,
            "layers_pulled": result.layers_pulled,
            "objects_skipped": result.objects_skipped,
            "layers_skipped": result.layers_skipped,
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        println!(
            "pulled {} ({} objects, {} layers; {} skipped)",
            &env_id[..12.min(env_id.len())],
            result.objects_pulled,
            result.layers_pulled,
            result.objects_skipped + result.layers_skipped,
        );
    }
    Ok(EXIT_SUCCESS)
}
