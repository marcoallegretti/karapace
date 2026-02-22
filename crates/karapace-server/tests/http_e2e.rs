//! IG-M3: HTTP client ↔ server E2E integration tests.
//!
//! These tests start a real `karapace-server` in-process on a random port
//! and exercise the real `HttpBackend` client against it. No mocks.

use karapace_remote::http::HttpBackend;
use karapace_remote::{BlobKind, RemoteBackend, RemoteConfig};
use karapace_server::TestServer;
use karapace_store::{
    EnvMetadata, EnvState, LayerKind, LayerManifest, LayerStore, MetadataStore, ObjectStore,
    StoreLayout,
};
fn start_server() -> (TestServer, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path().to_path_buf());
    (server, dir)
}

fn make_client(url: &str) -> HttpBackend {
    HttpBackend::new(RemoteConfig {
        url: url.to_owned(),
        auth_token: None,
    })
}

/// Create a local store with a mock-built environment for push/pull testing.
fn setup_local_env(dir: &std::path::Path) -> (StoreLayout, String) {
    let layout = StoreLayout::new(dir);
    layout.initialize().unwrap();

    let obj_store = ObjectStore::new(layout.clone());
    let layer_store = LayerStore::new(layout.clone());
    let meta_store = MetadataStore::new(layout.clone());

    let obj_hash = obj_store.put(b"test data content").unwrap();
    let manifest_hash = obj_store.put(b"{\"manifest\": \"test\"}").unwrap();

    let layer = LayerManifest {
        hash: "layer_hash_001".to_owned(),
        kind: LayerKind::Base,
        parent: None,
        object_refs: vec![obj_hash],
        read_only: true,
        tar_hash: String::new(),
    };
    let layer_content_hash = layer_store.put(&layer).unwrap();

    let meta = EnvMetadata {
        env_id: "env_abc123".into(),
        short_id: "env_abc123".into(),
        name: Some("test-env".to_owned()),
        state: EnvState::Built,
        base_layer: layer_content_hash.into(),
        dependency_layers: vec![],
        policy_layer: None,
        manifest_hash: manifest_hash.into(),
        ref_count: 1,
        created_at: "2025-01-01T00:00:00Z".to_owned(),
        updated_at: "2025-01-01T00:00:00Z".to_owned(),
        checksum: None,
    };
    meta_store.put(&meta).unwrap();

    (layout, "env_abc123".to_owned())
}

// --- Tests ---

#[test]
fn http_e2e_blob_roundtrip() {
    let (server, _dir) = start_server();
    let client = make_client(&server.url);

    // PUT
    client
        .put_blob(BlobKind::Object, "hash1", b"hello world")
        .unwrap();

    // GET
    let data = client.get_blob(BlobKind::Object, "hash1").unwrap();
    assert_eq!(data, b"hello world");

    // HEAD — exists
    assert!(client.has_blob(BlobKind::Object, "hash1").unwrap());

    // HEAD — missing
    assert!(!client.has_blob(BlobKind::Object, "missing").unwrap());

    // Multiple kinds
    client
        .put_blob(BlobKind::Layer, "l1", b"layer-data")
        .unwrap();
    client
        .put_blob(BlobKind::Metadata, "m1", b"meta-data")
        .unwrap();
    assert_eq!(
        client.get_blob(BlobKind::Layer, "l1").unwrap(),
        b"layer-data"
    );
    assert_eq!(
        client.get_blob(BlobKind::Metadata, "m1").unwrap(),
        b"meta-data"
    );
}

#[test]
fn http_e2e_push_pull_full_env() {
    let (server, _srv_dir) = start_server();
    let client = make_client(&server.url);

    // Set up source store with a mock environment
    let src_dir = tempfile::tempdir().unwrap();
    let (src_layout, env_id) = setup_local_env(src_dir.path());

    // Push to real server
    let push_result =
        karapace_remote::push_env(&src_layout, &env_id, &client, Some("test@latest")).unwrap();
    assert_eq!(push_result.objects_pushed, 2);
    assert_eq!(push_result.layers_pushed, 1);

    // Pull into a fresh store
    let dst_dir = tempfile::tempdir().unwrap();
    let dst_layout = StoreLayout::new(dst_dir.path());
    dst_layout.initialize().unwrap();

    let pull_result = karapace_remote::pull_env(&dst_layout, &env_id, &client).unwrap();
    assert_eq!(pull_result.objects_pulled, 2);
    assert_eq!(pull_result.layers_pulled, 1);

    // Verify metadata identical
    let src_meta = MetadataStore::new(src_layout).get(&env_id).unwrap();
    let dst_meta = MetadataStore::new(dst_layout.clone()).get(&env_id).unwrap();
    assert_eq!(src_meta.env_id, dst_meta.env_id);
    assert_eq!(src_meta.name, dst_meta.name);
    assert_eq!(src_meta.base_layer, dst_meta.base_layer);
    assert_eq!(src_meta.manifest_hash, dst_meta.manifest_hash);

    // Verify objects byte-for-byte identical
    let src_obj = ObjectStore::new(StoreLayout::new(src_dir.path()));
    let dst_obj = ObjectStore::new(dst_layout.clone());
    let src_data = src_obj.get(&src_meta.manifest_hash).unwrap();
    let dst_data = dst_obj.get(&dst_meta.manifest_hash).unwrap();
    assert_eq!(src_data, dst_data);

    // Verify layers identical
    let src_layer = LayerStore::new(StoreLayout::new(src_dir.path()))
        .get(&src_meta.base_layer)
        .unwrap();
    let dst_layer = LayerStore::new(dst_layout)
        .get(&dst_meta.base_layer)
        .unwrap();
    assert_eq!(src_layer.object_refs, dst_layer.object_refs);
    assert_eq!(src_layer.kind, dst_layer.kind);
}

#[test]
fn http_e2e_registry_roundtrip() {
    let (server, _dir) = start_server();
    let client = make_client(&server.url);

    // Set up and push an environment with a tag
    let src_dir = tempfile::tempdir().unwrap();
    let (src_layout, env_id) = setup_local_env(src_dir.path());
    karapace_remote::push_env(&src_layout, &env_id, &client, Some("myapp@latest")).unwrap();

    // Resolve the reference
    let resolved = karapace_remote::resolve_ref(&client, "myapp@latest").unwrap();
    assert_eq!(resolved, env_id);
}

#[test]
fn http_e2e_concurrent_4_clients() {
    let (server, _dir) = start_server();
    let url = server.url.clone();

    let handles: Vec<_> = (0..4)
        .map(|thread_idx| {
            let u = url.clone();
            std::thread::spawn(move || {
                let client = make_client(&u);
                for i in 0..10 {
                    let key = format!("t{thread_idx}_blob_{i}");
                    let data = format!("data-{thread_idx}-{i}");
                    client
                        .put_blob(BlobKind::Object, &key, data.as_bytes())
                        .unwrap();
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Verify all 40 blobs exist
    let client = make_client(&server.url);
    for thread_idx in 0..4 {
        for i in 0..10 {
            let key = format!("t{thread_idx}_blob_{i}");
            let expected = format!("data-{thread_idx}-{i}");
            let data = client.get_blob(BlobKind::Object, &key).unwrap();
            assert_eq!(data, expected.as_bytes(), "blob {key} data mismatch");
        }
    }
}

#[test]
fn http_e2e_server_restart_persistence() {
    let data_dir = tempfile::tempdir().unwrap();

    // Start server, push data
    {
        let server = TestServer::start(data_dir.path().to_path_buf());
        let client = make_client(&server.url);

        client
            .put_blob(BlobKind::Object, "persist1", b"data1")
            .unwrap();
        client
            .put_blob(BlobKind::Layer, "persist2", b"data2")
            .unwrap();
        client.put_registry(b"{\"entries\":{}}").unwrap();
        // server drops here — stops listening
    }

    // Start new server on same data_dir
    {
        let server2 = TestServer::start(data_dir.path().to_path_buf());
        let client2 = make_client(&server2.url);

        // All data must survive
        assert_eq!(
            client2.get_blob(BlobKind::Object, "persist1").unwrap(),
            b"data1"
        );
        assert_eq!(
            client2.get_blob(BlobKind::Layer, "persist2").unwrap(),
            b"data2"
        );
        assert_eq!(client2.get_registry().unwrap(), b"{\"entries\":{}}");
    }
}

#[test]
fn http_e2e_integrity_on_pull() {
    let (server, server_data) = start_server();
    let client = make_client(&server.url);

    // Push a real environment
    let src_dir = tempfile::tempdir().unwrap();
    let (src_layout, env_id) = setup_local_env(src_dir.path());
    karapace_remote::push_env(&src_layout, &env_id, &client, None).unwrap();

    // Tamper with an object on the server's filesystem directly
    let src_meta = MetadataStore::new(src_layout).get(&env_id).unwrap();
    let manifest_hash = src_meta.manifest_hash.to_string();
    let tampered_path = server_data
        .path()
        .join("blobs")
        .join("Object")
        .join(&manifest_hash);
    std::fs::write(&tampered_path, b"CORRUPTED DATA").unwrap();

    // Pull into a fresh store — must detect integrity failure
    let dst_dir = tempfile::tempdir().unwrap();
    let dst_layout = StoreLayout::new(dst_dir.path());
    dst_layout.initialize().unwrap();

    let result = karapace_remote::pull_env(&dst_layout, &env_id, &client);
    assert!(
        result.is_err(),
        "pull must fail when a blob has been tampered with"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("integrity") || err_msg.contains("Integrity"),
        "error must mention integrity failure, got: {err_msg}"
    );
}

#[test]
fn http_e2e_404_on_missing() {
    let (server, _dir) = start_server();
    let client = make_client(&server.url);

    let result = client.get_blob(BlobKind::Object, "nonexistent");
    assert!(result.is_err(), "GET missing blob must return error");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("404") || err_msg.contains("not found") || err_msg.contains("Not Found"),
        "error must indicate 404, got: {err_msg}"
    );
}
