use super::{json_pretty, EXIT_STORE_ERROR, EXIT_SUCCESS};
use karapace_core::Engine;
use karapace_store::verify_store_integrity;

pub fn run(engine: &Engine, json: bool) -> Result<u8, String> {
    let report = verify_store_integrity(engine.store_layout()).map_err(|e| e.to_string())?;

    if json {
        let payload = serde_json::json!({
            "checked": report.checked,
            "passed": report.passed,
            "failed": report.failed.len(),
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        println!(
            "store integrity: {}/{} objects passed",
            report.passed, report.checked
        );
        for f in &report.failed {
            println!("  FAIL {}: {}", f.hash, f.reason);
        }
    }

    if report.failed.is_empty() {
        Ok(EXIT_SUCCESS)
    } else {
        Ok(EXIT_STORE_ERROR)
    }
}
