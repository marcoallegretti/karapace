use crate::RuntimeError;
use karapace_schema::{NormalizedManifest, ResolutionResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSpec {
    pub env_id: String,
    pub root_path: String,
    pub overlay_path: String,
    pub store_root: String,
    pub manifest: NormalizedManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub env_id: String,
    pub running: bool,
    pub pid: Option<u32>,
}

pub trait RuntimeBackend: Send + Sync {
    fn name(&self) -> &str;

    fn available(&self) -> bool;

    /// Resolve dependencies: download/identify the base image and query the
    /// package manager for exact versions of each requested package.
    /// Returns a ResolutionResult with content digest and pinned versions.
    fn resolve(&self, spec: &RuntimeSpec) -> Result<ResolutionResult, RuntimeError>;

    fn build(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError>;

    fn enter(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError>;

    fn exec(
        &self,
        _spec: &RuntimeSpec,
        _command: &[String],
    ) -> Result<std::process::Output, RuntimeError> {
        Err(RuntimeError::ExecFailed(format!(
            "exec not supported by {} backend",
            self.name()
        )))
    }

    fn destroy(&self, spec: &RuntimeSpec) -> Result<(), RuntimeError>;

    fn status(&self, env_id: &str) -> Result<RuntimeStatus, RuntimeError>;
}

pub fn select_backend(
    name: &str,
    store_root: &str,
) -> Result<Box<dyn RuntimeBackend>, RuntimeError> {
    match name {
        "namespace" => Ok(Box::new(
            crate::namespace::NamespaceBackend::with_store_root(store_root),
        )),
        "oci" => Ok(Box::new(crate::oci::OciBackend::with_store_root(
            store_root,
        ))),
        "mock" => Ok(Box::new(crate::mock::MockBackend::new())),
        other => Err(RuntimeError::BackendUnavailable(other.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_valid_backends() {
        assert!(select_backend("namespace", "/tmp/test-store").is_ok());
        assert!(select_backend("oci", "/tmp/test-store").is_ok());
        assert!(select_backend("mock", "/tmp/test-store").is_ok());
    }

    #[test]
    fn select_invalid_backend_fails() {
        assert!(select_backend("nonexistent", "/tmp/test-store").is_err());
    }
}
