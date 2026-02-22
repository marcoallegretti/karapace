use crate::CoreError;
use karapace_store::StoreLayout;
use serde::Serialize;
use std::fs;
use std::path::Path;

const WHITEOUT_PREFIX: &str = ".wh.";

/// Report of filesystem drift detected in an environment's overlay upper layer.
#[derive(Debug, Serialize)]
pub struct DriftReport {
    pub env_id: String,
    pub added: Vec<String>,
    pub modified: Vec<String>,
    pub removed: Vec<String>,
    pub has_drift: bool,
}

/// Scan the overlay upper directory for added, modified, and removed files.
pub fn diff_overlay(layout: &StoreLayout, env_id: &str) -> Result<DriftReport, CoreError> {
    let upper_dir = layout.upper_dir(env_id);
    let lower_dir = layout.env_path(env_id).join("lower");

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut removed = Vec::new();

    if upper_dir.exists() {
        collect_drift(
            &upper_dir,
            &upper_dir,
            &lower_dir,
            &mut added,
            &mut modified,
            &mut removed,
        )?;
    }

    added.sort();
    modified.sort();
    removed.sort();

    let has_drift = !added.is_empty() || !modified.is_empty() || !removed.is_empty();

    Ok(DriftReport {
        env_id: env_id.to_owned(),
        added,
        modified,
        removed,
        has_drift,
    })
}

fn collect_drift(
    upper_base: &Path,
    current: &Path,
    lower_base: &Path,
    added: &mut Vec<String>,
    modified: &mut Vec<String>,
    removed: &mut Vec<String>,
) -> Result<(), CoreError> {
    if !current.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();

        let rel = path
            .strip_prefix(upper_base)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();

        // Overlayfs whiteout files indicate deletion of the corresponding
        // file in the lower layer.
        if name_str.starts_with(WHITEOUT_PREFIX) {
            let deleted_name = name_str.strip_prefix(WHITEOUT_PREFIX).unwrap_or(&name_str);
            let deleted_rel = if let Some(parent) = path.parent() {
                let parent_rel = parent
                    .strip_prefix(upper_base)
                    .unwrap_or(parent)
                    .to_string_lossy();
                if parent_rel.is_empty() {
                    deleted_name.to_string()
                } else {
                    format!("{parent_rel}/{deleted_name}")
                }
            } else {
                deleted_name.to_string()
            };
            removed.push(deleted_rel);
            continue;
        }

        if path.is_dir() {
            collect_drift(upper_base, &path, lower_base, added, modified, removed)?;
        } else {
            // If the same relative path exists in the lower layer,
            // this is a modification; otherwise it's a new file.
            let lower_path = lower_base.join(&rel);
            if lower_path.exists() {
                modified.push(rel);
            } else {
                added.push(rel);
            }
        }
    }
    Ok(())
}

pub fn export_overlay(layout: &StoreLayout, env_id: &str, dest: &Path) -> Result<usize, CoreError> {
    let upper_dir = layout.upper_dir(env_id);
    if !upper_dir.exists() {
        return Ok(0);
    }

    fs::create_dir_all(dest)?;
    let mut count = 0;
    copy_recursive(&upper_dir, dest, &mut count)?;
    Ok(count)
}

fn copy_recursive(src: &Path, dst: &Path, count: &mut usize) -> Result<(), CoreError> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_recursive(&src_path, &dst_path, count)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
            *count += 1;
        }
    }
    Ok(())
}

pub fn commit_overlay(
    layout: &StoreLayout,
    env_id: &str,
    obj_store: &karapace_store::ObjectStore,
) -> Result<Vec<String>, CoreError> {
    let upper_dir = layout.upper_dir(env_id);
    if !upper_dir.exists() {
        return Ok(Vec::new());
    }

    let mut committed = Vec::new();
    commit_files(&upper_dir, obj_store, &mut committed)?;
    Ok(committed)
}

fn commit_files(
    current: &Path,
    obj_store: &karapace_store::ObjectStore,
    committed: &mut Vec<String>,
) -> Result<(), CoreError> {
    if !current.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            commit_files(&path, obj_store, committed)?;
        } else {
            let data = fs::read(&path)?;
            let hash = obj_store.put(&data)?;
            committed.push(hash);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, StoreLayout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        (dir, layout)
    }

    #[test]
    fn empty_overlay_reports_no_drift() {
        let (_dir, layout) = setup();
        let report = diff_overlay(&layout, "test-env").unwrap();
        assert!(!report.has_drift);
        assert!(report.added.is_empty());
    }

    #[test]
    fn files_in_overlay_detected_as_drift() {
        let (_dir, layout) = setup();
        let upper = layout.upper_dir("test-env");
        fs::create_dir_all(&upper).unwrap();
        fs::write(upper.join("new_file.txt"), "content").unwrap();

        let report = diff_overlay(&layout, "test-env").unwrap();
        assert!(report.has_drift);
        assert_eq!(report.added.len(), 1);
        assert!(report.added.contains(&"new_file.txt".to_owned()));
    }

    #[test]
    fn whiteout_files_detected_as_removed() {
        let (_dir, layout) = setup();
        let upper = layout.upper_dir("test-env");
        fs::create_dir_all(&upper).unwrap();
        // Overlayfs whiteout: .wh.deleted_file marks "deleted_file" as removed
        fs::write(upper.join(".wh.deleted_file"), "").unwrap();

        let report = diff_overlay(&layout, "test-env").unwrap();
        assert!(report.has_drift);
        assert_eq!(report.removed.len(), 1);
        assert!(report.removed.contains(&"deleted_file".to_owned()));
        assert!(report.added.is_empty());
    }

    #[test]
    fn modified_files_classified_correctly() {
        let (_dir, layout) = setup();
        // Create a "lower" layer with an existing file
        let env_dir = layout.env_path("test-env");
        let lower = env_dir.join("lower");
        fs::create_dir_all(&lower).unwrap();
        fs::write(lower.join("existing.txt"), "original").unwrap();

        // Same file in upper = modification
        let upper = layout.upper_dir("test-env");
        fs::create_dir_all(&upper).unwrap();
        fs::write(upper.join("existing.txt"), "changed").unwrap();
        // New file in upper only = added
        fs::write(upper.join("brand_new.txt"), "new").unwrap();

        let report = diff_overlay(&layout, "test-env").unwrap();
        assert!(report.has_drift);
        assert_eq!(report.modified.len(), 1);
        assert!(report.modified.contains(&"existing.txt".to_owned()));
        assert_eq!(report.added.len(), 1);
        assert!(report.added.contains(&"brand_new.txt".to_owned()));
    }

    #[test]
    fn export_copies_overlay_files() {
        let (_dir, layout) = setup();
        let upper = layout.upper_dir("test-env");
        fs::create_dir_all(&upper).unwrap();
        fs::write(upper.join("file.txt"), "data").unwrap();

        let export_dir = tempfile::tempdir().unwrap();
        let count = export_overlay(&layout, "test-env", export_dir.path()).unwrap();
        assert_eq!(count, 1);
        assert!(export_dir.path().join("file.txt").exists());
    }

    #[test]
    fn commit_stores_overlay_as_objects() {
        let (_dir, layout) = setup();
        let upper = layout.upper_dir("test-env");
        fs::create_dir_all(&upper).unwrap();
        fs::write(upper.join("file.txt"), "data").unwrap();

        let obj_store = karapace_store::ObjectStore::new(layout.clone());
        let committed = commit_overlay(&layout, "test-env", &obj_store).unwrap();
        assert_eq!(committed.len(), 1);
        assert!(obj_store.exists(&committed[0]));
    }
}
