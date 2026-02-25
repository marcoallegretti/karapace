use crate::concurrency::StoreLock;
use crate::lifecycle::validate_transition;
use crate::CoreError;
use karapace_runtime::backend::{select_backend, RuntimeSpec};
use karapace_runtime::SecurityPolicy;
use karapace_schema::types::{LayerHash, ObjectHash};
use karapace_schema::{
    compute_env_id, parse_manifest_file, EnvIdentity, LockFile, ManifestV1, NormalizedManifest,
    ResolutionResult,
};
use karapace_store::{
    pack_layer, unpack_layer, EnvMetadata, EnvState, LayerKind, LayerManifest, LayerStore,
    MetadataStore, ObjectStore, RollbackStep, StoreLayout, WalOpKind, WriteAheadLog,
};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Central orchestration engine for Karapace environment lifecycle.
///
/// Coordinates manifest parsing, object storage, layer management, and runtime
/// backends to provide build, enter, exec, stop, destroy, and inspection operations.
pub struct Engine {
    layout: StoreLayout,
    /// Cached lossy UTF-8 representation of the store root, avoiding repeated
    /// `to_string_lossy()` allocations on every engine operation.
    store_root_str: String,
    meta_store: MetadataStore,
    obj_store: ObjectStore,
    layer_store: LayerStore,
    wal: WriteAheadLog,
}

/// Result of a successful environment build.
pub struct BuildResult {
    pub identity: EnvIdentity,
    pub lock_file: LockFile,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuildOptions {
    pub locked: bool,
    pub offline: bool,
    pub require_pinned_image: bool,
}

impl Engine {
    /// Create a new engine rooted at the given store directory.
    ///
    /// On construction, the WAL is scanned for incomplete entries from
    /// previous runs and any orphaned state is rolled back automatically.
    pub fn new(store_root: impl Into<PathBuf>) -> Self {
        let root: PathBuf = store_root.into();
        let layout = StoreLayout::new(&root);
        let meta_store = MetadataStore::new(layout.clone());
        let obj_store = ObjectStore::new(layout.clone());
        let layer_store = LayerStore::new(layout.clone());
        let wal = WriteAheadLog::new(&layout);

        // Recovery mutates the store; avoid running it while the store is locked.
        match StoreLock::try_acquire(&layout.lock_file()) {
            Ok(Some(_lock)) => {
                if let Err(e) = wal.recover() {
                    warn!("WAL recovery failed: {e}");
                }

                // Clean up stale .running markers.
                let env_base = layout.env_dir();
                if env_base.exists() {
                    if let Ok(entries) = std::fs::read_dir(&env_base) {
                        for entry in entries.flatten() {
                            let running_marker = entry.path().join(".running");
                            if running_marker.exists() {
                                debug!(
                                    "removing stale .running marker: {}",
                                    running_marker.display()
                                );
                                let _ = std::fs::remove_file(&running_marker);
                            }
                        }
                    }
                }
            }
            Ok(None) => {
                debug!("store lock held; skipping WAL recovery and stale marker cleanup");
            }
            Err(e) => {
                warn!("store lock check failed; skipping WAL recovery: {e}");
            }
        }

        let store_root_str = root.to_string_lossy().into_owned();
        Self {
            layout,
            store_root_str,
            meta_store,
            obj_store,
            layer_store,
            wal,
        }
    }

    /// Initialize an environment from a manifest without building it.
    pub fn init(&self, manifest_path: &Path) -> Result<BuildResult, CoreError> {
        info!("initializing environment from {}", manifest_path.display());
        self.layout.initialize()?;

        let manifest = parse_manifest_file(manifest_path)?;
        let normalized = manifest.normalize()?;

        let identity = compute_env_id(&normalized)?;

        if !self.meta_store.exists(&identity.env_id) {
            let manifest_json = normalized.canonical_json()?;
            let manifest_hash = self.obj_store.put(manifest_json.as_bytes())?;

            let now = chrono::Utc::now().to_rfc3339();
            let meta = EnvMetadata {
                env_id: identity.env_id.clone(),
                short_id: identity.short_id.clone(),
                name: None,
                state: EnvState::Defined,
                manifest_hash: ObjectHash::new(manifest_hash),
                base_layer: LayerHash::new(""),
                dependency_layers: Vec::new(),
                policy_layer: None,
                created_at: now.clone(),
                updated_at: now,
                ref_count: 1,
                checksum: None,
            };
            self.meta_store.put(&meta)?;
        }

        let preliminary_resolution = ResolutionResult {
            base_image_digest: blake3::hash(
                format!("unresolved:{}", normalized.base_image).as_bytes(),
            )
            .to_hex()
            .to_string(),
            resolved_packages: normalized
                .system_packages
                .iter()
                .map(|name| karapace_schema::ResolvedPackage {
                    name: name.clone(),
                    version: "unresolved".to_owned(),
                })
                .collect(),
        };
        let lock = LockFile::from_resolved(&normalized, &preliminary_resolution);

        let lock_path = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("karapace.lock");
        lock.write_to_file(&lock_path)?;

        Ok(BuildResult {
            identity,
            lock_file: lock,
        })
    }

    pub fn build(&self, manifest_path: &Path) -> Result<BuildResult, CoreError> {
        self.build_with_options(manifest_path, BuildOptions::default())
    }

    #[allow(clippy::too_many_lines)]
    pub fn build_with_options(
        &self,
        manifest_path: &Path,
        options: BuildOptions,
    ) -> Result<BuildResult, CoreError> {
        info!("building environment from {}", manifest_path.display());
        self.layout.initialize()?;

        let manifest = parse_manifest_file(manifest_path)?;
        let normalized = manifest.normalize()?;

        if options.offline && !normalized.system_packages.is_empty() {
            return Err(CoreError::Runtime(
                karapace_runtime::RuntimeError::ExecFailed(
                    "offline mode: cannot resolve system packages".to_owned(),
                ),
            ));
        }

        if options.require_pinned_image
            && !(normalized.base_image.starts_with("http://")
                || normalized.base_image.starts_with("https://"))
        {
            return Err(CoreError::Manifest(
                karapace_schema::ManifestError::UnpinnedBaseImage(normalized.base_image.clone()),
            ));
        }

        let lock_path = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("karapace.lock");

        let locked = if options.locked {
            let lock = LockFile::read_from_file(&lock_path)?;
            let _ = lock.verify_integrity()?;
            lock.verify_manifest_intent(&normalized)?;
            Some(lock)
        } else {
            None
        };

        let policy = SecurityPolicy::from_manifest(&normalized);
        policy.validate_mounts(&normalized)?;
        policy.validate_devices(&normalized)?;
        policy.validate_resource_limits(&normalized)?;

        let store_str = self.store_root_str.clone();
        let backend = select_backend(&normalized.runtime_backend, &store_str)?;

        let preliminary_id = compute_env_id(&normalized)?;
        let preliminary_spec = RuntimeSpec {
            env_id: preliminary_id.env_id.to_string(),
            root_path: self
                .layout
                .env_path(&preliminary_id.env_id)
                .to_string_lossy()
                .to_string(),
            overlay_path: self
                .layout
                .env_path(&preliminary_id.env_id)
                .to_string_lossy()
                .to_string(),
            store_root: store_str.clone(),
            manifest: normalized.clone(),
            offline: options.offline,
        };
        let resolution = backend.resolve(&preliminary_spec)?;
        debug!(
            "resolved {} packages, base digest {}",
            resolution.resolved_packages.len(),
            &resolution.base_image_digest[..12]
        );

        let lock = LockFile::from_resolved(&normalized, &resolution);
        let identity = lock.compute_identity();

        if let Some(existing) = locked {
            if existing.env_id != identity.env_id.as_str() {
                return Err(CoreError::Lock(karapace_schema::LockError::ManifestDrift(
                    format!(
                        "locked mode: lock env_id '{}' does not match resolved env_id '{}'",
                        existing.env_id, identity.env_id
                    ),
                )));
            }
        }

        info!(
            "canonical env_id: {} ({})",
            identity.env_id, identity.short_id
        );

        let manifest_json = normalized.canonical_json()?;
        let manifest_hash = self.obj_store.put(manifest_json.as_bytes())?;

        let env_dir = self.layout.env_path(&identity.env_id);

        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Build, &identity.env_id)?;

        // Register rollback before creating side effects.
        self.wal
            .add_rollback_step(&wal_op, RollbackStep::RemoveDir(env_dir.clone()))?;
        std::fs::create_dir_all(&env_dir)?;

        let spec = RuntimeSpec {
            env_id: identity.env_id.to_string(),
            root_path: env_dir.to_string_lossy().to_string(),
            overlay_path: env_dir.to_string_lossy().to_string(),
            store_root: store_str,
            manifest: normalized.clone(),
            offline: options.offline,
        };
        if let Err(e) = backend.build(&spec) {
            let _ = std::fs::remove_dir_all(&env_dir);
            let _ = self.wal.commit(&wal_op);
            return Err(e.into());
        }

        let upper_dir = self.layout.upper_dir(&identity.env_id);
        let build_tar = if upper_dir.exists() {
            pack_layer(&upper_dir)?
        } else {
            Vec::new()
        };
        let build_tar_hash = self.obj_store.put(&build_tar)?;
        debug!(
            "captured build layer: {} bytes, hash {}",
            build_tar.len(),
            &build_tar_hash[..12]
        );

        let base_layer = LayerManifest {
            hash: build_tar_hash.clone(),
            kind: LayerKind::Base,
            parent: None,
            object_refs: vec![build_tar_hash.clone()],
            read_only: true,
            tar_hash: build_tar_hash.clone(),
        };
        let base_layer_hash = self.layer_store.put(&base_layer)?;

        let dep_layers = Vec::new();

        let now = chrono::Utc::now().to_rfc3339();
        let meta = EnvMetadata {
            env_id: identity.env_id.clone(),
            short_id: identity.short_id.clone(),
            name: None,
            state: EnvState::Built,
            manifest_hash: ObjectHash::new(manifest_hash),
            base_layer: LayerHash::new(base_layer_hash),
            dependency_layers: dep_layers,
            policy_layer: None,
            created_at: now.clone(),
            updated_at: now,
            ref_count: 1,
            checksum: None,
        };

        let finalize = || -> Result<(), CoreError> {
            if let Ok(existing) = self.meta_store.get(&identity.env_id) {
                validate_transition(existing.state, EnvState::Built)?;
            }
            self.meta_store.put(&meta)?;

            if !options.locked {
                lock.write_to_file(&lock_path)?;
            }
            Ok(())
        };

        if let Err(e) = finalize() {
            warn!("post-build finalization failed, cleaning up env_dir: {e}");
            let _ = std::fs::remove_dir_all(&env_dir);
            let _ = self.wal.commit(&wal_op);
            return Err(e);
        }

        // Build succeeded — commit WAL (removes entry)
        self.wal.commit(&wal_op)?;

        Ok(BuildResult {
            identity,
            lock_file: lock,
        })
    }

    fn load_manifest(&self, manifest_hash: &str) -> Result<NormalizedManifest, CoreError> {
        let data = self.obj_store.get(manifest_hash)?;
        Ok(serde_json::from_slice(&data)?)
    }

    fn prepare_spec(&self, env_id: &str, manifest: NormalizedManifest) -> RuntimeSpec {
        let env_path_str = self.layout.env_path(env_id).to_string_lossy().into_owned();
        RuntimeSpec {
            env_id: env_id.to_owned(),
            root_path: env_path_str.clone(),
            overlay_path: env_path_str,
            store_root: self.store_root_str.clone(),
            manifest,
            offline: false,
        }
    }

    pub fn enter(&self, env_id: &str) -> Result<(), CoreError> {
        info!("entering environment {env_id}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        if meta.state == EnvState::Running {
            return Err(CoreError::Runtime(
                karapace_runtime::RuntimeError::AlreadyRunning(env_id.to_owned()),
            ));
        }
        if meta.state != EnvState::Built {
            return Err(CoreError::InvalidTransition {
                from: meta.state.to_string(),
                to: "enter requires built state".to_owned(),
            });
        }

        let normalized = self.load_manifest(&meta.manifest_hash)?;
        let store_str = self.store_root_str.clone();
        let backend = select_backend(&normalized.runtime_backend, &store_str)?;
        let spec = self.prepare_spec(env_id, normalized);

        // WAL: if we crash while Running, recover back to Built
        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Enter, env_id)?;
        self.wal.add_rollback_step(
            &wal_op,
            RollbackStep::ResetState {
                env_id: env_id.to_owned(),
                target_state: "Built".to_owned(),
            },
        )?;

        self.meta_store.update_state(env_id, EnvState::Running)?;
        if let Err(e) = backend.enter(&spec) {
            let _ = self.meta_store.update_state(env_id, EnvState::Built);
            let _ = self.wal.commit(&wal_op);
            return Err(e.into());
        }
        self.meta_store.update_state(env_id, EnvState::Built)?;
        self.wal.commit(&wal_op)?;

        Ok(())
    }

    pub fn exec(&self, env_id: &str, command: &[String]) -> Result<(), CoreError> {
        info!("exec in environment {env_id}: {command:?}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        if meta.state == EnvState::Running {
            return Err(CoreError::Runtime(
                karapace_runtime::RuntimeError::AlreadyRunning(env_id.to_owned()),
            ));
        }
        if meta.state != EnvState::Built {
            return Err(CoreError::InvalidTransition {
                from: meta.state.to_string(),
                to: "exec requires built state".to_owned(),
            });
        }

        let normalized = self.load_manifest(&meta.manifest_hash)?;
        let store_str = self.store_root_str.clone();
        let backend = select_backend(&normalized.runtime_backend, &store_str)?;
        let spec = self.prepare_spec(env_id, normalized);

        // WAL: if we crash while Running, recover back to Built
        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Exec, env_id)?;
        self.wal.add_rollback_step(
            &wal_op,
            RollbackStep::ResetState {
                env_id: env_id.to_owned(),
                target_state: "Built".to_owned(),
            },
        )?;

        self.meta_store.update_state(env_id, EnvState::Running)?;
        let result = backend.exec(&spec, command);
        let _ = self.meta_store.update_state(env_id, EnvState::Built);
        let _ = self.wal.commit(&wal_op);

        match result {
            Ok(output) => {
                use std::io::Write;
                let _ = std::io::stdout().write_all(&output.stdout);
                let _ = std::io::stderr().write_all(&output.stderr);
                if output.status.success() {
                    Ok(())
                } else {
                    let detail = if let Some(code) = output.status.code() {
                        format!("command exited with code {code}")
                    } else {
                        #[cfg(unix)]
                        {
                            use std::os::unix::process::ExitStatusExt;
                            match output.status.signal() {
                                Some(sig) => format!("command killed by signal {sig}"),
                                None => "command failed with unknown status".to_owned(),
                            }
                        }
                        #[cfg(not(unix))]
                        {
                            "command failed with unknown status".to_owned()
                        }
                    };
                    Err(CoreError::Runtime(
                        karapace_runtime::RuntimeError::ExecFailed(detail),
                    ))
                }
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn stop(&self, env_id: &str) -> Result<(), CoreError> {
        info!("stopping environment {env_id}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        if meta.state != EnvState::Running {
            return Err(CoreError::Runtime(
                karapace_runtime::RuntimeError::NotRunning(format!(
                    "{} (state: {})",
                    env_id, meta.state
                )),
            ));
        }

        let normalized = self.load_manifest(&meta.manifest_hash)?;
        let store_str = self.store_root_str.clone();
        let backend = select_backend(&normalized.runtime_backend, &store_str)?;
        let status = backend.status(env_id)?;

        if let Some(pid) = status.pid {
            let pid_i32 = i32::try_from(pid).map_err(|_| {
                CoreError::Runtime(karapace_runtime::RuntimeError::ExecFailed(format!(
                    "invalid pid {pid}: exceeds i32 range"
                )))
            })?;
            debug!("sending SIGTERM to pid {pid}");
            // SAFETY: kill() with a valid pid and signal is safe; pid validated via i32::try_from above.
            #[allow(unsafe_code)]
            let term_ret = unsafe { libc::kill(pid_i32, libc::SIGTERM) };
            if term_ret != 0 {
                let errno = std::io::Error::last_os_error();
                if errno.raw_os_error() == Some(libc::ESRCH) {
                    debug!("pid {pid} already exited before SIGTERM");
                } else {
                    return Err(CoreError::Runtime(
                        karapace_runtime::RuntimeError::ExecFailed(format!(
                            "failed to send SIGTERM to pid {pid}: {errno}"
                        )),
                    ));
                }
            } else {
                // Give it a moment to clean up
                std::thread::sleep(std::time::Duration::from_millis(500));
                // Force kill if still running
                if Path::new(&format!("/proc/{pid}")).exists() {
                    warn!("process {pid} did not exit after SIGTERM, sending SIGKILL");
                    // SAFETY: same as above — valid pid and signal.
                    #[allow(unsafe_code)]
                    let kill_ret = unsafe { libc::kill(pid_i32, libc::SIGKILL) };
                    if kill_ret != 0 {
                        let errno = std::io::Error::last_os_error();
                        if errno.raw_os_error() != Some(libc::ESRCH) {
                            warn!("failed to send SIGKILL to pid {pid}: {errno}");
                        }
                    }
                }
            }
        }

        // Clean up running marker
        let running_file = self.layout.env_path(env_id).join(".running");
        let _ = std::fs::remove_file(running_file);

        self.meta_store.update_state(env_id, EnvState::Built)?;
        Ok(())
    }

    pub fn destroy(&self, env_id: &str) -> Result<(), CoreError> {
        info!("destroying environment {env_id}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        if meta.state == EnvState::Running {
            return Err(CoreError::InvalidTransition {
                from: "Running".to_owned(),
                to: "cannot destroy a running environment; stop it first".to_owned(),
            });
        }

        let normalized = self.load_manifest(&meta.manifest_hash)?;
        let store_str = self.store_root_str.clone();
        let backend = select_backend(&normalized.runtime_backend, &store_str)?;
        let spec = self.prepare_spec(env_id, normalized);

        // Begin WAL entry BEFORE any side-effects (including backend.destroy).
        // If the backend cleans up runtime state but we crash before metadata
        // removal, recovery will complete the cleanup on next startup.
        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Destroy, env_id)?;

        let env_dir = self.layout.env_path(env_id);
        // Register rollback steps BEFORE side-effects. Destroy rollback is
        // best-effort: re-running destroy on an already-destroyed env is safe.
        self.wal
            .add_rollback_step(&wal_op, RollbackStep::RemoveDir(env_dir.clone()))?;

        if let Err(e) = backend.destroy(&spec) {
            let _ = self.wal.commit(&wal_op);
            return Err(e.into());
        }
        if env_dir.exists() {
            std::fs::remove_dir_all(&env_dir)?;
        }

        let metadata_path = self.layout.metadata_dir().join(env_id);
        self.wal
            .add_rollback_step(&wal_op, RollbackStep::RemoveFile(metadata_path))?;
        let remaining = self.meta_store.decrement_ref(env_id)?;
        if remaining == 0 {
            let _ = self.meta_store.remove(env_id);
        }

        // Destroy succeeded — commit WAL (removes entry)
        self.wal.commit(&wal_op)?;

        Ok(())
    }

    pub fn rebuild(&self, manifest_path: &Path) -> Result<BuildResult, CoreError> {
        self.rebuild_with_options(manifest_path, BuildOptions::default())
    }

    pub fn rebuild_with_options(
        &self,
        manifest_path: &Path,
        options: BuildOptions,
    ) -> Result<BuildResult, CoreError> {
        // Collect the old env_id(s) to clean up AFTER a successful build.
        // This ensures we don't lose the old environment if the new build fails.
        let lock_path = manifest_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("karapace.lock");

        let mut old_env_ids: Vec<String> = Vec::new();
        if let Ok(lock) = LockFile::read_from_file(&lock_path) {
            if self.meta_store.exists(&lock.env_id) {
                old_env_ids.push(lock.env_id);
            }
        }
        if old_env_ids.is_empty() {
            let manifest = parse_manifest_file(manifest_path)?;
            let normalized = manifest.normalize()?;
            let identity = compute_env_id(&normalized)?;
            if self.meta_store.exists(&identity.env_id) {
                old_env_ids.push(identity.env_id.to_string());
            }
        }

        // Build first — if this fails, old environment is preserved.
        let result = self.build_with_options(manifest_path, options)?;

        // Only destroy the old environment(s) after the new build succeeds.
        for old_id in &old_env_ids {
            if *old_id != result.identity.env_id {
                if let Err(e) = self.destroy(old_id) {
                    warn!("failed to destroy old environment {old_id} during rebuild: {e}");
                }
                let _ = self.meta_store.remove(old_id);
            }
        }

        Ok(result)
    }

    pub fn inspect(&self, env_id: &str) -> Result<EnvMetadata, CoreError> {
        self.meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))
    }

    pub fn list(&self) -> Result<Vec<EnvMetadata>, CoreError> {
        Ok(self.meta_store.list()?)
    }

    pub fn freeze(&self, env_id: &str) -> Result<(), CoreError> {
        info!("freezing environment {env_id}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        validate_transition(meta.state, EnvState::Frozen)?;
        self.meta_store.update_state(env_id, EnvState::Frozen)?;
        Ok(())
    }

    pub fn archive(&self, env_id: &str) -> Result<(), CoreError> {
        info!("archiving environment {env_id}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        validate_transition(meta.state, EnvState::Archived)?;
        self.meta_store.update_state(env_id, EnvState::Archived)?;
        Ok(())
    }

    pub fn set_name(&self, env_id: &str, name: Option<String>) -> Result<(), CoreError> {
        self.meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;
        self.meta_store.update_name(env_id, name)?;
        Ok(())
    }

    pub fn rename(&self, env_id: &str, new_name: &str) -> Result<(), CoreError> {
        info!("renaming environment {env_id} to '{new_name}'");
        self.set_name(env_id, Some(new_name.to_owned()))
    }

    pub fn commit(&self, env_id: &str) -> Result<String, CoreError> {
        info!("committing overlay drift for {env_id}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        if meta.state != EnvState::Built && meta.state != EnvState::Frozen {
            return Err(CoreError::InvalidTransition {
                from: meta.state.to_string(),
                to: "commit requires built or frozen state".to_owned(),
            });
        }

        // Begin WAL entry for commit
        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Commit, env_id)?;

        // Pack the overlay upper directory as a deterministic tar layer.
        let upper_dir = self.layout.upper_dir(env_id);
        let tar_data = if upper_dir.exists() {
            pack_layer(&upper_dir)?
        } else {
            let _ = self.wal.commit(&wal_op);
            return Err(CoreError::EnvNotFound(format!(
                "no overlay upper directory for {env_id}"
            )));
        };

        let tar_hash = self.obj_store.put(&tar_data)?;
        debug!(
            "committed snapshot layer: {} bytes, hash {}",
            tar_data.len(),
            &tar_hash[..12]
        );

        // Compute a unique layer hash for this snapshot.
        // The tar_hash alone may collide with the base layer if the upper
        // dir content hasn't changed. Use a composite identity.
        let snapshot_id_input = format!("snapshot:{}:{}:{}", env_id, meta.base_layer, tar_hash);
        let snapshot_hash = blake3::hash(snapshot_id_input.as_bytes())
            .to_hex()
            .to_string();

        let snapshot_layer = LayerManifest {
            hash: snapshot_hash.clone(),
            kind: LayerKind::Snapshot,
            parent: Some(meta.base_layer.to_string()),
            object_refs: vec![tar_hash.clone()],
            read_only: true,
            tar_hash,
        };
        // Compute the content hash before writing so we can register the
        // correct rollback path. Uses LayerStore::compute_hash() to ensure
        // the hash matches what put() will produce.
        let content_hash = LayerStore::compute_hash(&snapshot_layer)?;

        // Register rollback for the snapshot layer manifest BEFORE writing it,
        // so a crash after put() but before WAL commit cleans up the orphan.
        let layer_path = self.layout.layers_dir().join(&content_hash);
        self.wal
            .add_rollback_step(&wal_op, RollbackStep::RemoveFile(layer_path))?;
        let stored_hash = self.layer_store.put(&snapshot_layer)?;

        // Commit succeeded — remove WAL entry
        self.wal.commit(&wal_op)?;

        Ok(stored_hash)
    }

    /// Restore an environment's overlay from a snapshot layer.
    ///
    /// Unpacks the snapshot tar into the overlay upper directory, replacing
    /// any current upper content. The operation is atomic: the old upper is
    /// only removed after the new content is fully unpacked in a staging dir.
    pub fn restore(&self, env_id: &str, snapshot_hash: &str) -> Result<(), CoreError> {
        info!("restoring {env_id} from snapshot {snapshot_hash}");
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        if meta.state != EnvState::Built && meta.state != EnvState::Frozen {
            return Err(CoreError::InvalidTransition {
                from: meta.state.to_string(),
                to: "restore requires built or frozen state".to_owned(),
            });
        }

        // Verify the snapshot layer exists and is a Snapshot kind.
        let layer = self.layer_store.get(snapshot_hash).map_err(|_| {
            CoreError::Store(karapace_store::StoreError::LayerNotFound(
                snapshot_hash.to_owned(),
            ))
        })?;
        if layer.kind != LayerKind::Snapshot {
            return Err(CoreError::InvalidTransition {
                from: format!("{:?}", layer.kind),
                to: "restore requires a Snapshot layer".to_owned(),
            });
        }
        if layer.tar_hash.is_empty() {
            return Err(CoreError::Store(karapace_store::StoreError::LayerNotFound(
                format!("snapshot {snapshot_hash} has no tar content (legacy layer)"),
            )));
        }

        // Retrieve the tar data from the object store.
        let tar_data = self.obj_store.get(&layer.tar_hash)?;

        // Begin WAL entry for restore
        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Restore, env_id)?;

        // Atomic restore: unpack to staging, then swap with current upper.
        let staging = self.layout.staging_dir().join(format!("restore-{env_id}"));

        // Register rollback BEFORE any staging dir operations so a crash
        // between create and registration cannot orphan the staging dir.
        self.wal
            .add_rollback_step(&wal_op, RollbackStep::RemoveDir(staging.clone()))?;

        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }

        unpack_layer(&tar_data, &staging)?;

        // Swap: remove old upper, rename staging to upper.
        let upper_dir = self.layout.upper_dir(env_id);
        if upper_dir.exists() {
            std::fs::remove_dir_all(&upper_dir)?;
        }
        std::fs::rename(&staging, &upper_dir)?;

        // Restore succeeded — remove WAL entry
        self.wal.commit(&wal_op)?;

        debug!("restored upper dir from snapshot {}", &snapshot_hash[..12]);
        Ok(())
    }

    /// List all snapshot layers associated with an environment.
    ///
    /// Returns snapshot `LayerManifest` entries whose parent matches
    /// the environment's base layer, ordered by hash.
    pub fn list_snapshots(&self, env_id: &str) -> Result<Vec<LayerManifest>, CoreError> {
        let meta = self
            .meta_store
            .get(env_id)
            .map_err(|_| CoreError::EnvNotFound(env_id.to_owned()))?;

        let all_hashes = self.layer_store.list()?;
        let mut snapshots = Vec::new();
        for hash in &all_hashes {
            if let Ok(layer) = self.layer_store.get(hash) {
                if layer.kind == LayerKind::Snapshot
                    && layer.parent.as_deref() == Some(&meta.base_layer)
                {
                    snapshots.push(layer);
                }
            }
        }
        snapshots.sort_by(|a, b| a.hash.cmp(&b.hash));
        Ok(snapshots)
    }

    /// Run garbage collection on the store.
    ///
    /// Requires a `&StoreLock` parameter as compile-time proof that the caller
    /// holds the store lock. The lock is not used internally — its presence in
    /// the signature enforces the invariant at the type level.
    pub fn gc(
        &self,
        _lock: &StoreLock,
        dry_run: bool,
    ) -> Result<karapace_store::GcReport, CoreError> {
        info!("running garbage collection (dry_run={dry_run})");

        // WAL marker: track GC in-flight. No rollback steps — GC is
        // inherently idempotent (orphaned items re-discovered on next run).
        // On recovery, an incomplete GC entry is logged and removed.
        self.wal.initialize()?;
        let wal_op = self.wal.begin(WalOpKind::Gc, "gc")?;

        let gc = karapace_store::GarbageCollector::new(self.layout.clone());
        let report = gc.collect_with_cancel(dry_run, crate::shutdown_requested)?;

        self.wal.commit(&wal_op)?;
        Ok(report)
    }

    /// Push an environment to a remote store.
    ///
    /// Transfers metadata, layers, and objects to the remote backend,
    /// skipping blobs that already exist. Optionally publishes under
    /// a registry tag (e.g. `"my-env@latest"`).
    pub fn push(
        &self,
        env_id: &str,
        backend: &dyn karapace_remote::RemoteBackend,
        registry_tag: Option<&str>,
    ) -> Result<karapace_remote::PushResult, CoreError> {
        info!("pushing environment {env_id}");
        Ok(karapace_remote::push_env(
            &self.layout,
            env_id,
            backend,
            registry_tag,
        )?)
    }

    /// Pull an environment from a remote store into the local store.
    ///
    /// Downloads metadata, layers, and objects from the remote backend,
    /// skipping blobs that already exist locally. Verifies blake3 integrity
    /// on all downloaded objects.
    pub fn pull(
        &self,
        env_id: &str,
        backend: &dyn karapace_remote::RemoteBackend,
    ) -> Result<karapace_remote::PullResult, CoreError> {
        info!("pulling environment {env_id}");
        self.layout.initialize()?;
        Ok(karapace_remote::pull_env(&self.layout, env_id, backend)?)
    }

    /// Resolve a registry reference to an env_id using the remote registry.
    pub fn resolve_remote_ref(
        backend: &dyn karapace_remote::RemoteBackend,
        reference: &str,
    ) -> Result<String, CoreError> {
        Ok(karapace_remote::resolve_ref(backend, reference)?)
    }

    pub fn store_layout(&self) -> &StoreLayout {
        &self.layout
    }

    pub fn resolve_manifest(
        &self,
        manifest_path: &Path,
    ) -> Result<(ManifestV1, NormalizedManifest, EnvIdentity), CoreError> {
        let manifest = parse_manifest_file(manifest_path)?;
        let normalized = manifest.normalize()?;
        let identity = compute_env_id(&normalized)?;
        Ok((manifest, normalized, identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> (tempfile::TempDir, Engine, tempfile::TempDir) {
        let store_dir = tempfile::tempdir().unwrap();
        let project_dir = tempfile::tempdir().unwrap();

        let manifest_content = r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "clang"]
[runtime]
backend = "mock"
"#;
        std::fs::write(project_dir.path().join("karapace.toml"), manifest_content).unwrap();

        let engine = Engine::new(store_dir.path());
        (store_dir, engine, project_dir)
    }

    #[test]
    fn init_creates_lock_and_metadata() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let result = engine.init(&manifest_path).unwrap();

        assert!(!result.identity.env_id.is_empty());
        assert!(project.path().join("karapace.lock").exists());
    }

    #[test]
    fn build_creates_environment() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let result = engine.build(&manifest_path).unwrap();

        let meta = engine.inspect(&result.identity.env_id).unwrap();
        assert_eq!(meta.state, EnvState::Built);
    }

    #[test]
    fn rebuild_produces_same_id() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let r1 = engine.build(&manifest_path).unwrap();
        let r2 = engine.rebuild(&manifest_path).unwrap();
        assert_eq!(r1.identity.env_id, r2.identity.env_id);
    }

    #[test]
    fn destroy_cleans_up() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let result = engine.build(&manifest_path).unwrap();
        engine.destroy(&result.identity.env_id).unwrap();
    }

    #[test]
    fn freeze_transitions_correctly() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let result = engine.build(&manifest_path).unwrap();
        engine.freeze(&result.identity.env_id).unwrap();

        let meta = engine.inspect(&result.identity.env_id).unwrap();
        assert_eq!(meta.state, EnvState::Frozen);
    }

    #[test]
    fn list_environments() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        engine.build(&manifest_path).unwrap();
        let envs = engine.list().unwrap();
        assert_eq!(envs.len(), 1);
    }

    #[test]
    fn archive_transitions_correctly() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let result = engine.build(&manifest_path).unwrap();
        engine.freeze(&result.identity.env_id).unwrap();
        engine.archive(&result.identity.env_id).unwrap();

        let meta = engine.inspect(&result.identity.env_id).unwrap();
        assert_eq!(meta.state, EnvState::Archived);
    }

    #[test]
    fn set_name_and_rename() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let result = engine.build(&manifest_path).unwrap();

        engine
            .set_name(&result.identity.env_id, Some("test-env".to_owned()))
            .unwrap();
        let meta = engine.inspect(&result.identity.env_id).unwrap();
        assert_eq!(meta.name, Some("test-env".to_owned()));

        engine.rename(&result.identity.env_id, "new-name").unwrap();
        let meta = engine.inspect(&result.identity.env_id).unwrap();
        assert_eq!(meta.name, Some("new-name".to_owned()));
    }

    #[test]
    fn inspect_nonexistent_fails() {
        let (_store, engine, _project) = test_engine();
        assert!(engine.inspect("nonexistent").is_err());
    }

    #[test]
    fn destroy_nonexistent_fails() {
        let (_store, engine, _project) = test_engine();
        engine.store_layout().initialize().unwrap();
        assert!(engine.destroy("nonexistent").is_err());
    }

    #[test]
    fn list_empty_store() {
        let (_store, engine, _project) = test_engine();
        engine.store_layout().initialize().unwrap();
        let envs = engine.list().unwrap();
        assert!(envs.is_empty());
    }

    #[test]
    fn init_is_idempotent() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let r1 = engine.init(&manifest_path).unwrap();
        let r2 = engine.init(&manifest_path).unwrap();
        assert_eq!(r1.identity.env_id, r2.identity.env_id);
    }

    #[test]
    fn build_creates_lock_file() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        engine.build(&manifest_path).unwrap();
        let lock_path = project.path().join("karapace.lock");
        assert!(lock_path.exists());
        let content = std::fs::read_to_string(&lock_path).unwrap();
        assert!(content.contains("lock_version"));
    }

    #[test]
    fn resolve_manifest_returns_identity() {
        let (_store, engine, project) = test_engine();
        let manifest_path = project.path().join("karapace.toml");
        let (manifest, normalized, identity) = engine.resolve_manifest(&manifest_path).unwrap();
        assert_eq!(manifest.manifest_version, 1);
        assert_eq!(normalized.base_image, "rolling");
        assert!(!identity.env_id.is_empty());
    }

    #[test]
    fn store_layout_accessor() {
        let (_store, engine, _project) = test_engine();
        let layout = engine.store_layout();
        assert!(layout.objects_dir().to_string_lossy().contains("objects"));
    }
}
