//! Runtime backends and sandbox infrastructure for Karapace environments.
//!
//! This crate implements the execution layer: pluggable `RuntimeBackend` trait with
//! namespace (user-namespace + fuse-overlayfs) and OCI (runc) backends, sandbox
//! setup script generation, host integration (GPU, audio, X11/Wayland passthrough),
//! base image resolution, prerequisite checking, and security policy enforcement.

pub mod backend;
pub mod export;
pub mod host;
pub mod image;
pub mod mock;
pub mod namespace;
pub mod oci;
pub mod prereq;
pub mod sandbox;
pub mod security;
pub mod terminal;

pub use backend::{select_backend, RuntimeBackend, RuntimeSpec, RuntimeStatus};
pub use prereq::{check_namespace_prereqs, check_oci_prereqs, format_missing, MissingPrereq};
pub use security::SecurityPolicy;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("runtime I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend '{0}' is not available on this system")]
    BackendUnavailable(String),
    #[error("environment '{0}' is not running")]
    NotRunning(String),
    #[error("environment '{0}' is already running")]
    AlreadyRunning(String),
    #[error("security policy violation: {0}")]
    PolicyViolation(String),
    #[error("mount not allowed by policy: {0}")]
    MountDenied(String),
    #[error("device access not allowed: {0}")]
    DeviceDenied(String),
    #[error("runtime execution failed: {0}")]
    ExecFailed(String),
    #[error("image not found: {0}")]
    ImageNotFound(String),
}
