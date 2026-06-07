//! Mock server service — start/stop a mock HTTP server.

use crate::engine::{MockMatcher, MockRegistry};
use std::sync::Arc;

/// Input configuration for starting the mock server.
#[derive(Debug, Clone)]
pub struct ServeInput {
    /// Port to listen on (0 = random available port)
    pub port: u16,
    /// Host to bind to
    pub host: String,
    /// Directory containing mock collection files (YAML/JSON/HAR)
    pub mocks_dir: Option<String>,
    /// Specific mock file to load
    pub mock_file: Option<String>,
    /// Watch for file changes and hot-reload
    pub watch: bool,
    /// Enable CORS headers
    pub cors: bool,
    /// Enable template render and mock list endpoints
    pub enable_management_endpoints: bool,
    /// Log mock match details for every request
    pub log_matches: bool,
    /// Enable verbose request logging
    pub verbose: bool,
}

impl Default for ServeInput {
    fn default() -> Self {
        Self {
            port: 3006,
            host: "127.0.0.1".into(),
            mocks_dir: None,
            mock_file: None,
            watch: false,
            cors: false,
            enable_management_endpoints: false,
            log_matches: false,
            verbose: false,
        }
    }
}

/// Result from loading mocks into the server.
#[derive(Debug, Clone)]
pub struct LoadResult {
    /// Number of mocks loaded
    pub count: usize,
    /// Source path (directory or file)
    pub source: String,
}

/// Handle to a running mock server.
pub struct ServeHandle {
    /// The mock registry (shared, can be used to add/remove mocks at runtime)
    pub registry: Arc<MockRegistry>,
    /// The mock matcher
    pub matcher: MockMatcher,
    /// Actual port the server is listening on
    pub port: u16,
    /// Full URL the server is listening on
    pub url: String,
    /// Shutdown signal sender
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ServeHandle {
    /// Stop the server gracefully.
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Check if the server is still running.
    pub fn is_running(&self) -> bool {
        self.shutdown_tx.is_some()
    }
}

impl Drop for ServeHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Load mocks from directory and/or file into a registry.
///
/// Returns the registry, list of load results, and total count.
pub async fn load_mocks(
    registry: &MockRegistry,
    mocks_dir: Option<&str>,
    mock_file: Option<&str>,
) -> Result<Vec<LoadResult>, crate::MockpitError> {
    let mut results = Vec::new();

    // Load from directory
    if let Some(dir) = mocks_dir {
        let count = registry
            .load_from_directory(dir)
            .await
            .map_err(|e| crate::mp_err!(e))?;
        results.push(LoadResult {
            count,
            source: dir.to_string(),
        });
    } else if mock_file.is_none() {
        // Default directory if neither --mocks nor --mock-file given
        let default_dir =
            std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());
        let count = registry
            .load_from_directory(&default_dir)
            .await
            .map_err(|e| crate::mp_err!(e))?;
        results.push(LoadResult {
            count,
            source: default_dir,
        });
    }

    // Load specific file
    if let Some(file) = mock_file {
        let path = std::path::Path::new(file);
        let count = registry
            .load_collection_file(path)
            .await
            .map_err(|e| crate::mp_err!(e))?;
        results.push(LoadResult {
            count,
            source: file.to_string(),
        });
    }

    Ok(results)
}

/// Start a mock server with the given configuration.
///
/// Returns a [`ServeHandle`] that can be used to control the server.
pub async fn start(input: ServeInput) -> Result<ServeHandle, crate::MockpitError> {
    let registry = Arc::new(MockRegistry::new());

    // Load mocks
    load_mocks(
        &registry,
        input.mocks_dir.as_deref(),
        input.mock_file.as_deref(),
    )
    .await?;

    // Set up hot reload if enabled
    if input.watch {
        let collections_dir = input.mocks_dir.clone().unwrap_or_else(|| {
            std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
        });
        init_hot_reload(&collections_dir, Arc::clone(&registry))?;
    }

    // Create matcher
    let matcher = MockMatcher::new((*registry).clone());

    // Bind listener
    let addr = format!("{}:{}", input.host, input.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let actual_port = listener.local_addr()?.port();
    let url = format!("http://{}:{}", input.host, actual_port);

    // Build router
    let app = build_router(
        Arc::clone(&registry),
        matcher.clone(),
        input.cors,
        input.verbose,
    );

    // Shutdown channel
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Spawn server
    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });

    Ok(ServeHandle {
        registry,
        matcher,
        port: actual_port,
        url,
        shutdown_tx: Some(shutdown_tx),
    })
}

/// Build the axum router for the mock server.
fn build_router(
    registry: Arc<MockRegistry>,
    matcher: MockMatcher,
    cors: bool,
    verbose: bool,
) -> axum::Router {
    use axum::routing::any;
    use tower_http::cors::{Any, CorsLayer};
    use tower_http::trace::TraceLayer;

    let state = Arc::new(ServerState { matcher, registry });

    let mut app = axum::Router::new().fallback(any(mock_handler));

    if cors {
        app = app.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    }

    if verbose {
        app = app.layer(TraceLayer::new_for_http());
    }

    app.with_state(state)
}

#[cfg(feature = "server")]
fn init_hot_reload(
    collections_dir: &str,
    registry: Arc<MockRegistry>,
) -> Result<(), crate::MockpitError> {
    use crate::server::hot_reload::{HotReloadConfig, HotReloadManager};
    use std::path::PathBuf;

    let collections_path = PathBuf::from(collections_dir);
    if !collections_path.exists() {
        return Ok(());
    }

    let hot_reload_config = HotReloadConfig { debounce_ms: 300 };
    let mut manager =
        HotReloadManager::new(registry, vec![collections_path], hot_reload_config)?;
    manager.start_watching()?;
    manager.spawn();
    Ok(())
}

#[cfg(not(feature = "server"))]
fn init_hot_reload(
    _collections_dir: &str,
    _registry: Arc<MockRegistry>,
) -> Result<(), crate::MockpitError> {
    Ok(()) // Hot reload requires the "server" feature
}

// -- Internal server implementation --

#[derive(Clone)]
struct ServerState {
    matcher: MockMatcher,
    #[allow(dead_code)]
    registry: Arc<MockRegistry>,
}

/// Canonical mock-response builder shared by every mock server (this service,
/// the NAPI `MockpitServer.listen()` path, and the CLI). Matches the request,
/// generates the dynamic response, and builds an axum response with the
/// `X-Mock-Id` header. This is the single source of truth — do not reimplement.
pub async fn respond(
    matcher: &MockMatcher,
    method: &http::Method,
    path: &str,
    query: Option<&str>,
    headers: &http::HeaderMap,
    body: Option<&[u8]>,
) -> axum::response::Response {
    use crate::engine::types::ResponseGeneratorExt;
    use axum::body::Body;
    use axum::response::IntoResponse;
    use http::header;

    let Some(mock_match) = matcher.find_match(method, path, query, headers, body) else {
        let body = serde_json::json!({
            "error": "No matching mock found",
            "method": method.as_str(),
            "path": path,
            "query": query
        });
        return (
            http::StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response();
    };

    let mock_def = &mock_match.mock;
    let dynamic = mock_def
        .response
        .generate_dynamic(
            method.as_str(),
            path,
            query,
            headers,
            body,
            mock_match.captures,
            mock_def.vars.as_ref(),
        )
        .await;

    match dynamic {
        Ok(resp) => {
            let status = resp.status.unwrap_or(mock_def.response.status);
            let mut response = http::Response::builder().status(status);
            for (key, value) in &mock_def.response.headers {
                response = response.header(key.as_str(), value.as_str());
            }
            if let Some(dyn_headers) = &resp.headers {
                for (key, value) in dyn_headers {
                    response = response.header(key.as_str(), value.as_str());
                }
            }
            response
                .header("X-Mock-Id", mock_def.id.as_str())
                .body(Body::from(resp.body))
                .unwrap_or_else(|_| {
                    (http::StatusCode::INTERNAL_SERVER_ERROR, "Response build error")
                        .into_response()
                })
        }
        Err(e) => {
            let body = serde_json::json!({
                "error": "Mock response generation failed",
                "mock_id": mock_def.id.as_str(),
                "details": e.to_string()
            });
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                body.to_string(),
            )
                .into_response()
        }
    }
}

async fn mock_handler(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .ok();
    let body_ref = body_bytes
        .as_ref()
        .filter(|b| !b.is_empty())
        .map(|b| b.as_ref());

    respond(
        &state.matcher,
        &method,
        uri.path(),
        uri.query(),
        &headers,
        body_ref,
    )
    .await
}
