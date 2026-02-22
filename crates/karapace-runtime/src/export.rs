use crate::RuntimeError;
use std::path::{Path, PathBuf};

pub struct ExportedApp {
    pub name: String,
    pub desktop_file: PathBuf,
    pub exec_command: String,
}

fn default_desktop_dir() -> Result<PathBuf, RuntimeError> {
    if let Ok(home) = std::env::var("HOME") {
        Ok(PathBuf::from(home).join(".local/share/applications"))
    } else {
        Err(RuntimeError::ExecFailed(
            "HOME environment variable not set".to_owned(),
        ))
    }
}

fn desktop_file_name(env_id: &str, app_name: &str) -> String {
    let short_id = &env_id[..12.min(env_id.len())];
    format!("karapace-{short_id}-{app_name}.desktop")
}

fn desktop_prefix(env_id: &str) -> String {
    let short_id = &env_id[..12.min(env_id.len())];
    format!("karapace-{short_id}-")
}

fn write_desktop_entry(
    desktop_dir: &Path,
    env_id: &str,
    app_name: &str,
    binary_path: &str,
    karapace_bin: &str,
    store_path: &str,
) -> Result<ExportedApp, RuntimeError> {
    let short_id = &env_id[..12.min(env_id.len())];

    std::fs::create_dir_all(desktop_dir)?;

    let desktop_id = desktop_file_name(env_id, app_name);
    let desktop_path = desktop_dir.join(&desktop_id);

    let exec_cmd = format!("{karapace_bin} --store {store_path} enter {short_id} -- {binary_path}");

    let icon = app_name;

    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={app_name} (Karapace {short_id})\n\
         Exec={exec_cmd}\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories=Karapace;\n\
         X-Karapace-EnvId={env_id}\n\
         X-Karapace-Store={store_path}\n\
         Comment=Launched inside Karapace environment {short_id}\n"
    );

    std::fs::write(&desktop_path, &contents)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&desktop_path, std::fs::Permissions::from_mode(0o755));
    }

    Ok(ExportedApp {
        name: app_name.to_owned(),
        desktop_file: desktop_path,
        exec_command: exec_cmd,
    })
}

pub fn export_app(
    env_id: &str,
    app_name: &str,
    binary_path: &str,
    karapace_bin: &str,
    store_path: &str,
) -> Result<ExportedApp, RuntimeError> {
    let desktop_dir = default_desktop_dir()?;
    write_desktop_entry(
        &desktop_dir,
        env_id,
        app_name,
        binary_path,
        karapace_bin,
        store_path,
    )
}

pub fn unexport_app(env_id: &str, app_name: &str) -> Result<(), RuntimeError> {
    let desktop_dir = default_desktop_dir()?;
    remove_desktop_entry(&desktop_dir, env_id, app_name)
}

fn remove_desktop_entry(
    desktop_dir: &Path,
    env_id: &str,
    app_name: &str,
) -> Result<(), RuntimeError> {
    let desktop_id = desktop_file_name(env_id, app_name);
    let desktop_path = desktop_dir.join(&desktop_id);
    if desktop_path.exists() {
        std::fs::remove_file(&desktop_path)?;
    }
    Ok(())
}

pub fn unexport_all(env_id: &str) -> Result<Vec<String>, RuntimeError> {
    let desktop_dir = default_desktop_dir()?;
    remove_all_entries(&desktop_dir, env_id)
}

fn remove_all_entries(desktop_dir: &Path, env_id: &str) -> Result<Vec<String>, RuntimeError> {
    let prefix = desktop_prefix(env_id);
    let mut removed = Vec::new();

    if desktop_dir.exists() {
        for entry in std::fs::read_dir(desktop_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) && name_str.ends_with(".desktop") {
                std::fs::remove_file(entry.path())?;
                removed.push(name_str.to_string());
            }
        }
    }

    Ok(removed)
}

pub fn list_exported(env_id: &str) -> Result<Vec<String>, RuntimeError> {
    let desktop_dir = default_desktop_dir()?;
    list_entries(&desktop_dir, env_id)
}

fn list_entries(desktop_dir: &Path, env_id: &str) -> Result<Vec<String>, RuntimeError> {
    let prefix = desktop_prefix(env_id);
    let mut apps = Vec::new();

    if desktop_dir.exists() {
        for entry in std::fs::read_dir(desktop_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(&prefix) && name_str.ends_with(".desktop") {
                let app_name = name_str
                    .strip_prefix(&prefix)
                    .and_then(|s| s.strip_suffix(".desktop"))
                    .unwrap_or(&name_str)
                    .to_string();
                apps.push(app_name);
            }
        }
    }

    Ok(apps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_desktop_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join(".local/share/applications");
        (dir, apps)
    }

    const TEST_ENV_ID: &str = "abc123def456789012345678901234567890123456789012345678901234";

    #[test]
    fn export_unexport_roundtrip() {
        let (_dir, apps) = test_desktop_dir();

        let result = write_desktop_entry(
            &apps,
            TEST_ENV_ID,
            "test-app",
            "/usr/bin/test-app",
            "/usr/bin/karapace",
            "/tmp/store",
        )
        .unwrap();

        assert!(result.desktop_file.exists());
        let contents = std::fs::read_to_string(&result.desktop_file).unwrap();
        assert!(contents.contains("X-Karapace-EnvId="));
        assert!(contents.contains("test-app"));

        let found = list_entries(&apps, TEST_ENV_ID).unwrap();
        assert_eq!(found, vec!["test-app"]);

        remove_desktop_entry(&apps, TEST_ENV_ID, "test-app").unwrap();
        assert!(!result.desktop_file.exists());
    }

    #[test]
    fn unexport_all_cleans_up() {
        let (_dir, apps) = test_desktop_dir();

        write_desktop_entry(
            &apps,
            TEST_ENV_ID,
            "app1",
            "/usr/bin/app1",
            "/usr/bin/karapace",
            "/tmp/store",
        )
        .unwrap();
        write_desktop_entry(
            &apps,
            TEST_ENV_ID,
            "app2",
            "/usr/bin/app2",
            "/usr/bin/karapace",
            "/tmp/store",
        )
        .unwrap();

        let removed = remove_all_entries(&apps, TEST_ENV_ID).unwrap();
        assert_eq!(removed.len(), 2);

        let found = list_entries(&apps, TEST_ENV_ID).unwrap();
        assert!(found.is_empty());
    }
}
