use criterion::{criterion_group, criterion_main, Criterion};
use std::fs;
use std::path::Path;

fn create_test_manifest(dir: &Path) -> std::path::PathBuf {
    let manifest_path = dir.join("karapace.toml");
    fs::write(
        &manifest_path,
        r#"
manifest_version = 1
[base]
image = "rolling"
[system]
packages = ["git", "clang"]
[runtime]
backend = "mock"
"#,
    )
    .unwrap();
    manifest_path
}

fn bench_build(c: &mut Criterion) {
    c.bench_function("engine_build_mock_2pkg", |b| {
        b.iter_with_setup(
            || {
                let store_dir = tempfile::tempdir().unwrap();
                let project_dir = tempfile::tempdir().unwrap();
                let manifest = create_test_manifest(project_dir.path());
                let engine = karapace_core::Engine::new(store_dir.path());
                (store_dir, project_dir, manifest, engine)
            },
            |(_sd, _pd, manifest, engine)| {
                engine.build(&manifest).unwrap();
            },
        );
    });
}

fn bench_rebuild_unchanged(c: &mut Criterion) {
    c.bench_function("engine_rebuild_unchanged", |b| {
        b.iter_with_setup(
            || {
                let store_dir = tempfile::tempdir().unwrap();
                let project_dir = tempfile::tempdir().unwrap();
                let manifest = create_test_manifest(project_dir.path());
                let engine = karapace_core::Engine::new(store_dir.path());
                let result = engine.build(&manifest).unwrap();
                (store_dir, project_dir, manifest, engine, result)
            },
            |(_sd, _pd, manifest, engine, _result)| {
                engine.build(&manifest).unwrap();
            },
        );
    });
}

fn bench_commit(c: &mut Criterion) {
    c.bench_function("engine_commit_100files", |b| {
        b.iter_with_setup(
            || {
                let store_dir = tempfile::tempdir().unwrap();
                let project_dir = tempfile::tempdir().unwrap();
                let manifest = create_test_manifest(project_dir.path());
                let engine = karapace_core::Engine::new(store_dir.path());
                let result = engine.build(&manifest).unwrap();
                let env_id = result.identity.env_id.to_string();

                // Create 100 files in the upper directory to simulate drift
                let upper = store_dir.path().join("env").join(&env_id).join("upper");
                fs::create_dir_all(&upper).unwrap();
                for i in 0..100 {
                    fs::write(
                        upper.join(format!("file_{i:03}.txt")),
                        format!("content {i}"),
                    )
                    .unwrap();
                }

                (store_dir, project_dir, engine, env_id)
            },
            |(_sd, _pd, engine, env_id)| {
                engine.commit(&env_id).unwrap();
            },
        );
    });
}

fn bench_restore(c: &mut Criterion) {
    c.bench_function("engine_restore_snapshot", |b| {
        b.iter_with_setup(
            || {
                let store_dir = tempfile::tempdir().unwrap();
                let project_dir = tempfile::tempdir().unwrap();
                let manifest = create_test_manifest(project_dir.path());
                let engine = karapace_core::Engine::new(store_dir.path());
                let result = engine.build(&manifest).unwrap();
                let env_id = result.identity.env_id.to_string();

                // Create files and commit a snapshot
                let upper = store_dir.path().join("env").join(&env_id).join("upper");
                fs::create_dir_all(&upper).unwrap();
                for i in 0..50 {
                    fs::write(
                        upper.join(format!("file_{i:03}.txt")),
                        format!("content {i}"),
                    )
                    .unwrap();
                }
                let snapshot_hash = engine.commit(&env_id).unwrap();

                (store_dir, project_dir, engine, env_id, snapshot_hash)
            },
            |(_sd, _pd, engine, env_id, snapshot_hash)| {
                engine.restore(&env_id, &snapshot_hash).unwrap();
            },
        );
    });
}

fn bench_gc(c: &mut Criterion) {
    c.bench_function("gc_50envs", |b| {
        b.iter_with_setup(
            || {
                let store_dir = tempfile::tempdir().unwrap();
                let layout = karapace_store::StoreLayout::new(store_dir.path());
                layout.initialize().unwrap();
                let meta_store = karapace_store::MetadataStore::new(layout.clone());
                let obj_store = karapace_store::ObjectStore::new(layout.clone());

                // Create 50 environments: 25 live (ref_count=1), 25 dead (ref_count=0)
                for i in 0..50 {
                    let obj_hash = obj_store.put(format!("obj-{i}").as_bytes()).unwrap();
                    let meta = karapace_store::EnvMetadata {
                        env_id: format!("env_{i:04}").into(),
                        short_id: format!("env_{i:04}").into(),
                        name: None,
                        state: karapace_store::EnvState::Built,
                        manifest_hash: obj_hash.into(),
                        base_layer: "".into(),
                        dependency_layers: vec![],
                        policy_layer: None,
                        created_at: "2026-01-01T00:00:00Z".to_owned(),
                        updated_at: "2026-01-01T00:00:00Z".to_owned(),
                        ref_count: u32::from(i < 25),
                        checksum: None,
                    };
                    meta_store.put(&meta).unwrap();
                }

                // Create 200 orphan objects
                for i in 0..200 {
                    obj_store
                        .put(format!("orphan-object-{i}").as_bytes())
                        .unwrap();
                }

                (store_dir, layout)
            },
            |(_sd, layout)| {
                let gc = karapace_store::GarbageCollector::new(layout);
                gc.collect(false).unwrap();
            },
        );
    });
}

fn bench_verify_store(c: &mut Criterion) {
    c.bench_function("verify_store_200objects", |b| {
        b.iter_with_setup(
            || {
                let store_dir = tempfile::tempdir().unwrap();
                let layout = karapace_store::StoreLayout::new(store_dir.path());
                layout.initialize().unwrap();
                let obj_store = karapace_store::ObjectStore::new(layout.clone());

                // Create 200 objects
                for i in 0..200 {
                    obj_store
                        .put(format!("verify-object-{i}").as_bytes())
                        .unwrap();
                }

                (store_dir, layout)
            },
            |(_sd, layout)| {
                karapace_store::verify_store_integrity(&layout).unwrap();
            },
        );
    });
}

criterion_group!(
    benches,
    bench_build,
    bench_rebuild_unchanged,
    bench_commit,
    bench_restore,
    bench_gc,
    bench_verify_store,
);
criterion_main!(benches);
