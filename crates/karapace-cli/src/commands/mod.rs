pub mod archive;
pub mod build;
pub mod commit;
pub mod completions;
pub mod destroy;
pub mod diff;
pub mod doctor;
pub mod enter;
pub mod exec;
pub mod freeze;
pub mod gc;
pub mod inspect;
pub mod list;
pub mod man_pages;
pub mod migrate;
pub mod pull;
pub mod push;
pub mod rebuild;
pub mod rename;
pub mod restore;
pub mod snapshots;
pub mod stop;
pub mod verify_store;

use indicatif::{ProgressBar, ProgressStyle};
use karapace_core::Engine;
use std::time::Duration;

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_FAILURE: u8 = 1;
pub const EXIT_MANIFEST_ERROR: u8 = 2;
pub const EXIT_STORE_ERROR: u8 = 3;

pub fn json_pretty(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|e| format!("JSON serialization failed: {e}"))
}

pub fn spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .expect("valid template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    pb.set_message(msg.to_owned());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn spin_ok(pb: &ProgressBar, msg: &str) {
    pb.set_style(ProgressStyle::with_template("{msg}").expect("valid template"));
    pb.finish_with_message(format!("✓ {msg}"));
}

pub fn spin_fail(pb: &ProgressBar, msg: &str) {
    pb.set_style(ProgressStyle::with_template("{msg}").expect("valid template"));
    pb.finish_with_message(format!("✗ {msg}"));
}

pub fn colorize_state(state: &str) -> String {
    use console::Style;
    match state {
        "built" => Style::new().green().apply_to(state).to_string(),
        "running" => Style::new().cyan().bold().apply_to(state).to_string(),
        "defined" => Style::new().yellow().apply_to(state).to_string(),
        "frozen" => Style::new().blue().apply_to(state).to_string(),
        "archived" => Style::new().dim().apply_to(state).to_string(),
        other => other.to_owned(),
    }
}

pub fn resolve_env_id(engine: &Engine, input: &str) -> Result<String, String> {
    if input.len() == 64 {
        return Ok(input.to_owned());
    }

    let envs = engine.list().map_err(|e| e.to_string())?;

    for e in &envs {
        if *e.env_id == *input || *e.short_id == *input || e.name.as_deref() == Some(input) {
            return Ok(e.env_id.to_string());
        }
    }

    let matches: Vec<_> = envs
        .iter()
        .filter(|e| e.env_id.starts_with(input) || e.short_id.starts_with(input))
        .collect();

    match matches.len() {
        0 => Err(format!("no environment matching '{input}'")),
        1 => Ok(matches[0].env_id.to_string()),
        n => Err(format!(
            "ambiguous env_id prefix '{input}': matches {n} environments"
        )),
    }
}

pub fn make_remote_backend(
    remote_url: Option<&str>,
) -> Result<karapace_remote::http::HttpBackend, String> {
    let config = if let Some(url) = remote_url {
        karapace_remote::RemoteConfig::new(url)
    } else {
        karapace_remote::RemoteConfig::load_default()
            .map_err(|e| format!("no --remote and no config: {e}"))?
    };
    Ok(karapace_remote::http::HttpBackend::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_pretty_serializes_string() {
        let val = serde_json::json!({"key": "value"});
        let result = json_pretty(&val).unwrap();
        assert!(result.contains("\"key\""));
        assert!(result.contains("\"value\""));
    }

    #[test]
    fn json_pretty_serializes_array() {
        let val = vec![1, 2, 3];
        let result = json_pretty(&val).unwrap();
        assert!(result.contains('1'));
    }

    #[test]
    fn colorize_state_built() {
        let result = colorize_state("built");
        assert!(result.contains("built"));
    }

    #[test]
    fn colorize_state_running() {
        let result = colorize_state("running");
        assert!(result.contains("running"));
    }

    #[test]
    fn colorize_state_defined() {
        let result = colorize_state("defined");
        assert!(result.contains("defined"));
    }

    #[test]
    fn colorize_state_frozen() {
        let result = colorize_state("frozen");
        assert!(result.contains("frozen"));
    }

    #[test]
    fn colorize_state_archived() {
        let result = colorize_state("archived");
        assert!(result.contains("archived"));
    }

    #[test]
    fn colorize_state_unknown() {
        assert_eq!(colorize_state("unknown"), "unknown");
    }

    #[test]
    fn resolve_env_id_64_char_passthrough() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(dir.path());
        let id = "a".repeat(64);
        assert_eq!(resolve_env_id(&engine, &id).unwrap(), id);
    }

    #[test]
    fn resolve_env_id_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(dir.path());
        karapace_store::StoreLayout::new(dir.path())
            .initialize()
            .unwrap();
        let result = resolve_env_id(&engine, "nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no environment matching"));
    }

    #[test]
    fn exit_codes_are_distinct() {
        assert_ne!(EXIT_SUCCESS, EXIT_FAILURE);
        assert_ne!(EXIT_FAILURE, EXIT_MANIFEST_ERROR);
        assert_ne!(EXIT_MANIFEST_ERROR, EXIT_STORE_ERROR);
    }

    #[test]
    fn make_remote_backend_with_url() {
        let backend = make_remote_backend(Some("http://localhost:8080"));
        assert!(backend.is_ok());
    }

    #[test]
    fn spinner_creates_progress_bar() {
        let pb = spinner("testing...");
        spin_ok(&pb, "done");
    }

    #[test]
    fn spinner_fail_creates_progress_bar() {
        let pb = spinner("testing...");
        spin_fail(&pb, "failed");
    }
}
