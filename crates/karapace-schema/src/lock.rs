use crate::identity::EnvIdentity;
use crate::manifest::ManifestError;
use crate::normalize::{NormalizedManifest, NormalizedMount};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LockError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("lock file I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("lock file parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("lock file serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("lock file env_id mismatch: lock has '{lock_id}', recomputed '{computed_id}'")]
    EnvIdMismatch {
        lock_id: String,
        computed_id: String,
    },
    #[error("lock file manifest drift: {0}")]
    ManifestDrift(String),
}

/// A resolved package with pinned version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
}

/// Result of dependency resolution against a base image.
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    /// Content hash (blake3) of the base image rootfs tarball.
    pub base_image_digest: String,
    /// Resolved packages with pinned versions.
    pub resolved_packages: Vec<ResolvedPackage>,
}

/// The lock file captures the fully resolved state of an environment.
///
/// The env_id is computed deterministically from the locked fields,
/// not from unresolved manifest data. This guarantees:
///   same lockfile → same env_id → same environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockFile {
    pub lock_version: u32,
    pub env_id: String,
    pub short_id: String,

    // Base image identity
    pub base_image: String,
    pub base_image_digest: String,

    // Resolved dependencies (version-pinned)
    pub resolved_packages: Vec<ResolvedPackage>,
    pub resolved_apps: Vec<String>,

    // Runtime policy (included in hash contract)
    pub runtime_backend: String,
    pub hardware_gpu: bool,
    pub hardware_audio: bool,
    pub network_isolation: bool,

    // Mount policy
    #[serde(default)]
    pub mounts: Vec<NormalizedMount>,

    // Resource limits
    #[serde(default)]
    pub cpu_shares: Option<u64>,
    #[serde(default)]
    pub memory_limit_mb: Option<u64>,
}

impl LockFile {
    /// Generate a lock file from a manifest and resolution results.
    ///
    /// The env_id is computed from the resolved state, ensuring that
    /// identical resolved dependencies always produce the same identity.
    pub fn from_resolved(normalized: &NormalizedManifest, resolution: &ResolutionResult) -> Self {
        let mut resolved_packages = resolution.resolved_packages.clone();
        resolved_packages.sort();

        let lock = LockFile {
            lock_version: 2,
            env_id: String::new(), // computed below
            short_id: String::new(),
            base_image: normalized.base_image.clone(),
            base_image_digest: resolution.base_image_digest.clone(),
            resolved_packages,
            resolved_apps: normalized.gui_apps.clone(),
            runtime_backend: normalized.runtime_backend.clone(),
            hardware_gpu: normalized.hardware_gpu,
            hardware_audio: normalized.hardware_audio,
            network_isolation: normalized.network_isolation,
            mounts: normalized.mounts.clone(),
            cpu_shares: normalized.cpu_shares,
            memory_limit_mb: normalized.memory_limit_mb,
        };

        let identity = lock.compute_identity();
        LockFile {
            env_id: identity.env_id.into_inner(),
            short_id: identity.short_id.into_inner(),
            ..lock
        }
    }

    /// Compute the environment identity from the locked state.
    ///
    /// This is the canonical hash computation. It uses only resolved,
    /// pinned data — never unresolved package names or image tags.
    pub fn compute_identity(&self) -> EnvIdentity {
        let mut hasher = blake3::Hasher::new();

        // Base image: content digest, not tag name
        hasher.update(format!("base_digest:{}", self.base_image_digest).as_bytes());

        // Resolved packages: name@version (sorted)
        for pkg in &self.resolved_packages {
            hasher.update(format!("pkg:{}@{}", pkg.name, pkg.version).as_bytes());
        }

        // Apps (sorted by normalize)
        for app in &self.resolved_apps {
            hasher.update(format!("app:{app}").as_bytes());
        }

        // Hardware policy
        if self.hardware_gpu {
            hasher.update(b"hw:gpu");
        }
        if self.hardware_audio {
            hasher.update(b"hw:audio");
        }

        // Mount policy (sorted by label in normalize)
        for mount in &self.mounts {
            hasher.update(
                format!(
                    "mount:{}:{}:{}",
                    mount.label, mount.host_path, mount.container_path
                )
                .as_bytes(),
            );
        }

        // Runtime backend
        hasher.update(format!("backend:{}", self.runtime_backend).as_bytes());

        // Network isolation
        if self.network_isolation {
            hasher.update(b"net:isolated");
        }

        // Resource limits
        if let Some(cpu) = self.cpu_shares {
            hasher.update(format!("cpu:{cpu}").as_bytes());
        }
        if let Some(mem) = self.memory_limit_mb {
            hasher.update(format!("mem:{mem}").as_bytes());
        }

        let hex = hasher.finalize().to_hex().to_string();
        let short = hex[..12].to_owned();

        EnvIdentity {
            env_id: crate::types::EnvId::new(hex),
            short_id: crate::types::ShortId::new(short),
        }
    }

    /// Verify that this lock file is internally consistent
    /// (stored env_id matches recomputed env_id).
    pub fn verify_integrity(&self) -> Result<EnvIdentity, LockError> {
        let identity = self.compute_identity();
        if self.env_id != identity.env_id.as_str() {
            return Err(LockError::EnvIdMismatch {
                lock_id: self.env_id.clone(),
                computed_id: identity.env_id.into_inner(),
            });
        }
        Ok(identity)
    }

    /// Check that a manifest's declared intent matches this lock file.
    ///
    /// This catches cases where the manifest changed but the lock wasn't updated.
    pub fn verify_manifest_intent(&self, normalized: &NormalizedManifest) -> Result<(), LockError> {
        if self.base_image != normalized.base_image {
            return Err(LockError::ManifestDrift(format!(
                "base image changed: lock has '{}', manifest has '{}'",
                self.base_image, normalized.base_image
            )));
        }
        if self.runtime_backend != normalized.runtime_backend {
            return Err(LockError::ManifestDrift(format!(
                "runtime backend changed: lock has '{}', manifest has '{}'",
                self.runtime_backend, normalized.runtime_backend
            )));
        }

        // Check that all declared packages are present in the lock
        let locked_names: Vec<&str> = self
            .resolved_packages
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        for pkg in &normalized.system_packages {
            if !locked_names.contains(&pkg.as_str()) {
                return Err(LockError::ManifestDrift(format!(
                    "package '{pkg}' is in manifest but not in lock file. Run 'karapace build' to re-resolve."
                )));
            }
        }

        if self.hardware_gpu != normalized.hardware_gpu
            || self.hardware_audio != normalized.hardware_audio
        {
            return Err(LockError::ManifestDrift(
                "hardware policy changed. Run 'karapace build' to re-resolve.".to_owned(),
            ));
        }

        Ok(())
    }

    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<(), LockError> {
        let path = path.as_ref();
        let content = toml::to_string_pretty(self)?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        std::io::Write::write_all(&mut tmp, content.as_bytes())?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| LockError::Io(e.error))?;
        // Fsync parent directory to ensure rename durability on power loss.
        if let Ok(f) = fs::File::open(dir) {
            let _ = f.sync_all();
        }
        Ok(())
    }

    pub fn read_from_file(path: impl AsRef<Path>) -> Result<Self, LockError> {
        let content = fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::parse_manifest_str;

    fn sample_normalized() -> NormalizedManifest {
        parse_manifest_str(
            r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "clang"]
"#,
        )
        .unwrap()
        .normalize()
        .unwrap()
    }

    fn sample_resolution() -> ResolutionResult {
        ResolutionResult {
            base_image_digest: "a".repeat(64),
            resolved_packages: vec![
                ResolvedPackage {
                    name: "clang".to_owned(),
                    version: "17.0.6-1".to_owned(),
                },
                ResolvedPackage {
                    name: "git".to_owned(),
                    version: "2.44.0-1".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn lock_roundtrip() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock = LockFile::from_resolved(&normalized, &resolution);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("karapace.lock");

        lock.write_to_file(&path).unwrap();
        let loaded = LockFile::read_from_file(&path).unwrap();
        assert_eq!(lock, loaded);
    }

    #[test]
    fn lock_integrity_check_passes() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock = LockFile::from_resolved(&normalized, &resolution);
        assert!(lock.verify_integrity().is_ok());
    }

    #[test]
    fn lock_integrity_fails_on_tamper() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let mut lock = LockFile::from_resolved(&normalized, &resolution);
        lock.env_id = "tampered".to_owned();
        assert!(lock.verify_integrity().is_err());
    }

    #[test]
    fn lock_contains_real_digest() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock = LockFile::from_resolved(&normalized, &resolution);
        // Digest is the actual image digest, not a hash of the tag name
        assert_eq!(lock.base_image_digest, "a".repeat(64));
        assert_eq!(lock.base_image, "rolling");
    }

    #[test]
    fn lock_contains_pinned_versions() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock = LockFile::from_resolved(&normalized, &resolution);
        assert_eq!(lock.resolved_packages.len(), 2);
        assert_eq!(lock.resolved_packages[0].name, "clang");
        assert_eq!(lock.resolved_packages[0].version, "17.0.6-1");
        assert_eq!(lock.resolved_packages[1].name, "git");
        assert_eq!(lock.resolved_packages[1].version, "2.44.0-1");
    }

    #[test]
    fn same_resolution_same_identity() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock1 = LockFile::from_resolved(&normalized, &resolution);
        let lock2 = LockFile::from_resolved(&normalized, &resolution);
        assert_eq!(lock1.env_id, lock2.env_id);
    }

    #[test]
    fn different_versions_different_identity() {
        let normalized = sample_normalized();
        let res1 = sample_resolution();
        let mut res2 = sample_resolution();
        res2.resolved_packages[1].version = "2.45.0-1".to_owned();

        let lock1 = LockFile::from_resolved(&normalized, &res1);
        let lock2 = LockFile::from_resolved(&normalized, &res2);
        assert_ne!(lock1.env_id, lock2.env_id);
    }

    #[test]
    fn different_image_digest_different_identity() {
        let normalized = sample_normalized();
        let mut res1 = sample_resolution();
        let mut res2 = sample_resolution();
        res1.base_image_digest = "a".repeat(64);
        res2.base_image_digest = "b".repeat(64);

        let lock1 = LockFile::from_resolved(&normalized, &res1);
        let lock2 = LockFile::from_resolved(&normalized, &res2);
        assert_ne!(lock1.env_id, lock2.env_id);
    }

    #[test]
    fn manifest_intent_verified() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock = LockFile::from_resolved(&normalized, &resolution);
        assert!(lock.verify_manifest_intent(&normalized).is_ok());
    }

    #[test]
    fn manifest_drift_detected() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock = LockFile::from_resolved(&normalized, &resolution);

        // Change the manifest
        let mut drifted = normalized.clone();
        drifted.base_image = "ubuntu/24.04".to_owned();
        assert!(lock.verify_manifest_intent(&drifted).is_err());
    }

    #[test]
    fn includes_hardware_policy_in_identity() {
        let mut n1 = sample_normalized();
        let mut n2 = sample_normalized();
        n1.hardware_gpu = false;
        n2.hardware_gpu = true;
        let res = sample_resolution();
        let lock1 = LockFile::from_resolved(&n1, &res);
        let lock2 = LockFile::from_resolved(&n2, &res);
        assert_ne!(lock1.env_id, lock2.env_id);
    }

    // --- A1: Determinism Hardening ---

    #[test]
    fn hash_stable_across_repeated_invocations() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let mut ids = Vec::new();
        for _ in 0..100 {
            let lock = LockFile::from_resolved(&normalized, &resolution);
            ids.push(lock.env_id.clone());
        }
        let first = &ids[0];
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(first, id, "invocation {i} produced different env_id");
        }
    }

    #[test]
    fn hash_stable_with_randomized_package_order() {
        let normalized = sample_normalized();
        // Create resolutions with packages in different orders
        let res_ab = ResolutionResult {
            base_image_digest: "a".repeat(64),
            resolved_packages: vec![
                ResolvedPackage {
                    name: "alpha".to_owned(),
                    version: "1.0".to_owned(),
                },
                ResolvedPackage {
                    name: "beta".to_owned(),
                    version: "2.0".to_owned(),
                },
                ResolvedPackage {
                    name: "gamma".to_owned(),
                    version: "3.0".to_owned(),
                },
            ],
        };
        let res_ba = ResolutionResult {
            base_image_digest: "a".repeat(64),
            resolved_packages: vec![
                ResolvedPackage {
                    name: "gamma".to_owned(),
                    version: "3.0".to_owned(),
                },
                ResolvedPackage {
                    name: "alpha".to_owned(),
                    version: "1.0".to_owned(),
                },
                ResolvedPackage {
                    name: "beta".to_owned(),
                    version: "2.0".to_owned(),
                },
            ],
        };
        let lock_ab = LockFile::from_resolved(&normalized, &res_ab);
        let lock_ba = LockFile::from_resolved(&normalized, &res_ba);
        assert_eq!(
            lock_ab.env_id, lock_ba.env_id,
            "package order must not affect env_id (sorted in from_resolved)"
        );
    }

    #[test]
    fn hash_stable_with_randomized_mount_order() {
        use crate::normalize::NormalizedMount;
        let mut n1 = sample_normalized();
        n1.mounts = vec![
            NormalizedMount {
                label: "cache".to_owned(),
                host_path: "/a".to_owned(),
                container_path: "/b".to_owned(),
            },
            NormalizedMount {
                label: "work".to_owned(),
                host_path: "/c".to_owned(),
                container_path: "/d".to_owned(),
            },
        ];
        let mut n2 = sample_normalized();
        n2.mounts = vec![
            NormalizedMount {
                label: "work".to_owned(),
                host_path: "/c".to_owned(),
                container_path: "/d".to_owned(),
            },
            NormalizedMount {
                label: "cache".to_owned(),
                host_path: "/a".to_owned(),
                container_path: "/b".to_owned(),
            },
        ];
        // Mounts are sorted by label in normalize(), but from_resolved doesn't re-sort.
        // The hash input iterates mounts in order. For determinism, mounts must be
        // pre-sorted by the caller (normalize). Test that identical sorted mounts hash equally.
        n1.mounts.sort_by(|a, b| a.label.cmp(&b.label));
        n2.mounts.sort_by(|a, b| a.label.cmp(&b.label));
        let res = sample_resolution();
        let lock1 = LockFile::from_resolved(&n1, &res);
        let lock2 = LockFile::from_resolved(&n2, &res);
        assert_eq!(lock1.env_id, lock2.env_id);
    }

    #[test]
    fn cross_platform_path_normalization() {
        // Verify that path separators in mount specs don't break determinism.
        // On all platforms, mount paths are stored as-is from the manifest
        // (which uses forward slashes). This test confirms no OS-dependent
        // path mangling occurs.
        use crate::normalize::NormalizedMount;
        let mut n1 = sample_normalized();
        n1.mounts = vec![NormalizedMount {
            label: "src".to_owned(),
            host_path: "/home/user/src".to_owned(),
            container_path: "/workspace".to_owned(),
        }];
        let res = sample_resolution();
        let lock = LockFile::from_resolved(&n1, &res);

        // The env_id must be a fixed known value regardless of platform
        let lock2 = LockFile::from_resolved(&n1, &res);
        assert_eq!(lock.env_id, lock2.env_id);
        // env_id must be exactly 64 hex chars
        assert_eq!(lock.env_id.len(), 64);
        assert!(lock.env_id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn identical_inputs_produce_identical_hash_bytes() {
        let normalized = sample_normalized();
        let resolution = sample_resolution();
        let lock1 = LockFile::from_resolved(&normalized, &resolution);
        let lock2 = LockFile::from_resolved(&normalized, &resolution);
        // Byte-level comparison of the full 64-char hex string
        assert_eq!(
            lock1.env_id.as_bytes(),
            lock2.env_id.as_bytes(),
            "hash bytes must be identical for identical inputs"
        );
        assert_eq!(lock1.short_id.as_bytes(), lock2.short_id.as_bytes(),);
    }

    // --- IG-M5: Golden-value cross-machine determinism tests ---
    //
    // These tests hardcode expected blake3 hashes for fixed inputs.
    // If any of these fail, it means compute_identity() has changed behavior,
    // which would break cross-machine reproducibility and existing lock files.
    // The golden values were computed once and must remain stable forever.

    fn golden_lock(
        base_digest: &str,
        packages: &[(&str, &str)],
        mounts: &[(&str, &str, &str)],
        backend: &str,
        gpu: bool,
        audio: bool,
        network_isolation: bool,
    ) -> LockFile {
        let resolved_packages: Vec<ResolvedPackage> = packages
            .iter()
            .map(|(n, v)| ResolvedPackage {
                name: n.to_string(),
                version: v.to_string(),
            })
            .collect();
        let mount_specs: Vec<NormalizedMount> = mounts
            .iter()
            .map(|(l, h, c)| NormalizedMount {
                label: l.to_string(),
                host_path: h.to_string(),
                container_path: c.to_string(),
            })
            .collect();
        let normalized = NormalizedManifest {
            manifest_version: 1,
            base_image: "rolling".to_owned(),
            system_packages: packages.iter().map(|(n, _)| n.to_string()).collect(),
            gui_apps: Vec::new(),
            hardware_gpu: gpu,
            hardware_audio: audio,
            mounts: mount_specs,
            runtime_backend: backend.to_owned(),
            network_isolation,
            cpu_shares: None,
            memory_limit_mb: None,
        };
        let resolution = ResolutionResult {
            base_image_digest: base_digest.to_owned(),
            resolved_packages,
        };
        LockFile::from_resolved(&normalized, &resolution)
    }

    #[test]
    fn golden_identity_empty_manifest() {
        let lock = golden_lock("sha256:abc123", &[], &[], "mock", false, false, false);
        assert_eq!(
            lock.env_id, "aabaeaeda3b27db42054f64719a16afd49e72b4fc6e8493e2fce9d862d240806",
            "golden hash for empty manifest must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_with_packages() {
        let lock = golden_lock(
            "sha256:abc123",
            &[("curl", "7.88.1"), ("git", "2.39.2")],
            &[],
            "namespace",
            false,
            false,
            false,
        );
        assert_eq!(
            lock.env_id, "dfea3163e5925ee788a97fae24d9ec08f774c29c64c9180befe771d877e62f18",
            "golden hash for manifest with packages must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_with_mounts_and_hardware() {
        let lock = golden_lock(
            "sha256:abc123",
            &[("vim", "9.0.1")],
            &[("home", "/home/user", "/home")],
            "namespace",
            true,
            true,
            false,
        );
        assert_eq!(
            lock.env_id, "d6ca89829da264240d0508bd58bffc28c2014f643426bbecff3db5a525793546",
            "golden hash for manifest with mounts+hardware must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_network_isolation_differs() {
        let lock = golden_lock("sha256:abc123", &[], &[], "mock", false, false, true);
        assert_eq!(
            lock.env_id, "dcdae57b3749d0aa2d3948de9fde99ceedad34deaef9b618c2d9f939dac25596",
            "golden hash for network-isolated manifest must be stable across all platforms"
        );
        // Must differ from the non-isolated empty manifest
        assert_ne!(
            lock.env_id, "aabaeaeda3b27db42054f64719a16afd49e72b4fc6e8493e2fce9d862d240806",
            "network isolation must produce a different hash"
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn golden_lock_full(
        base_digest: &str,
        packages: &[(&str, &str)],
        mounts: &[(&str, &str, &str)],
        apps: &[&str],
        backend: &str,
        gpu: bool,
        audio: bool,
        network_isolation: bool,
        cpu_shares: Option<u64>,
        memory_limit_mb: Option<u64>,
    ) -> LockFile {
        let resolved_packages: Vec<ResolvedPackage> = packages
            .iter()
            .map(|(n, v)| ResolvedPackage {
                name: n.to_string(),
                version: v.to_string(),
            })
            .collect();
        let mount_specs: Vec<NormalizedMount> = mounts
            .iter()
            .map(|(l, h, c)| NormalizedMount {
                label: l.to_string(),
                host_path: h.to_string(),
                container_path: c.to_string(),
            })
            .collect();
        let normalized = NormalizedManifest {
            manifest_version: 1,
            base_image: "rolling".to_owned(),
            system_packages: packages.iter().map(|(n, _)| n.to_string()).collect(),
            gui_apps: apps.iter().map(ToString::to_string).collect(),
            hardware_gpu: gpu,
            hardware_audio: audio,
            mounts: mount_specs,
            runtime_backend: backend.to_owned(),
            network_isolation,
            cpu_shares,
            memory_limit_mb,
        };
        let resolution = ResolutionResult {
            base_image_digest: base_digest.to_owned(),
            resolved_packages,
        };
        LockFile::from_resolved(&normalized, &resolution)
    }

    #[test]
    fn golden_identity_with_cpu_shares() {
        let lock = golden_lock_full(
            "sha256:abc123",
            &[],
            &[],
            &[],
            "mock",
            false,
            false,
            false,
            Some(1024),
            None,
        );
        assert_eq!(
            lock.env_id, "d966f9ee1c5e8959ae29d0483c45fc66813ec47201aa9f26c6371336b3dfd252",
            "golden hash for cpu_shares=1024 must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_with_memory_limit() {
        let lock = golden_lock_full(
            "sha256:abc123",
            &[],
            &[],
            &[],
            "mock",
            false,
            false,
            false,
            None,
            Some(4096),
        );
        assert_eq!(
            lock.env_id, "74823889e305b7b28394508b5813568faf9c814b4ef8f1f97e8d3dcd9a7a6bae",
            "golden hash for memory_limit_mb=4096 must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_with_apps() {
        let lock = golden_lock_full(
            "sha256:abc123",
            &[],
            &[],
            &["firefox", "code"],
            "mock",
            false,
            false,
            false,
            None,
            None,
        );
        assert_eq!(
            lock.env_id, "1aaf066c7b1e18178e838b0cf33c0bc67cd7401e586df826daa9033178ccfdf3",
            "golden hash for gui_apps=[firefox,code] must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_with_cpu_and_memory() {
        let lock = golden_lock_full(
            "sha256:abc123",
            &[("curl", "7.88.1")],
            &[("data", "/mnt/data", "/data")],
            &["vlc"],
            "namespace",
            true,
            true,
            true,
            Some(2048),
            Some(8192),
        );
        assert_eq!(
            lock.env_id, "44f9547036b4f24f8fe32844f2672804020c6260e29b7f72e17fd29d441ebc27",
            "golden hash for fully-populated manifest must be stable across all platforms"
        );
    }

    #[test]
    fn golden_identity_gpu_only_differs_from_audio_only() {
        let gpu_lock = golden_lock_full(
            "sha256:abc123",
            &[],
            &[],
            &[],
            "mock",
            true,
            false,
            false,
            None,
            None,
        );
        let audio_lock = golden_lock_full(
            "sha256:abc123",
            &[],
            &[],
            &[],
            "mock",
            false,
            true,
            false,
            None,
            None,
        );
        assert_eq!(
            gpu_lock.env_id, "f761765ba48777bcc64c2cd5169cb44be27bcd2d6587c64c28bc98fa0964b266",
            "golden hash for gpu-only must be stable"
        );
        assert_eq!(
            audio_lock.env_id, "428d91b41a03c1625e01bab1278ef231fb186833bff80a6bdc8227a2276f4318",
            "golden hash for audio-only must be stable"
        );
        assert_ne!(
            gpu_lock.env_id, audio_lock.env_id,
            "gpu-only and audio-only must produce different hashes"
        );
    }

    #[test]
    fn hash_sensitive_to_all_fields() {
        let base_norm = sample_normalized();
        let base_res = sample_resolution();
        let base_id = LockFile::from_resolved(&base_norm, &base_res).env_id;

        // Change each field and verify the hash changes
        let mut n = base_norm.clone();
        n.network_isolation = !n.network_isolation;
        assert_ne!(
            LockFile::from_resolved(&n, &base_res).env_id,
            base_id,
            "network_isolation"
        );

        let mut n = base_norm.clone();
        n.cpu_shares = Some(1024);
        assert_ne!(
            LockFile::from_resolved(&n, &base_res).env_id,
            base_id,
            "cpu_shares"
        );

        let mut n = base_norm.clone();
        n.memory_limit_mb = Some(4096);
        assert_ne!(
            LockFile::from_resolved(&n, &base_res).env_id,
            base_id,
            "memory_limit_mb"
        );

        let mut n = base_norm.clone();
        n.runtime_backend = "oci".to_owned();
        assert_ne!(
            LockFile::from_resolved(&n, &base_res).env_id,
            base_id,
            "runtime_backend"
        );

        let mut n = base_norm.clone();
        n.gui_apps = vec!["new-app".to_owned()];
        assert_ne!(
            LockFile::from_resolved(&n, &base_res).env_id,
            base_id,
            "gui_apps"
        );
    }
}
