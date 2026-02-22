use super::{EXIT_FAILURE, EXIT_SUCCESS};
use std::path::Path;

pub fn run(store_path: &Path, json_output: bool) -> Result<u8, String> {
    let store_dir = store_path.join("store");
    if !store_dir.exists() {
        msg(
            json_output,
            r#"{"status": "no_store", "message": "No store found."}"#,
            &format!(
                "No store found at {}. Nothing to migrate.",
                store_path.display()
            ),
        );
        return Ok(EXIT_SUCCESS);
    }

    let version_path = store_dir.join("version");
    if !version_path.exists() {
        msg(json_output,
            r#"{"status": "error", "message": "Store exists but has no version file."}"#,
            &format!("Store at {} has no version file. May be corrupted or very old.\nRecommended: back up and rebuild.", store_path.display()));
        return Ok(EXIT_FAILURE);
    }

    let content = std::fs::read_to_string(&version_path)
        .map_err(|e| format!("failed to read version file: {e}"))?;
    let ver: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("invalid version file: {e}"))?;
    let found = ver
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let current = u64::from(karapace_store::STORE_FORMAT_VERSION);

    if found == current {
        msg(
            json_output,
            &format!(r#"{{"status": "current", "format_version": {current}}}"#),
            &format!("Store format version: {current} (current)\nNo migration needed."),
        );
        return Ok(EXIT_SUCCESS);
    }

    if found > current {
        msg(json_output,
            &format!(r#"{{"status": "newer", "found": {found}, "supported": {current}}}"#),
            &format!("Store format version: {found}\nSupported: {current}\n\nCreated by a newer Karapace. Please upgrade."));
        return Ok(EXIT_FAILURE);
    }

    // Attempt automatic migration
    match karapace_store::migrate_store(store_path) {
        Ok(Some(result)) => {
            msg(
                json_output,
                &format!(
                    r#"{{"status": "migrated", "from": {}, "to": {}, "environments": {}, "backup": "{}"}}"#,
                    result.from_version,
                    result.to_version,
                    result.environments_migrated,
                    result.backup_path.display()
                ),
                &format!(
                    "Migrated store from v{} to v{}.\n{} environments updated.\nBackup: {}",
                    result.from_version,
                    result.to_version,
                    result.environments_migrated,
                    result.backup_path.display()
                ),
            );
            Ok(EXIT_SUCCESS)
        }
        Ok(None) => {
            // Should not reach here (we already checked found != current above)
            msg(
                json_output,
                &format!(r#"{{"status": "current", "format_version": {current}}}"#),
                &format!("Store format version: {current} (current)\nNo migration needed."),
            );
            Ok(EXIT_SUCCESS)
        }
        Err(e) => {
            msg(
                json_output,
                &format!(r#"{{"status": "error", "message": "{e}"}}"#),
                &format!("Migration failed: {e}"),
            );
            Ok(EXIT_FAILURE)
        }
    }
}

fn msg(json_output: bool, json: &str, human: &str) {
    if json_output {
        println!("{json}");
    } else {
        println!("{human}");
    }
}
