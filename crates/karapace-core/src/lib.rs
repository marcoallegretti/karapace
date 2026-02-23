//! Core orchestration engine for Karapace environment lifecycle.
//!
//! This crate ties together schema parsing, store operations, and runtime backends
//! into the `Engine` — the central API for building, entering, stopping, destroying,
//! and inspecting deterministic container environments. It also provides overlay
//! drift detection, concurrent store locking, and state-machine lifecycle validation.

pub mod concurrency;
pub mod drift;
pub mod engine;
pub mod lifecycle;

pub use concurrency::{install_signal_handler, shutdown_requested, StoreLock};
pub use drift::{commit_overlay, diff_overlay, export_overlay, DriftReport};
pub use engine::{BuildOptions, BuildResult, Engine};
pub use lifecycle::validate_transition;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("manifest error: {0}")]
    Manifest(#[from] karapace_schema::ManifestError),
    #[error("lock error: {0}")]
    Lock(#[from] karapace_schema::LockError),
    #[error("store error: {0}")]
    Store(#[from] karapace_store::StoreError),
    #[error("runtime error: {0}")]
    Runtime(#[from] karapace_runtime::RuntimeError),
    #[error("invalid state transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },
    #[error("environment not found: {0}")]
    EnvNotFound(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("remote error: {0}")]
    Remote(#[from] karapace_remote::RemoteError),
}
