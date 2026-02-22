//! Manifest parsing, normalization, lock files, and environment identity for Karapace.
//!
//! This crate defines the schema layer: TOML manifest parsing (`ManifestV1`),
//! normalized representations (`NormalizedManifest`), deterministic environment
//! identity computation (`compute_env_id`), lock file generation/verification
//! (`LockFile`), and built-in preset definitions.

pub mod identity;
pub mod lock;
pub mod manifest;
pub mod normalize;
pub mod preset;
pub mod types;

pub use identity::{compute_env_id, EnvIdentity};
pub use lock::{LockError, LockFile, ResolutionResult, ResolvedPackage};
pub use manifest::{
    parse_manifest_file, parse_manifest_str, BaseSection, GuiSection, HardwareSection,
    ManifestError, ManifestV1, MountsSection, ResourceLimits, RuntimeSection, SystemSection,
};
pub use normalize::{NormalizedManifest, NormalizedMount};
pub use preset::{get_preset, list_presets, Preset, BUILTIN_PRESETS};
pub use types::{EnvId, LayerHash, ObjectHash, ShortId};
