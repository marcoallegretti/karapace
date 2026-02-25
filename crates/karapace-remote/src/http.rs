use crate::{BlobKind, RemoteBackend, RemoteConfig, RemoteError};
use std::io::Read;

/// HTTP-based remote store backend.
///
/// Expects a simple REST API:
/// - `PUT  /objects/<key>`   — upload object blob
/// - `GET  /objects/<key>`   — download object blob
/// - `HEAD /objects/<key>`   — check existence
/// - `GET  /objects/`        — list objects (JSON array of strings)
/// - Same pattern for `/layers/` and `/metadata/`
/// - `PUT  /registry`        — upload registry index
/// - `GET  /registry`        — download registry index
pub struct HttpBackend {
    config: RemoteConfig,
    agent: ureq::Agent,
}

impl HttpBackend {
    pub fn new(config: RemoteConfig) -> Self {
        let agent = ureq::Agent::new_with_defaults();
        Self { config, agent }
    }

    fn kind_path(kind: BlobKind) -> &'static str {
        match kind {
            BlobKind::Object => "objects",
            BlobKind::Layer => "layers",
            BlobKind::Metadata => "metadata",
        }
    }

    fn url(&self, kind: BlobKind, key: &str) -> String {
        format!("{}/{}/{}", self.config.url, Self::kind_path(kind), key)
    }

    fn do_put(&self, url: &str, content_type: &str, data: &[u8]) -> Result<(), RemoteError> {
        let mut req = self
            .agent
            .put(url)
            .header("Content-Type", content_type)
            .header("X-Karapace-Protocol", &crate::PROTOCOL_VERSION.to_string());
        if let Some(ref token) = self.config.auth_token {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        req.send(data as &[u8])
            .map_err(|e| RemoteError::Http(e.to_string()))?;
        Ok(())
    }

    fn do_get(&self, url: &str) -> Result<Vec<u8>, RemoteError> {
        let mut req = self
            .agent
            .get(url)
            .header("X-Karapace-Protocol", &crate::PROTOCOL_VERSION.to_string());
        if let Some(ref token) = self.config.auth_token {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(404)) => {
                return Err(RemoteError::NotFound(url.to_owned()));
            }
            Err(ureq::Error::StatusCode(code)) => {
                return Err(RemoteError::Http(format!("HTTP {code} for {url}")));
            }
            Err(e) => {
                return Err(RemoteError::Http(e.to_string()));
            }
        };

        let status = resp.status();
        let code = status.as_u16();
        if code == 404 {
            return Err(RemoteError::NotFound(url.to_owned()));
        }
        if code >= 400 {
            return Err(RemoteError::Http(format!("HTTP {code} for {url}")));
        }

        let mut reader = resp.into_body().into_reader();
        let mut body = Vec::new();
        reader
            .read_to_end(&mut body)
            .map_err(|e| RemoteError::Http(e.to_string()))?;
        Ok(body)
    }

    fn do_head(&self, url: &str) -> Result<u16, RemoteError> {
        let mut req = self
            .agent
            .head(url)
            .header("X-Karapace-Protocol", &crate::PROTOCOL_VERSION.to_string());
        if let Some(ref token) = self.config.auth_token {
            req = req.header("Authorization", &format!("Bearer {token}"));
        }
        match req.call() {
            Ok(resp) => Ok(resp.status().into()),
            Err(ureq::Error::StatusCode(code)) => Ok(code),
            Err(e) => Err(RemoteError::Http(e.to_string())),
        }
    }
}

impl RemoteBackend for HttpBackend {
    fn put_blob(&self, kind: BlobKind, key: &str, data: &[u8]) -> Result<(), RemoteError> {
        let url = self.url(kind, key);
        tracing::debug!("PUT {url} ({} bytes)", data.len());
        self.do_put(&url, "application/octet-stream", data)
    }

    fn get_blob(&self, kind: BlobKind, key: &str) -> Result<Vec<u8>, RemoteError> {
        let url = self.url(kind, key);
        tracing::debug!("GET {url}");
        self.do_get(&url)
    }

    fn has_blob(&self, kind: BlobKind, key: &str) -> Result<bool, RemoteError> {
        let url = self.url(kind, key);
        tracing::debug!("HEAD {url}");
        match self.do_head(&url)? {
            200 => Ok(true),
            404 => Ok(false),
            code => Err(RemoteError::Http(format!("HTTP {code} for HEAD {url}"))),
        }
    }

    fn list_blobs(&self, kind: BlobKind) -> Result<Vec<String>, RemoteError> {
        let url = format!("{}/{}/", self.config.url, Self::kind_path(kind));
        tracing::debug!("GET {url}");
        let body = self.do_get(&url)?;
        let body_str = String::from_utf8(body).map_err(|e| RemoteError::Http(e.to_string()))?;
        let keys: Vec<String> = serde_json::from_str(&body_str)
            .map_err(|e| RemoteError::Serialization(e.to_string()))?;
        Ok(keys)
    }

    fn put_registry(&self, data: &[u8]) -> Result<(), RemoteError> {
        let url = format!("{}/registry", self.config.url);
        tracing::debug!("PUT {url} ({} bytes)", data.len());
        self.do_put(&url, "application/json", data)
    }

    fn get_registry(&self) -> Result<Vec<u8>, RemoteError> {
        let url = format!("{}/registry", self.config.url);
        tracing::debug!("GET {url}");
        self.do_get(&url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A captured HTTP request for header inspection.
    #[derive(Debug, Clone)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: HashMap<String, String>,
    }

    struct MockServer {
        addr: String,
        _handle: std::thread::JoinHandle<()>,
        requests: Arc<Mutex<Vec<CapturedRequest>>>,
    }

    impl MockServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = format!("http://{}", listener.local_addr().unwrap());
            let store: Arc<Mutex<HashMap<String, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
            let requests: Arc<Mutex<Vec<CapturedRequest>>> = Arc::new(Mutex::new(Vec::new()));

            let store_clone = Arc::clone(&store);
            let requests_clone = Arc::clone(&requests);
            let handle = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let store = Arc::clone(&store_clone);
                    let reqs = Arc::clone(&requests_clone);

                    std::thread::spawn(move || {
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut request_line = String::new();
                        if reader.read_line(&mut request_line).is_err() {
                            return;
                        }
                        let parts: Vec<&str> = request_line.trim().splitn(3, ' ').collect();
                        if parts.len() < 2 {
                            return;
                        }
                        let method = parts[0].to_owned();
                        let path = parts[1].to_owned();

                        let mut content_length: usize = 0;
                        let mut headers = HashMap::new();
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
                                break;
                            }
                            if let Some((k, v)) = line.trim().split_once(": ") {
                                headers.insert(k.to_lowercase(), v.to_owned());
                            }
                            let lower = line.to_lowercase();
                            if let Some(val) = lower.strip_prefix("content-length: ") {
                                content_length = val.trim().parse().unwrap_or(0);
                            }
                        }

                        reqs.lock().unwrap().push(CapturedRequest {
                            method: method.clone(),
                            path: path.clone(),
                            headers,
                        });

                        let mut body = vec![0u8; content_length];
                        if content_length > 0 {
                            let _ = reader.read_exact(&mut body);
                        }

                        let mut data = store.lock().unwrap();
                        let response = match method.as_str() {
                            "PUT" => {
                                data.insert(path.clone(), body);
                                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                    .to_owned()
                            }
                            "GET" => {
                                if let Some(val) = data.get(&path) {
                                    format!(
                                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                        val.len()
                                    )
                                } else {
                                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                        .to_owned()
                                }
                            }
                            "HEAD" => {
                                if data.contains_key(&path) {
                                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                        .to_owned()
                                } else {
                                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                        .to_owned()
                                }
                            }
                            _ => "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                .to_owned(),
                        };

                        let _ = stream.write_all(response.as_bytes());
                        if method == "GET" {
                            if let Some(val) = data.get(&path) {
                                let _ = stream.write_all(val);
                            }
                        }
                        let _ = stream.flush();
                    });
                }
            });

            MockServer {
                addr,
                _handle: handle,
                requests,
            }
        }

        fn captured_requests(&self) -> Vec<CapturedRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn test_backend(url: &str) -> HttpBackend {
        HttpBackend::new(RemoteConfig {
            url: url.to_owned(),
            auth_token: None,
        })
    }

    fn test_backend_with_auth(url: &str, token: &str) -> HttpBackend {
        HttpBackend::new(RemoteConfig {
            url: url.to_owned(),
            auth_token: Some(token.to_owned()),
        })
    }

    #[test]
    fn http_put_and_get_blob() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);
        backend
            .put_blob(BlobKind::Object, "hash123", b"test data")
            .unwrap();
        let data = backend.get_blob(BlobKind::Object, "hash123").unwrap();
        assert_eq!(data, b"test data");
    }

    #[test]
    fn http_has_blob_true_and_false() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);
        assert!(!backend.has_blob(BlobKind::Object, "missing").unwrap());
        backend
            .put_blob(BlobKind::Object, "exists", b"data")
            .unwrap();
        assert!(backend.has_blob(BlobKind::Object, "exists").unwrap());
    }

    #[test]
    fn http_get_nonexistent_fails() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);
        let result = backend.get_blob(BlobKind::Object, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn http_put_and_get_registry() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);
        let registry_data = b"{\"entries\":{}}";
        backend.put_registry(registry_data).unwrap();
        let data = backend.get_registry().unwrap();
        assert_eq!(data, registry_data);
    }

    #[test]
    fn http_connection_refused_returns_error() {
        let backend = test_backend("http://127.0.0.1:1");
        let result = backend.put_blob(BlobKind::Object, "key", b"data");
        assert!(result.is_err());
    }

    #[test]
    fn http_multiple_blob_kinds() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);

        backend
            .put_blob(BlobKind::Object, "obj1", b"object-data")
            .unwrap();
        backend
            .put_blob(BlobKind::Layer, "layer1", b"layer-data")
            .unwrap();
        backend
            .put_blob(BlobKind::Metadata, "meta1", b"meta-data")
            .unwrap();

        assert_eq!(
            backend.get_blob(BlobKind::Object, "obj1").unwrap(),
            b"object-data"
        );
        assert_eq!(
            backend.get_blob(BlobKind::Layer, "layer1").unwrap(),
            b"layer-data"
        );
        assert_eq!(
            backend.get_blob(BlobKind::Metadata, "meta1").unwrap(),
            b"meta-data"
        );
    }

    // --- M4: Protocol version header tests ---

    #[test]
    fn http_requests_include_protocol_header() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);

        // PUT sends the header
        backend.put_blob(BlobKind::Object, "h1", b"data").unwrap();
        // GET sends the header
        let _ = backend.get_blob(BlobKind::Object, "h1");
        // HEAD sends the header
        let _ = backend.has_blob(BlobKind::Object, "h1");

        // Allow the mock server threads to finish
        std::thread::sleep(std::time::Duration::from_millis(50));

        let reqs = server.captured_requests();
        assert!(
            reqs.len() >= 3,
            "expected at least 3 requests, got {}",
            reqs.len()
        );
        for req in &reqs {
            let proto = req.headers.get("x-karapace-protocol");
            assert_eq!(
                proto,
                Some(&"1".to_owned()),
                "{} {} missing X-Karapace-Protocol header",
                req.method,
                req.path
            );
        }
    }

    #[test]
    fn http_protocol_version_constant_is_1() {
        assert_eq!(crate::PROTOCOL_VERSION, 1);
    }

    #[test]
    fn http_auth_token_sent_as_bearer_header() {
        let server = MockServer::start();
        let backend = test_backend_with_auth(&server.addr, "secret-token-42");

        backend
            .put_blob(BlobKind::Object, "auth1", b"data")
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let reqs = server.captured_requests();
        assert!(!reqs.is_empty());
        let auth = reqs[0].headers.get("authorization");
        assert_eq!(
            auth,
            Some(&"Bearer secret-token-42".to_owned()),
            "PUT must include Authorization: Bearer header"
        );
    }

    #[test]
    fn http_no_auth_header_without_token() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);

        backend
            .put_blob(BlobKind::Object, "noauth", b"data")
            .unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let reqs = server.captured_requests();
        assert!(!reqs.is_empty());
        assert!(
            !reqs[0].headers.contains_key("authorization"),
            "no auth token configured — Authorization header must not be sent"
        );
    }

    // --- M7.2: Remote HTTP coverage ---

    #[test]
    fn http_list_blobs_returns_keys() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);

        // Populate the mock store with a list response
        backend.put_blob(BlobKind::Object, "a", b"data-a").unwrap();
        backend.put_blob(BlobKind::Object, "b", b"data-b").unwrap();
        backend.put_blob(BlobKind::Object, "c", b"data-c").unwrap();

        // Store the list response at the list endpoint
        let list_url = format!("{}/objects/", server.addr);
        let list_body = serde_json::to_vec(&["a", "b", "c"]).unwrap();
        // Manually insert the list response via a PUT to the list path
        backend
            .do_put(&list_url, "application/json", &list_body)
            .unwrap();

        let keys = backend.list_blobs(BlobKind::Object).unwrap();
        assert_eq!(keys, vec!["a", "b", "c"]);
    }

    #[test]
    fn http_large_blob_roundtrip() {
        let server = MockServer::start();
        let backend = test_backend(&server.addr);

        // Create a 1MB blob
        let large_data: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();
        backend
            .put_blob(BlobKind::Object, "large", &large_data)
            .unwrap();
        let retrieved = backend.get_blob(BlobKind::Object, "large").unwrap();
        assert_eq!(retrieved.len(), large_data.len());
        assert_eq!(retrieved, large_data);
    }
}
