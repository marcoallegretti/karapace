use super::EXIT_SUCCESS;
use std::path::Path;

pub fn run(store_path: &Path, json: bool) -> Result<u8, String> {
    if json {
        return Err("JSON output is not supported for 'tui'".to_owned());
    }
    karapace_tui::run(store_path)?;
    Ok(EXIT_SUCCESS)
}
