use super::{colorize_state, json_pretty, EXIT_SUCCESS};
use karapace_core::Engine;

pub fn run(engine: &Engine, json: bool) -> Result<u8, String> {
    let envs = engine.list().map_err(|e| e.to_string())?;
    if json {
        println!("{}", json_pretty(&envs)?);
    } else if envs.is_empty() {
        println!("no environments found");
    } else {
        println!("{:<14} {:<16} {:<10} ENV_ID", "SHORT_ID", "NAME", "STATE");
        for env in &envs {
            let name_display = env.name.as_deref().unwrap_or("");
            let state_str = colorize_state(&env.state.to_string());
            println!(
                "{:<14} {:<16} {:<10} {}",
                env.short_id, name_display, state_str, env.env_id
            );
        }
    }
    Ok(EXIT_SUCCESS)
}
