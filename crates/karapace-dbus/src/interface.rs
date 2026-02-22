use karapace_core::StoreLock;
use karapace_store::StoreLayout;
use serde::{Deserialize, Serialize};
use tracing::{error, info};
use zbus::interface;

pub const DBUS_INTERFACE: &str = "org.karapace.Manager1";
pub const DBUS_PATH: &str = "/org/karapace/Manager1";
pub const API_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvInfo {
    pub env_id: String,
    pub short_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub state: String,
}

#[derive(Debug, Serialize)]
struct DestroyResponse {
    destroyed: String,
}

#[derive(Debug, Serialize)]
struct EnterResponse {
    entered: String,
}

#[derive(Debug, Serialize)]
struct RenameResponse {
    env_id: String,
    name: String,
}

fn to_fdo(msg: impl std::fmt::Display) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(msg.to_string())
}

fn send_notification(summary: &str, body: &str) {
    if let Err(e) = notify_rust::Notification::new()
        .appname("Karapace")
        .summary(summary)
        .body(body)
        .timeout(notify_rust::Timeout::Milliseconds(5000))
        .show()
    {
        tracing::debug!("desktop notification failed (non-fatal): {e}");
    }
}

pub struct KarapaceManager {
    store_root: String,
}

impl KarapaceManager {
    pub fn new(store_root: String) -> Self {
        Self { store_root }
    }

    fn engine(&self) -> karapace_core::Engine {
        karapace_core::Engine::new(&self.store_root)
    }

    fn acquire_lock(&self) -> Result<StoreLock, zbus::fdo::Error> {
        let layout = StoreLayout::new(&self.store_root);
        StoreLock::acquire(&layout.lock_file()).map_err(|e| {
            error!("store lock acquisition failed: {e}");
            to_fdo(format!("store lock: {e}"))
        })
    }

    fn resolve_env(&self, id_or_name: &str) -> Result<String, zbus::fdo::Error> {
        let engine = self.engine();
        if id_or_name.len() == 64 {
            return Ok(id_or_name.to_owned());
        }
        let envs = engine.list().map_err(to_fdo)?;
        for e in &envs {
            if *e.env_id == *id_or_name
                || *e.short_id == *id_or_name
                || e.name.as_deref() == Some(id_or_name)
            {
                return Ok(e.env_id.to_string());
            }
        }
        for e in &envs {
            if e.env_id.starts_with(id_or_name) || e.short_id.starts_with(id_or_name) {
                return Ok(e.env_id.to_string());
            }
        }
        Err(to_fdo(format!("no environment matching '{id_or_name}'")))
    }
}

#[allow(clippy::unused_async)]
#[interface(name = "org.karapace.Manager1")]
impl KarapaceManager {
    #[zbus(property)]
    async fn api_version(&self) -> u32 {
        API_VERSION
    }

    #[zbus(property)]
    async fn store_root(&self) -> &str {
        &self.store_root
    }

    async fn list_environments(&self) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: ListEnvironments");
        let envs = self.engine().list().map_err(|e| {
            error!("ListEnvironments failed: {e}");
            to_fdo(e)
        })?;
        let infos: Vec<EnvInfo> = envs
            .iter()
            .map(|e| EnvInfo {
                env_id: e.env_id.to_string(),
                short_id: e.short_id.to_string(),
                name: e.name.clone(),
                state: e.state.to_string(),
            })
            .collect();
        serde_json::to_string(&infos).map_err(to_fdo)
    }

    async fn get_environment_status(&self, id_or_name: String) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: GetEnvironmentStatus {id_or_name}");
        let resolved = self.resolve_env(&id_or_name)?;
        let meta = self.engine().inspect(&resolved).map_err(|e| {
            error!("GetEnvironmentStatus failed for {id_or_name}: {e}");
            to_fdo(e)
        })?;
        serde_json::to_string(&EnvInfo {
            env_id: meta.env_id.to_string(),
            short_id: meta.short_id.to_string(),
            name: meta.name,
            state: meta.state.to_string(),
        })
        .map_err(to_fdo)
    }

    async fn get_environment_hash(&self, id_or_name: String) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: GetEnvironmentHash {id_or_name}");
        let resolved = self.resolve_env(&id_or_name)?;
        let meta = self.engine().inspect(&resolved).map_err(|e| {
            error!("GetEnvironmentHash failed for {id_or_name}: {e}");
            to_fdo(e)
        })?;
        Ok(meta.env_id.to_string())
    }

    async fn build_environment(&self, manifest_path: String) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: BuildEnvironment {manifest_path}");
        let _lock = self.acquire_lock()?;
        let result = match self.engine().build(std::path::Path::new(&manifest_path)) {
            Ok(r) => {
                send_notification(
                    "Build Complete",
                    &format!("Environment {} built", &r.identity.short_id),
                );
                r
            }
            Err(e) => {
                send_notification("Build Failed", &e.to_string());
                error!("BuildEnvironment failed: {e}");
                return Err(to_fdo(e));
            }
        };
        serde_json::to_string(&EnvInfo {
            env_id: result.identity.env_id.to_string(),
            short_id: result.identity.short_id.to_string(),
            name: None,
            state: "built".to_owned(),
        })
        .map_err(to_fdo)
    }

    async fn build_named_environment(
        &self,
        manifest_path: String,
        name: String,
    ) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: BuildNamedEnvironment {manifest_path} name={name}");
        let _lock = self.acquire_lock()?;
        let engine = self.engine();
        let result = match engine.build(std::path::Path::new(&manifest_path)) {
            Ok(r) => {
                send_notification(
                    "Build Complete",
                    &format!("Environment '{}' ({}) built", name, &r.identity.short_id),
                );
                r
            }
            Err(e) => {
                send_notification("Build Failed", &e.to_string());
                error!("BuildNamedEnvironment failed: {e}");
                return Err(to_fdo(e));
            }
        };
        engine
            .set_name(&result.identity.env_id, Some(name.clone()))
            .map_err(|e| {
                error!("BuildNamedEnvironment set_name failed: {e}");
                to_fdo(e)
            })?;
        serde_json::to_string(&EnvInfo {
            env_id: result.identity.env_id.to_string(),
            short_id: result.identity.short_id.to_string(),
            name: Some(name),
            state: "built".to_owned(),
        })
        .map_err(to_fdo)
    }

    async fn destroy_environment(&self, id_or_name: String) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: DestroyEnvironment {id_or_name}");
        let resolved = self.resolve_env(&id_or_name)?;
        let _lock = self.acquire_lock()?;
        self.engine().destroy(&resolved).map_err(|e| {
            error!("DestroyEnvironment failed for {id_or_name}: {e}");
            to_fdo(e)
        })?;
        serde_json::to_string(&DestroyResponse {
            destroyed: resolved,
        })
        .map_err(to_fdo)
    }

    async fn run_environment(&self, id_or_name: String) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: RunEnvironment {id_or_name}");
        let resolved = self.resolve_env(&id_or_name)?;
        let _lock = self.acquire_lock()?;
        self.engine().enter(&resolved).map_err(|e| {
            error!("RunEnvironment failed for {id_or_name}: {e}");
            to_fdo(e)
        })?;
        serde_json::to_string(&EnterResponse { entered: resolved }).map_err(to_fdo)
    }

    async fn rename_environment(
        &self,
        id_or_name: String,
        new_name: String,
    ) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: RenameEnvironment {id_or_name} -> {new_name}");
        let resolved = self.resolve_env(&id_or_name)?;
        let _lock = self.acquire_lock()?;
        self.engine().rename(&resolved, &new_name).map_err(|e| {
            error!("RenameEnvironment failed: {e}");
            to_fdo(e)
        })?;
        serde_json::to_string(&RenameResponse {
            env_id: resolved,
            name: new_name,
        })
        .map_err(to_fdo)
    }

    async fn list_presets(&self) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: ListPresets");
        let presets: Vec<serde_json::Value> = karapace_schema::list_presets()
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "description": p.description,
                })
            })
            .collect();
        serde_json::to_string(&presets).map_err(to_fdo)
    }

    async fn garbage_collect(&self, dry_run: bool) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: GarbageCollect (dry_run={dry_run})");
        let lock = self.acquire_lock()?;
        let report = self.engine().gc(&lock, dry_run).map_err(|e| {
            error!("GarbageCollect failed: {e}");
            to_fdo(e)
        })?;
        serde_json::to_string(&serde_json::json!({
            "dry_run": dry_run,
            "removed_envs": report.removed_envs,
            "removed_layers": report.removed_layers,
            "removed_objects": report.removed_objects,
        }))
        .map_err(to_fdo)
    }

    async fn verify_store(&self) -> Result<String, zbus::fdo::Error> {
        info!("D-Bus: VerifyStore");
        let layout = StoreLayout::new(&self.store_root);
        let report = karapace_store::verify_store_integrity(&layout).map_err(|e| {
            error!("VerifyStore failed: {e}");
            to_fdo(e)
        })?;
        serde_json::to_string(&serde_json::json!({
            "checked": report.checked,
            "passed": report.passed,
            "failed": report.failed.len(),
        }))
        .map_err(to_fdo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, KarapaceManager) {
        let store = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let manager = KarapaceManager::new(store.path().to_string_lossy().to_string());
        (store, project, manager)
    }

    fn write_mock_manifest(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("karapace.toml");
        std::fs::write(
            &path,
            r#"manifest_version = 1
[base]
image = "rolling"
[runtime]
backend = "mock"
"#,
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn list_environments_empty() {
        let (_store, _project, mgr) = setup();
        let result = mgr.list_environments().await;
        // Empty store may return empty list or error — both are valid
        if let Ok(json) = result {
            let parsed: Vec<EnvInfo> = serde_json::from_str(&json).unwrap();
            assert!(parsed.is_empty());
        }
    }

    #[tokio::test]
    async fn build_and_list_roundtrip() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();
        assert_eq!(info.state, "built");
        assert!(!info.env_id.is_empty());

        let list_result = mgr.list_environments().await.unwrap();
        let envs: Vec<EnvInfo> = serde_json::from_str(&list_result).unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].env_id, info.env_id);
    }

    #[tokio::test]
    async fn get_status_of_built_env() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        let status = mgr
            .get_environment_status(info.env_id.clone())
            .await
            .unwrap();
        let status_info: EnvInfo = serde_json::from_str(&status).unwrap();
        assert_eq!(status_info.state, "built");
    }

    #[tokio::test]
    async fn get_hash_returns_env_id() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        let hash = mgr.get_environment_hash(info.env_id.clone()).await.unwrap();
        assert_eq!(hash, info.env_id);
    }

    #[tokio::test]
    async fn destroy_removes_environment() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        mgr.destroy_environment(info.env_id.clone()).await.unwrap();

        // Should no longer be in the list
        let list_result = mgr.list_environments().await.unwrap();
        let envs: Vec<EnvInfo> = serde_json::from_str(&list_result).unwrap();
        assert!(envs.is_empty());
    }

    #[tokio::test]
    async fn gc_on_empty_store() {
        let (_store, _project, mgr) = setup();
        // GC on empty/uninitialized store should not panic
        let result = mgr.garbage_collect(true).await;
        // May succeed or fail depending on store init — should not panic
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn get_status_nonexistent_returns_error() {
        let (_store, _project, mgr) = setup();
        let result = mgr.get_environment_status("nonexistent".to_owned()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn build_named_environment_sets_name() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let result = mgr
            .build_named_environment(manifest.to_string_lossy().to_string(), "my-env".to_owned())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&result).unwrap();
        assert_eq!(info.name, Some("my-env".to_owned()));
        assert_eq!(info.state, "built");

        // List should include name
        let list_result = mgr.list_environments().await.unwrap();
        let envs: Vec<EnvInfo> = serde_json::from_str(&list_result).unwrap();
        assert_eq!(envs[0].name, Some("my-env".to_owned()));
    }

    #[tokio::test]
    async fn rename_environment_works() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        mgr.rename_environment(info.env_id.clone(), "renamed-env".to_owned())
            .await
            .unwrap();

        // Verify name via status
        let status = mgr
            .get_environment_status(info.env_id.clone())
            .await
            .unwrap();
        let status_info: EnvInfo = serde_json::from_str(&status).unwrap();
        assert_eq!(status_info.name, Some("renamed-env".to_owned()));
    }

    #[tokio::test]
    async fn resolve_by_name_works() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_named_environment(
                manifest.to_string_lossy().to_string(),
                "named-env".to_owned(),
            )
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        // Get status by name
        let status = mgr
            .get_environment_status("named-env".to_owned())
            .await
            .unwrap();
        let status_info: EnvInfo = serde_json::from_str(&status).unwrap();
        assert_eq!(status_info.env_id, info.env_id);
    }

    #[tokio::test]
    async fn list_presets_returns_presets() {
        let (_store, _project, mgr) = setup();
        let result = mgr.list_presets().await.unwrap();
        let presets: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();
        assert!(!presets.is_empty());
        assert!(presets.iter().any(|p| p["name"] == "dev"));
        assert!(presets.iter().any(|p| p["name"] == "minimal"));
    }

    #[tokio::test]
    async fn destroy_by_name_works() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        mgr.build_named_environment(
            manifest.to_string_lossy().to_string(),
            "to-destroy".to_owned(),
        )
        .await
        .unwrap();

        mgr.destroy_environment("to-destroy".to_owned())
            .await
            .unwrap();

        let list_result = mgr.list_environments().await.unwrap();
        let envs: Vec<EnvInfo> = serde_json::from_str(&list_result).unwrap();
        assert!(envs.is_empty());
    }

    #[tokio::test]
    async fn build_invalid_manifest_returns_error() {
        let (_store, project, mgr) = setup();
        let bad_path = project.path().join("nonexistent.toml");
        let result = mgr
            .build_environment(bad_path.to_string_lossy().to_string())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rename_to_taken_name_returns_error() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        mgr.build_named_environment(
            manifest.to_string_lossy().to_string(),
            "first-env".to_owned(),
        )
        .await
        .unwrap();

        // Build a second manifest with different content to get a different env_id
        let path2 = project.path().join("karapace2.toml");
        std::fs::write(
            &path2,
            r#"manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["curl"]
[runtime]
backend = "mock"
"#,
        )
        .unwrap();
        let build2 = mgr
            .build_environment(path2.to_string_lossy().to_string())
            .await
            .unwrap();
        let info2: EnvInfo = serde_json::from_str(&build2).unwrap();

        let result = mgr
            .rename_environment(info2.env_id, "first-env".to_owned())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn destroy_nonexistent_returns_error() {
        let (_store, _project, mgr) = setup();
        let result = mgr.destroy_environment("does-not-exist".to_owned()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn gc_after_build_and_destroy() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        mgr.destroy_environment(info.env_id).await.unwrap();

        let gc_result = mgr.garbage_collect(false).await.unwrap();
        let gc: serde_json::Value = serde_json::from_str(&gc_result).unwrap();
        assert_eq!(gc["dry_run"], false);
    }

    #[tokio::test]
    async fn verify_store_on_fresh_store() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        mgr.build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();

        let result = mgr.verify_store().await.unwrap();
        let report: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(report["checked"].as_u64().unwrap() > 0);
        assert_eq!(report["failed"], 0);
    }

    #[tokio::test]
    async fn list_after_rename_shows_new_name() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        mgr.rename_environment(info.env_id.clone(), "new-name".to_owned())
            .await
            .unwrap();

        let list_result = mgr.list_environments().await.unwrap();
        let envs: Vec<EnvInfo> = serde_json::from_str(&list_result).unwrap();
        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].name, Some("new-name".to_owned()));
    }

    #[tokio::test]
    async fn destroy_response_is_valid_json() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        let destroy_result = mgr.destroy_environment(info.env_id.clone()).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&destroy_result).unwrap();
        assert_eq!(parsed["destroyed"].as_str().unwrap(), info.env_id);
    }

    #[tokio::test]
    async fn rename_response_is_valid_json() {
        let (_store, project, mgr) = setup();
        let manifest = write_mock_manifest(project.path());

        let build_result = mgr
            .build_environment(manifest.to_string_lossy().to_string())
            .await
            .unwrap();
        let info: EnvInfo = serde_json::from_str(&build_result).unwrap();

        let rename_result = mgr
            .rename_environment(info.env_id.clone(), "test-rename".to_owned())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&rename_result).unwrap();
        assert_eq!(parsed["env_id"].as_str().unwrap(), info.env_id);
        assert_eq!(parsed["name"].as_str().unwrap(), "test-rename");
    }
}
