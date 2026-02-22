//! Reference HTTP server library for the Karapace remote protocol v1.
//!
//! Implements the blob store and registry routes defined in `docs/protocol-v1.md`.
//! Storage is file-backed: blobs go into `{data_dir}/blobs/{kind}/{key}`,
//! the registry lives at `{data_dir}/registry.json`.
//!
//! The [`TestServer`] helper starts a server on a random port for integration testing.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tiny_http::{Header, Method, Response, Server, StatusCode};
use tracing::{debug, error, info};

/// In-memory + file-backed blob store.
pub struct Store {
    data_dir: PathBuf,
    /// Cache of registry data (kept in memory for atomic read-modify-write).
    registry: RwLock<Option<Vec<u8>>>,
}

impl Store {
    pub fn new(data_dir: PathBuf) -> Self {
        let reg_path = data_dir.join("registry.json");
        let registry = if reg_path.exists() {
            fs::read(&reg_path).ok()
        } else {
            None
        };

        Self {
            data_dir,
            registry: RwLock::new(registry),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn blob_dir(&self, kind: &str) -> PathBuf {
        self.data_dir.join("blobs").join(kind)
    }

    fn blob_path(&self, kind: &str, key: &str) -> PathBuf {
        self.blob_dir(kind).join(key)
    }

    pub fn put_blob(&self, kind: &str, key: &str, data: &[u8]) -> std::io::Result<()> {
        let dir = self.blob_dir(kind);
        fs::create_dir_all(&dir)?;
        let path = dir.join(key);
        fs::write(&path, data)?;
        Ok(())
    }

    pub fn get_blob(&self, kind: &str, key: &str) -> Option<Vec<u8>> {
        let path = self.blob_path(kind, key);
        fs::read(&path).ok()
    }

    pub fn has_blob(&self, kind: &str, key: &str) -> bool {
        self.blob_path(kind, key).exists()
    }

    pub fn list_blobs(&self, kind: &str) -> Vec<String> {
        let dir = self.blob_dir(kind);
        if !dir.exists() {
            return Vec::new();
        }
        fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter_map(|e| e.file_name().to_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn put_registry(&self, data: &[u8]) -> std::io::Result<()> {
        let reg_path = self.data_dir.join("registry.json");
        fs::create_dir_all(&self.data_dir)?;
        fs::write(&reg_path, data)?;
        let mut reg = self.registry.write().expect("registry lock poisoned");
        *reg = Some(data.to_vec());
        Ok(())
    }

    pub fn get_registry(&self) -> Option<Vec<u8>> {
        let reg = self.registry.read().expect("registry lock poisoned");
        reg.clone()
    }
}

/// Valid blob kinds per protocol spec.
pub fn is_valid_kind(kind: &str) -> bool {
    matches!(kind, "Object" | "Layer" | "Metadata")
}

/// Map the HttpBackend's plural lowercase path prefix to the server's internal kind name.
/// `/objects/` → "Object", `/layers/` → "Layer", `/metadata/` → "Metadata".
fn map_client_kind(prefix: &str) -> Option<&'static str> {
    match prefix {
        "objects" => Some("Object"),
        "layers" => Some("Layer"),
        "metadata" => Some("Metadata"),
        _ => None,
    }
}

/// Parse a URL path into (kind, key).
///
/// Accepts two URL schemes:
/// - Server-canonical: `/blobs/Object/abc123`
/// - Client (HttpBackend): `/objects/abc123`, `/layers/abc123`, `/metadata/abc123`
pub fn parse_blob_route(path: &str) -> Option<(&str, Option<&str>)> {
    // Try /blobs/{Kind}/... first
    if let Some(rest) = path.strip_prefix("/blobs/") {
        if let Some(idx) = rest.find('/') {
            let kind = &rest[..idx];
            let key = &rest[idx + 1..];
            if is_valid_kind(kind) && !key.is_empty() {
                return Some((kind, Some(key)));
            }
        } else if is_valid_kind(rest) {
            return Some((rest, None));
        }
    }
    None
}

/// Parse the client URL scheme: `/{plural_kind}/{key}` or `/{plural_kind}/`.
fn parse_client_route(path: &str) -> Option<(&'static str, Option<&str>)> {
    let path = path.strip_prefix('/')?;
    if let Some(idx) = path.find('/') {
        let prefix = &path[..idx];
        let rest = &path[idx + 1..];
        let kind = map_client_kind(prefix)?;
        if rest.is_empty() {
            Some((kind, None))
        } else {
            Some((kind, Some(rest)))
        }
    } else {
        let kind = map_client_kind(path)?;
        Some((kind, None))
    }
}

fn respond_err(req: tiny_http::Request, code: u16, msg: &str) {
    let _ = req.respond(Response::from_string(msg).with_status_code(StatusCode(code)));
}

fn respond_octet(req: tiny_http::Request, data: Vec<u8>) {
    let header =
        Header::from_bytes("Content-Type", "application/octet-stream").expect("valid header");
    let _ = req.respond(Response::from_data(data).with_header(header));
}

fn respond_json(req: tiny_http::Request, json: impl Into<Vec<u8>>) {
    let header = Header::from_bytes("Content-Type", "application/json").expect("valid header");
    let _ = req.respond(Response::from_data(json.into()).with_header(header));
}

fn read_body(req: &mut tiny_http::Request) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    if req.as_reader().read_to_end(&mut body).is_ok() {
        Some(body)
    } else {
        None
    }
}

fn handle_blob_keyed(
    store: &Store,
    mut req: tiny_http::Request,
    method: &Method,
    kind: &str,
    key: &str,
) {
    match *method {
        Method::Put => {
            let Some(body) = read_body(&mut req) else {
                respond_err(req, 500, "read error");
                return;
            };
            match store.put_blob(kind, key, &body) {
                Ok(()) => {
                    info!("PUT {kind}/{key}: {} bytes", body.len());
                    let _ = req.respond(Response::from_string("ok"));
                }
                Err(e) => {
                    error!("PUT {kind}/{key}: {e}");
                    respond_err(req, 500, &format!("write error: {e}"));
                }
            }
        }
        Method::Get => match store.get_blob(kind, key) {
            Some(data) => respond_octet(req, data),
            None => respond_err(req, 404, "not found"),
        },
        Method::Head => {
            let code = if store.has_blob(kind, key) { 200 } else { 404 };
            let _ = req.respond(Response::empty(code));
        }
        _ => respond_err(req, 405, "method not allowed"),
    }
}

fn handle_registry(store: &Store, mut req: tiny_http::Request, method: &Method) {
    match *method {
        Method::Put => {
            let Some(body) = read_body(&mut req) else {
                respond_err(req, 500, "read error");
                return;
            };
            match store.put_registry(&body) {
                Ok(()) => {
                    info!("PUT /registry: {} bytes", body.len());
                    let _ = req.respond(Response::from_string("ok"));
                }
                Err(e) => {
                    error!("PUT /registry: {e}");
                    respond_err(req, 500, &format!("write error: {e}"));
                }
            }
        }
        Method::Get => match store.get_registry() {
            Some(data) => respond_json(req, data),
            None => respond_err(req, 404, "not found"),
        },
        _ => respond_err(req, 405, "method not allowed"),
    }
}

/// Handle a single HTTP request, dispatching to the appropriate route handler.
pub fn handle_request(store: &Store, req: tiny_http::Request) {
    let method = req.method().clone();
    let url = req.url().to_owned();
    debug!("{method} {url}");

    // Try both URL schemes: /blobs/Kind/key (server canonical) and /kind_plural/key (client)
    let route = parse_blob_route(&url).or_else(|| parse_client_route(&url));
    if let Some(parsed) = route {
        match parsed {
            (kind, Some(key)) => handle_blob_keyed(store, req, &method, kind, key),
            (kind, None) if method == Method::Get => {
                let keys = store.list_blobs(kind);
                let json = serde_json::to_string(&keys).unwrap_or_else(|_| "[]".to_owned());
                respond_json(req, json.into_bytes());
            }
            _ => respond_err(req, 405, "method not allowed"),
        }
    } else if url == "/registry" {
        handle_registry(store, req, &method);
    } else if url == "/health" && method == Method::Get {
        let _ = req.respond(Response::from_string(r#"{"status":"ok"}"#));
    } else {
        respond_err(req, 404, "not found");
    }
}

/// Start the server loop, blocking the current thread.
pub fn run_server(store: &Arc<Store>, addr: &str) {
    let server = Server::http(addr).expect("failed to bind HTTP server");
    for request in server.incoming_requests() {
        handle_request(store, request);
    }
}

/// A test helper that starts a karapace-server on a random port in a background thread.
///
/// The server listens on `127.0.0.1:{port}` and stores data in the provided `data_dir`.
/// Drop the `TestServer` to stop the server (via `Server::unblock`).
pub struct TestServer {
    pub url: String,
    pub port: u16,
    pub data_dir: PathBuf,
    _server: Arc<Server>,
    _handle: std::thread::JoinHandle<()>,
}

impl TestServer {
    /// Start a test server with a temporary data directory.
    /// Binds to `127.0.0.1:0` (random port).
    pub fn start(data_dir: PathBuf) -> Self {
        fs::create_dir_all(&data_dir).expect("failed to create test data dir");
        let server =
            Arc::new(Server::http("127.0.0.1:0").expect("failed to bind test HTTP server"));
        let port = server.server_addr().to_ip().expect("not an IP addr").port();
        let url = format!("http://127.0.0.1:{port}");

        let store = Arc::new(Store::new(data_dir.clone()));
        let srv = Arc::clone(&server);
        let handle = std::thread::spawn(move || {
            for request in srv.incoming_requests() {
                handle_request(&store, request);
            }
        });

        Self {
            url,
            port,
            data_dir,
            _server: server,
            _handle: handle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_blob_route_object_with_key() {
        let (kind, key) = parse_blob_route("/blobs/Object/abc123").unwrap();
        assert_eq!(kind, "Object");
        assert_eq!(key, Some("abc123"));
    }

    #[test]
    fn parse_blob_route_layer_list() {
        let (kind, key) = parse_blob_route("/blobs/Layer").unwrap();
        assert_eq!(kind, "Layer");
        assert_eq!(key, None);
    }

    #[test]
    fn parse_blob_route_metadata_with_key() {
        let (kind, key) = parse_blob_route("/blobs/Metadata/env_abc").unwrap();
        assert_eq!(kind, "Metadata");
        assert_eq!(key, Some("env_abc"));
    }

    #[test]
    fn parse_blob_route_invalid_kind() {
        assert!(parse_blob_route("/blobs/Invalid/key").is_none());
    }

    #[test]
    fn parse_blob_route_missing_prefix() {
        assert!(parse_blob_route("/other/Object/key").is_none());
    }

    #[test]
    fn store_blob_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());

        store.put_blob("Object", "hash1", b"content").unwrap();
        assert!(store.has_blob("Object", "hash1"));
        assert_eq!(store.get_blob("Object", "hash1"), Some(b"content".to_vec()));
        assert!(!store.has_blob("Object", "missing"));
    }

    #[test]
    fn store_list_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());

        store.put_blob("Layer", "l1", b"a").unwrap();
        store.put_blob("Layer", "l2", b"b").unwrap();
        let mut keys = store.list_blobs("Layer");
        keys.sort();
        assert_eq!(keys, vec!["l1", "l2"]);
    }

    #[test]
    fn store_registry_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());

        assert!(store.get_registry().is_none());
        store.put_registry(b"{\"entries\":{}}").unwrap();
        assert_eq!(store.get_registry(), Some(b"{\"entries\":{}}".to_vec()));
    }

    #[test]
    fn store_registry_persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::new(dir.path().to_path_buf());
            store.put_registry(b"reg_data").unwrap();
        }
        let store2 = Store::new(dir.path().to_path_buf());
        assert_eq!(store2.get_registry(), Some(b"reg_data".to_vec()));
    }
}
