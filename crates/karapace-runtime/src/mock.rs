use crate::backend::{RuntimeBackend, RuntimeSpec, RuntimeStatus};
use crate::RuntimeError;
use karapace_schema::{ResolutionResult, ResolvedPackage};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct MockBackend {
    state: Mutex<HashMap<String, bool>>,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
        }
    }
}

impl MockBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl RuntimeBackend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn available(&self) -> bool {
        true
    }

    fn resolve(&self, spec: &RuntimeSpec) -> Result<ResolutionResult, RuntimeError> {
        // Mock resolution: deterministic digest from image name,
        // packages get version "0.0.0-mock" for deterministic identity.
        let base_image_digest =
            blake3::hash(format!("mock-image:{}", spec.manifest.base_image).as_bytes())
                .to_hex()
                .to_string();

        let resolved_packages = spec
            .manifest
            .system_packages
            .iter()
            .map(|name| ResolvedPackage {
                name: name.clone(),
                version: "0.0.0-mock".to_owned(),
            })
            .collect();

        Ok(ResolutionResult {
            base_image_digest,
            resolved_packages,
        })
    }

    fn build(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| RuntimeError::ExecFailed(format!("mutex poisoned: {e}")))?;
        state.insert(spec.env_id.clone(), false);

        let root = std::path::Path::new(&spec.root_path);
        if !root.exists() {
            std::fs::create_dir_all(root)?;
        }
        let overlay = std::path::Path::new(&spec.overlay_path);
        if !overlay.exists() {
            std::fs::create_dir_all(overlay)?;
        }

        // Create upper dir with mock filesystem content so engine tests
        // exercise the real layer capture path (pack_layer on upper dir).
        let upper = overlay.join("upper");
        std::fs::create_dir_all(&upper)?;
        std::fs::write(
            upper.join(".karapace-mock"),
            format!("mock-env:{}", spec.env_id),
        )?;
        for pkg in &spec.manifest.system_packages {
            std::fs::write(
                upper.join(format!(".pkg-{pkg}")),
                format!("{pkg}@0.0.0-mock"),
            )?;
        }

        Ok(())
    }

    fn enter(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| RuntimeError::ExecFailed(format!("mutex poisoned: {e}")))?;
        if state.get(&spec.env_id) == Some(&true) {
            return Err(RuntimeError::AlreadyRunning(spec.env_id.clone()));
        }
        state.insert(spec.env_id.clone(), true);
        Ok(())
    }

    fn exec(
        &self,
        _spec: &RuntimeSpec,
        command: &[String],
    ) -> Result<std::process::Output, RuntimeError> {
        let stdout = format!("mock-exec: {}\n", command.join(" "));
        // Create a real success ExitStatus portably
        let success_status = std::process::Command::new("true")
            .status()
            .unwrap_or_else(|_| {
                std::process::Command::new("/bin/true")
                    .status()
                    .expect("cannot execute /bin/true")
            });
        Ok(std::process::Output {
            status: success_status,
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
        })
    }

    fn destroy(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| RuntimeError::ExecFailed(format!("mutex poisoned: {e}")))?;
        state.remove(&spec.env_id);

        let overlay = std::path::Path::new(&spec.overlay_path);
        if overlay.exists() {
            std::fs::remove_dir_all(overlay)?;
        }
        Ok(())
    }

    fn status(&self, env_id: &str) -> Result<RuntimeStatus, RuntimeError> {
        let state = self
            .state
            .lock()
            .map_err(|e| RuntimeError::ExecFailed(format!("mutex poisoned: {e}")))?;
        let running = state.get(env_id).copied().unwrap_or(false);
        Ok(RuntimeStatus {
            env_id: env_id.to_owned(),
            running,
            pid: if running { Some(99999) } else { None },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use karapace_schema::parse_manifest_str;

    fn test_spec(dir: &std::path::Path) -> RuntimeSpec {
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        RuntimeSpec {
            env_id: "mock-test".to_owned(),
            root_path: dir.join("root").to_string_lossy().to_string(),
            overlay_path: dir.join("overlay").to_string_lossy().to_string(),
            store_root: dir.to_string_lossy().to_string(),
            manifest,
            offline: false,
        }
    }

    #[test]
    fn mock_resolve_determinism() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new();
        let spec = test_spec(dir.path());

        let r1 = backend.resolve(&spec).unwrap();
        let r2 = backend.resolve(&spec).unwrap();

        assert_eq!(r1.base_image_digest, r2.base_image_digest);
        assert_eq!(r1.resolved_packages.len(), r2.resolved_packages.len());
        for (a, b) in r1.resolved_packages.iter().zip(r2.resolved_packages.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.version, b.version);
        }
    }

    #[test]
    fn mock_resolve_with_packages() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "clang", "cmake"]
[runtime]
backend = "mock"
"#,
        )
        .unwrap()
        .normalize()
        .unwrap();

        let spec = RuntimeSpec {
            env_id: "mock-pkg-test".to_owned(),
            root_path: dir.path().join("root").to_string_lossy().to_string(),
            overlay_path: dir.path().join("overlay").to_string_lossy().to_string(),
            store_root: dir.path().to_string_lossy().to_string(),
            manifest,
            offline: false,
        };

        let backend = MockBackend::new();
        let result = backend.resolve(&spec).unwrap();

        assert_eq!(result.resolved_packages.len(), 3);
        assert!(result
            .resolved_packages
            .iter()
            .all(|p| p.version == "0.0.0-mock"));
        assert!(result.resolved_packages.iter().any(|p| p.name == "git"));
        assert!(!result.base_image_digest.is_empty());
    }

    #[test]
    fn mock_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let backend = MockBackend::new();
        let spec = test_spec(dir.path());

        backend.build(&spec).unwrap();
        let status = backend.status(&spec.env_id).unwrap();
        assert!(!status.running);

        backend.enter(&spec).unwrap();
        let status = backend.status(&spec.env_id).unwrap();
        assert!(status.running);
        assert_eq!(status.pid, Some(99999));

        assert!(backend.enter(&spec).is_err());

        backend.destroy(&spec).unwrap();
        let status = backend.status(&spec.env_id).unwrap();
        assert!(!status.running);
    }
}
