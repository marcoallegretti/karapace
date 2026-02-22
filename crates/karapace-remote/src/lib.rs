//! Remote store synchronization for sharing Karapace environments.
//!
//! This crate provides push/pull transfer of content-addressable objects and layer
//! manifests to/from a remote HTTP backend, a registry for named environment
//! references, and configuration for remote endpoints with optional authentication.

pub mod config;
pub mod http;
pub mod registry;
pub mod transfer;

pub use config::RemoteConfig;
pub use registry::{parse_ref, Registry, RegistryEntry};
pub use transfer::{pull_env, push_env, resolve_ref, PullResult, PushResult};

/// Protocol version sent as `X-Karapace-Protocol` header on all HTTP requests.
/// Servers can reject clients with incompatible protocol versions.
pub const PROTOCOL_VERSION: u32 = 1;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error("remote I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("store error: {0}")]
    Store(#[from] karapace_store::StoreError),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("remote config error: {0}")]
    Config(String),
    #[error("integrity failure for '{key}': expected {expected}, got {actual}")]
    IntegrityFailure {
        key: String,
        expected: String,
        actual: String,
    },
}

/// A content-addressable blob in the remote store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobKind {
    Object,
    Layer,
    Metadata,
}

/// Trait for remote storage backends.
pub trait RemoteBackend: Send + Sync {
    /// Upload a blob to the remote store. Returns the key used.
    fn put_blob(&self, kind: BlobKind, key: &str, data: &[u8]) -> Result<(), RemoteError>;

    /// Download a blob from the remote store.
    fn get_blob(&self, kind: BlobKind, key: &str) -> Result<Vec<u8>, RemoteError>;

    /// Check if a blob exists in the remote store.
    fn has_blob(&self, kind: BlobKind, key: &str) -> Result<bool, RemoteError>;

    /// List all blobs of a given kind.
    fn list_blobs(&self, kind: BlobKind) -> Result<Vec<String>, RemoteError>;

    /// Upload the registry index.
    fn put_registry(&self, data: &[u8]) -> Result<(), RemoteError>;

    /// Download the registry index.
    fn get_registry(&self) -> Result<Vec<u8>, RemoteError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_kind_debug() {
        assert_eq!(format!("{:?}", BlobKind::Object), "Object");
        assert_eq!(format!("{:?}", BlobKind::Layer), "Layer");
        assert_eq!(format!("{:?}", BlobKind::Metadata), "Metadata");
    }
}
