use super::{
    json_pretty, make_remote_backend, resolve_env_id, spin_fail, spin_ok, spinner, EXIT_SUCCESS,
};
use karapace_core::Engine;

pub fn run(
    engine: &Engine,
    env_id: &str,
    tag: Option<&str>,
    remote_url: Option<&str>,
    json: bool,
) -> Result<u8, String> {
    let resolved = resolve_env_id(engine, env_id)?;
    let backend = make_remote_backend(remote_url)?;

    let pb = spinner("pushing environment…");
    let result = engine.push(&resolved, &backend, tag).map_err(|e| {
        spin_fail(&pb, "push failed");
        e.to_string()
    })?;
    spin_ok(&pb, "push complete");

    if json {
        let payload = serde_json::json!({
            "env_id": resolved,
            "tag": tag,
            "objects_pushed": result.objects_pushed,
            "layers_pushed": result.layers_pushed,
            "objects_skipped": result.objects_skipped,
            "layers_skipped": result.layers_skipped,
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        println!(
            "pushed {} ({} objects, {} layers; {} skipped)",
            &resolved[..12],
            result.objects_pushed,
            result.layers_pushed,
            result.objects_skipped + result.layers_skipped,
        );
        if let Some(t) = tag {
            println!("tagged as '{t}'");
        }
    }
    Ok(EXIT_SUCCESS)
}
