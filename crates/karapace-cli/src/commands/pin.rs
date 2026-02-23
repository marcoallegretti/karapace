use super::{json_pretty, EXIT_SUCCESS};
use karapace_runtime::image::resolve_pinned_image_url;
use karapace_schema::manifest::{parse_manifest_file, ManifestV1};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

fn write_atomic(dest: &Path, content: &str) -> Result<(), String> {
    let dir = dest
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let mut tmp = NamedTempFile::new_in(&dir).map_err(|e| format!("write temp file: {e}"))?;
    use std::io::Write;
    tmp.write_all(content.as_bytes())
        .map_err(|e| format!("write temp file: {e}"))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| format!("fsync temp file: {e}"))?;
    tmp.persist(dest)
        .map_err(|e| format!("persist manifest: {}", e.error))?;
    Ok(())
}

fn is_pinned(image: &str) -> bool {
    let s = image.trim();
    s.starts_with("http://") || s.starts_with("https://")
}

pub fn run(
    manifest_path: &Path,
    check: bool,
    write_lock: bool,
    json: bool,
    store_path: Option<&Path>,
) -> Result<u8, String> {
    let manifest =
        parse_manifest_file(manifest_path).map_err(|e| format!("failed to parse manifest: {e}"))?;

    if check {
        if is_pinned(&manifest.base.image) {
            if json {
                let payload = serde_json::json!({
                    "status": "pinned",
                    "manifest": manifest_path,
                });
                println!("{}", json_pretty(&payload)?);
            }
            return Ok(EXIT_SUCCESS);
        }
        return Err(format!(
            "base.image is not pinned: '{}' (run 'karapace pin')",
            manifest.base.image
        ));
    }

    let pinned = resolve_pinned_image_url(&manifest.base.image)
        .map_err(|e| format!("failed to resolve pinned image URL: {e}"))?;

    let mut updated: ManifestV1 = manifest;
    updated.base.image = pinned;

    let toml =
        toml::to_string_pretty(&updated).map_err(|e| format!("TOML serialization failed: {e}"))?;
    write_atomic(manifest_path, &toml)?;

    if write_lock {
        let store = store_path.ok_or_else(|| "internal error: missing store path".to_owned())?;
        let engine = karapace_core::Engine::new(store);
        engine.build(manifest_path).map_err(|e| e.to_string())?;
    }

    if json {
        let payload = serde_json::json!({
            "status": "pinned",
            "manifest": manifest_path,
            "base_image": updated.base.image,
        });
        println!("{}", json_pretty(&payload)?);
    } else {
        println!("pinned base image in {}", manifest_path.display());
    }

    Ok(EXIT_SUCCESS)
}
