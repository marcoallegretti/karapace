use crate::layers::LayerStore;
use crate::layout::StoreLayout;
use crate::metadata::{EnvState, MetadataStore};
use crate::objects::ObjectStore;
use crate::StoreError;
use std::collections::HashSet;
use std::fs;

pub struct GarbageCollector {
    layout: StoreLayout,
}

#[derive(Debug, Default)]
pub struct GcReport {
    pub orphaned_envs: Vec<String>,
    pub orphaned_layers: Vec<String>,
    pub orphaned_objects: Vec<String>,
    pub removed_envs: usize,
    pub removed_layers: usize,
    pub removed_objects: usize,
}

impl GarbageCollector {
    pub fn new(layout: StoreLayout) -> Self {
        Self { layout }
    }

    pub fn collect(&self, dry_run: bool) -> Result<GcReport, StoreError> {
        self.collect_with_cancel(dry_run, || false)
    }

    pub fn collect_with_cancel(
        &self,
        dry_run: bool,
        should_stop: impl Fn() -> bool,
    ) -> Result<GcReport, StoreError> {
        let meta_store = MetadataStore::new(self.layout.clone());
        let layer_store = LayerStore::new(self.layout.clone());
        let object_store = ObjectStore::new(self.layout.clone());

        let mut report = GcReport::default();

        let all_meta = meta_store.list()?;
        let mut live_layers: HashSet<String> = HashSet::new();

        // Objects directly referenced by live environments (manifest hashes)
        let mut live_objects: HashSet<String> = HashSet::new();

        for meta in &all_meta {
            if meta.ref_count == 0
                && meta.state != EnvState::Running
                && meta.state != EnvState::Archived
            {
                report.orphaned_envs.push(meta.env_id.to_string());
            } else {
                live_layers.insert(meta.base_layer.to_string());
                for dep in &meta.dependency_layers {
                    live_layers.insert(dep.to_string());
                }
                if let Some(ref policy) = meta.policy_layer {
                    live_layers.insert(policy.to_string());
                }
                // Manifest object is directly referenced by metadata
                if !meta.manifest_hash.is_empty() {
                    live_objects.insert(meta.manifest_hash.to_string());
                }
            }
        }

        let all_layers = layer_store.list()?;

        // Preserve snapshot layers whose parent is a live layer.
        // Without this, snapshots created by commit() would be GC'd as orphans.
        for layer_hash in &all_layers {
            if !live_layers.contains(layer_hash) {
                if let Ok(layer) = layer_store.get(layer_hash) {
                    if layer.kind == crate::layers::LayerKind::Snapshot {
                        if let Some(ref parent) = layer.parent {
                            if live_layers.contains(parent) {
                                live_layers.insert(layer_hash.clone());
                            }
                        }
                    }
                }
            }
        }

        for layer_hash in &all_layers {
            if live_layers.contains(layer_hash) {
                if let Ok(layer) = layer_store.get(layer_hash) {
                    for obj_ref in &layer.object_refs {
                        live_objects.insert(obj_ref.clone());
                    }
                }
            } else {
                report.orphaned_layers.push(layer_hash.clone());
            }
        }

        let all_objects = object_store.list()?;
        for obj_hash in &all_objects {
            if !live_objects.contains(obj_hash) {
                report.orphaned_objects.push(obj_hash.clone());
            }
        }

        if !dry_run {
            for env_id in &report.orphaned_envs {
                if should_stop() {
                    break;
                }
                let env_path = self.layout.env_path(env_id);
                if env_path.exists() {
                    fs::remove_dir_all(&env_path)?;
                }
                meta_store.remove(env_id)?;
                report.removed_envs += 1;
            }

            for layer_hash in &report.orphaned_layers {
                if should_stop() {
                    break;
                }
                layer_store.remove(layer_hash)?;
                report.removed_layers += 1;
            }

            for obj_hash in &report.orphaned_objects {
                if should_stop() {
                    break;
                }
                object_store.remove(obj_hash)?;
                report.removed_objects += 1;
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::EnvMetadata;

    fn setup() -> (tempfile::TempDir, StoreLayout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = StoreLayout::new(dir.path());
        layout.initialize().unwrap();
        (dir, layout)
    }

    #[test]
    fn gc_removes_zero_refcount_envs() {
        let (_dir, layout) = setup();
        let meta_store = MetadataStore::new(layout.clone());

        let meta = EnvMetadata {
            env_id: "orphan1".into(),
            short_id: "orphan1".into(),
            name: None,
            state: EnvState::Built,
            manifest_hash: "mhash".into(),
            base_layer: "base1".into(),
            dependency_layers: vec![],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 0,
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let gc = GarbageCollector::new(layout);
        let report = gc.collect(false).unwrap();
        assert_eq!(report.removed_envs, 1);
    }

    #[test]
    fn gc_dry_run_does_not_remove() {
        let (_dir, layout) = setup();
        let meta_store = MetadataStore::new(layout.clone());

        let meta = EnvMetadata {
            env_id: "orphan2".into(),
            short_id: "orphan2".into(),
            name: None,
            state: EnvState::Defined,
            manifest_hash: "mhash".into(),
            base_layer: "base1".into(),
            dependency_layers: vec![],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 0,
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let gc = GarbageCollector::new(layout.clone());
        let report = gc.collect(true).unwrap();
        assert_eq!(report.orphaned_envs.len(), 1);
        assert_eq!(report.removed_envs, 0);
        assert!(meta_store.exists("orphan2"));
    }

    #[test]
    fn gc_preserves_manifest_objects() {
        let (_dir, layout) = setup();
        let meta_store = MetadataStore::new(layout.clone());
        let object_store = ObjectStore::new(layout.clone());

        // Create a manifest object
        let manifest_hash = object_store.put(b"manifest-content").unwrap();

        // Create a live environment referencing the manifest
        let meta = EnvMetadata {
            env_id: "live1".into(),
            short_id: "live1".into(),
            name: None,
            state: EnvState::Built,
            manifest_hash: manifest_hash.clone().into(),
            base_layer: "".into(),
            dependency_layers: vec![],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 1,
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let gc = GarbageCollector::new(layout.clone());
        let report = gc.collect(false).unwrap();

        // Manifest object must NOT be collected
        assert!(object_store.exists(&manifest_hash));
        assert!(!report.orphaned_objects.contains(&manifest_hash));
    }

    #[test]
    fn gc_preserves_archived_envs() {
        let (_dir, layout) = setup();
        let meta_store = MetadataStore::new(layout.clone());

        let meta = EnvMetadata {
            env_id: "archived1".into(),
            short_id: "archived1".into(),
            name: None,
            state: EnvState::Archived,
            manifest_hash: "mhash".into(),
            base_layer: "base1".into(),
            dependency_layers: vec![],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 0,
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let gc = GarbageCollector::new(layout);
        let report = gc.collect(false).unwrap();
        assert_eq!(report.removed_envs, 0, "archived envs must be preserved");
        assert!(report.orphaned_envs.is_empty());
    }

    #[test]
    fn gc_preserves_running_envs() {
        let (_dir, layout) = setup();
        let meta_store = MetadataStore::new(layout.clone());

        let meta = EnvMetadata {
            env_id: "running1".into(),
            short_id: "running1".into(),
            name: None,
            state: EnvState::Running,
            manifest_hash: "mhash".into(),
            base_layer: "base1".into(),
            dependency_layers: vec![],
            policy_layer: None,
            created_at: "2025-01-01T00:00:00Z".to_owned(),
            updated_at: "2025-01-01T00:00:00Z".to_owned(),
            ref_count: 0,
            checksum: None,
        };
        meta_store.put(&meta).unwrap();

        let gc = GarbageCollector::new(layout);
        let report = gc.collect(false).unwrap();
        assert_eq!(report.removed_envs, 0);
    }
}
