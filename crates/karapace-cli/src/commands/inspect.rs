use super::{colorize_state, json_pretty, resolve_env_id, resolve_env_id_pretty, EXIT_SUCCESS};
use karapace_core::Engine;

pub fn run(engine: &Engine, env_id: &str, json: bool) -> Result<u8, String> {
    let resolved = if json {
        resolve_env_id(engine, env_id)?
    } else {
        resolve_env_id_pretty(engine, env_id)?
    };
    let meta = engine.inspect(&resolved).map_err(|e| e.to_string())?;
    if json {
        println!("{}", json_pretty(&meta)?);
    } else {
        println!("env_id:      {}", meta.env_id);
        println!("short_id:    {}", meta.short_id);
        println!("name:        {}", meta.name.as_deref().unwrap_or("(none)"));
        println!("state:       {}", colorize_state(&meta.state.to_string()));
        println!("base_layer:  {}", meta.base_layer);
        println!("deps:        {}", meta.dependency_layers.len());
        println!("ref_count:   {}", meta.ref_count);
        println!("created_at:  {}", meta.created_at);
        println!("updated_at:  {}", meta.updated_at);
    }
    Ok(EXIT_SUCCESS)
}
