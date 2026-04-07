//! Standalone mock server
//!
//! A lightweight HTTP server that serves mock responses without the full proxy overhead.
//! Reuses the existing mock infrastructure including hot reload.

use std::sync::Arc;

use super::ui;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{Html, Response};
use axum::routing::{any, get, post};
use mockpit::engine::{MockMatcher, MockRegistry, RequestContext, ResponseGeneratorExt};
use rustc_hash::FxHashMap;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};

use anyhow::Context;

/// Configuration for the mock server
pub struct MockServerConfig {
    pub port: u16,
    pub host: String,
    pub mocks_dir: Option<String>,
    pub mock_file: Option<String>,
    pub watch: bool,
    pub cors: bool,
    pub enable_render_endpoint: bool,
    pub log_matches: bool,
    pub verbose: bool,
    pub open_browser: bool,
}

/// Shared state for the mock server
#[derive(Clone)]
struct MockServerState {
    matcher: MockMatcher,
    registry: Arc<MockRegistry>,
    verbose: bool,
    log_matches: bool,
    enable_render_endpoint: bool,
}

/// Start a standalone mock server
pub async fn serve_mock_server(config: MockServerConfig) -> anyhow::Result<()> {
    let MockServerConfig {
        port,
        host,
        mocks_dir,
        mock_file,
        watch,
        cors,
        enable_render_endpoint,
        log_matches,
        verbose,
        open_browser,
    } = config;

    println!("{}", ui::header("Mock Server"));
    println!();

    let registry = Arc::new(MockRegistry::new());
    let mut total_count = 0usize;

    // Load mocks from directory if provided
    if let Some(ref dir) = mocks_dir {
        let spinner = ui::spinner(&format!("Loading mocks from {}...", ui::path(dir)));
        let count = registry
            .load_from_directory(dir)
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .context("Failed to load mocks from directory")?;
        spinner.finish_and_clear();
        total_count += count;
        println!(
            "{}",
            ui::success(&format!(
                "Loaded {} mock(s) from {}",
                ui::number(count),
                ui::path(dir)
            ))
        );
    } else if mock_file.is_none() {
        // Default directory if neither --mocks nor --mock-file given
        let default_dir =
            std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());
        let spinner = ui::spinner(&format!("Loading mocks from {}...", ui::path(&default_dir)));
        let count = registry
            .load_from_directory(&default_dir)
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .context("Failed to load mocks")?;
        spinner.finish_and_clear();
        total_count += count;
        println!(
            "{}",
            ui::success(&format!(
                "Loaded {} mock(s) from {}",
                ui::number(count),
                ui::path(&default_dir)
            ))
        );
    }

    // Load specific mock file if provided
    if let Some(ref file) = mock_file {
        let path = std::path::Path::new(file);
        let spinner = ui::spinner(&format!("Loading mocks from {}...", ui::path(file)));
        let count = registry
            .load_collection_file(path)
            .await
            .map_err(|e| anyhow::anyhow!(e))
            .context("Failed to load mock file")?;
        spinner.finish_and_clear();
        total_count += count;
        println!(
            "{}",
            ui::success(&format!(
                "Loaded {} mock(s) from {}",
                ui::number(count),
                ui::path(file)
            ))
        );
    }

    println!(
        "{}",
        ui::success(&format!(
            "Total: {} mock definition(s)",
            ui::number(total_count)
        ))
    );

    let collections_dir = mocks_dir.unwrap_or_else(|| {
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
    });

    // Set up hot reload if enabled
    if watch {
        init_hot_reload(&collections_dir, Arc::clone(&registry))?;
        println!(
            "{}",
            ui::info(&format!(
                "Watching {} for changes",
                ui::path(&collections_dir)
            ))
        );
    }

    // Create matcher
    let matcher = MockMatcher::new((*registry).clone());

    let state = Arc::new(MockServerState {
        matcher,
        registry: Arc::clone(&registry),
        verbose,
        log_matches,
        enable_render_endpoint,
    });

    println!();
    println!("{}", ui::kv("Address", &format!("http://{host}:{port}")));
    println!("{}", ui::kv("Mocks Directory", &collections_dir));
    println!();

    println!("{}", ui::emphasis("Endpoints:"));
    println!(
        "{}",
        ui::list_item("ANY  /*path                 - Mock matching (all methods/paths)")
    );
    println!(
        "{}",
        ui::list_item("GET  /__mock/status         - Server status and info")
    );
    if enable_render_endpoint {
        println!(
            "{}",
            ui::list_item("POST /__mock/render         - Render template with context")
        );
        println!(
            "{}",
            ui::list_item("GET  /__mock/list           - List all loaded mocks")
        );
    }
    println!();

    // Build router - use /__mock/ prefix for management endpoints to avoid conflicts
    let mut app = Router::new().route("/__mock/status", get(index_handler));

    // Add render endpoint if enabled
    if enable_render_endpoint {
        app = app
            .route("/__mock/render", post(render_handler))
            .route("/__mock/list", get(list_mocks_handler));
    }

    // Catch-all handler for mock matching
    app = app.fallback(any(mock_handler));

    // Add CORS if enabled (before state)
    if cors {
        app = app.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );
        println!("{}", ui::info("CORS enabled"));
    }

    // Add logging if verbose (before state)
    if verbose {
        app = app.layer(TraceLayer::new_for_http());
        println!("{}", ui::info("Verbose logging enabled"));
    }

    if log_matches {
        println!("{}", ui::info("Match logging enabled"));
    }

    // Add state LAST to convert Router<S> to Router<()>
    let app = app.with_state(state);

    println!();
    println!("{}", ui::success("Server starting..."));
    println!("{}", ui::dim("Press Ctrl+C to stop"));

    // Open browser if requested
    if open_browser {
        let url = format!("http://{host}:{port}");
        let _ = open::that(&url);
    }

    // Start server using existing utilities
    let addr: std::net::SocketAddr = format!("{host}:{port}")
        .parse()
        .context("Invalid address")?;

    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Use the existing serve utility which handles Router<()> properly
    let server_handle =
        mockpit::server::serve_with_graceful_shutdown(listener, app, shutdown_signal())?;

    // Wait for server to complete
    if let Err(e) = server_handle.await {
        eprintln!("{}", ui::error(&format!("Server task failed: {e:?}")));
    }

    println!();
    println!("{}", ui::success("Server stopped"));

    Ok(())
}

/// Set up hot reload using existing infrastructure
fn init_hot_reload(collections_dir: &str, registry: Arc<MockRegistry>) -> anyhow::Result<()> {
    use mockpit::server::hot_reload::{HotReloadConfig, HotReloadManager};
    use std::path::PathBuf;

    let collections_path = PathBuf::from(collections_dir);
    anyhow::ensure!(
        collections_path.exists(),
        "Mock collections directory does not exist: {collections_dir}"
    );

    let hot_reload_config = HotReloadConfig { debounce_ms: 300 };

    let mut manager = HotReloadManager::new(registry, vec![collections_path], hot_reload_config)
        .context("Failed to create hot reload manager")?;
    manager
        .start_watching()
        .context("Failed to start hot reload watcher")?;
    // Spawn the hot reload loop in the background
    manager.spawn();
    Ok(())
}

/// Graceful shutdown signal
async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        eprintln!("Failed to install Ctrl+C handler: {e}");
    }
}

/// Index page handler
async fn index_handler(State(state): State<Arc<MockServerState>>) -> Html<String> {
    let mock_count = state.registry.len();

    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
  <title>Mock Server</title>
  <style>
    body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 0 auto; padding: 2rem; }}
    h1 {{ color: #333; }}
    h2 {{ color: #666; margin-top: 2rem; }}
    code {{ background: #f4f4f4; padding: 0.2rem 0.4rem; border-radius: 3px; }}
    pre {{ background: #f4f4f4; padding: 1rem; border-radius: 5px; overflow-x: auto; }}
    .status {{ padding: 0.5rem 1rem; background: #e8f5e9; border-radius: 5px; margin: 1rem 0; }}
    .endpoint {{ margin: 1rem 0; padding: 1rem; border: 1px solid #ddd; border-radius: 5px; }}
    .method {{ font-weight: bold; color: #0066cc; }}
  </style>
</head>
<body>
  <h1>Mock Server</h1>
  <div class="status">
    <strong>Status:</strong> Running | <strong>Loaded Mocks:</strong> {mock_count}
  </div>

  <h2>How it works</h2>
  <p>This server matches incoming requests against loaded mock definitions and returns mock responses.</p>
  <p>All requests (except <code>/__mock/*</code>) are matched against your mocks.</p>

  <h2>Endpoints</h2>

  <div class="endpoint">
    <p><span class="method">ANY</span> <code>/*</code></p>
    <p>All requests are matched against loaded mocks. Returns mock response if matched, 404 if not.</p>
    <p>Response includes <code>X-Mock-Id</code> header indicating which mock was used.</p>
  </div>
  {render_endpoint}
</body>
</html>"#,
        mock_count = mock_count,
        render_endpoint = if state.enable_render_endpoint {
            r#"
  <div class="endpoint">
    <p><span class="method">POST</span> <code>/__mock/render</code></p>
    <p>Render a template with fake data and optional context.</p>
    <pre>curl -X POST http://localhost:PORT/__mock/render \
  -H "Content-Type: application/json" \
  -d '{"template": "{\"name\": \"{{ fake_name() }}\"}"}'</pre>
  </div>

  <div class="endpoint">
    <p><span class="method">GET</span> <code>/__mock/list</code></p>
    <p>List all loaded mock definitions.</p>
  </div>"#
        } else {
            ""
        }
    );

    Html(html)
}

/// Main mock matching handler
#[allow(clippy::expect_used)]
async fn mock_handler(State(state): State<Arc<MockServerState>>, req: Request) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path();
    let query = uri.query();
    let headers = req.headers().clone();

    // Read body for non-safe methods
    let body_bytes = if method.is_safe() {
        None
    } else {
        match axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024).await {
            Ok(bytes) => Some(bytes.to_vec()),
            Err(_) => None,
        }
    };

    let request_start = std::time::Instant::now();

    // Log request if verbose
    if state.verbose {
        debug!(
          method = %method,
          path = %path,
          query = ?query,
          "Incoming request"
        );
    }

    // Find matching mock
    let mock_match =
        state
            .matcher
            .find_match(&method, path, query, &headers, body_bytes.as_deref());

    if let Some(mock_match) = mock_match {
        let mock_def = &mock_match.mock;
        let captures = mock_match.captures;

        if state.verbose {
            debug!(
              mock_id = %mock_def.id,
              "Request matched mock"
            );
        }

        if state.log_matches {
            let elapsed = request_start.elapsed();
            info!(
              mock_id = %mock_def.id,
              method = %method,
              path = %path,
              status = %mock_def.response.status.as_u16(),
              captures = ?captures,
              elapsed_ms = elapsed.as_secs_f64() * 1000.0,
              "Mock matched"
            );
        }

        // Generate response
        let response_result = generate_mock_response(
            mock_def,
            &method,
            path,
            query,
            &headers,
            body_bytes.as_deref(),
            captures,
        )
        .await;

        match response_result {
            Ok(response) => response,
            Err(e) => {
                let error_body = serde_json::json!({
                  "error": "Mock Response Generation Failed",
                  "mock_id": mock_def.id,
                  "details": format!("{e}")
                });

                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("X-Mock-Id", mock_def.id.as_str())
                    .header("X-Mock-Error", "true")
                    .body(Body::from(error_body.to_string()))
                    .expect("building error response should not fail")
            }
        }
    } else {
        if state.verbose || state.log_matches {
            let elapsed = request_start.elapsed();
            warn!(
              method = %method,
              path = %path,
              query = ?query,
              elapsed_ms = elapsed.as_secs_f64() * 1000.0,
              "No matching mock found"
            );
        }

        let error_body = serde_json::json!({
          "error": "No matching mock found",
          "method": method.as_str(),
          "path": path,
          "query": query,
          "hint": "Use 'mockpit mock test --debug' to see why mocks didn't match"
        });

        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(error_body.to_string()))
            .expect("building 404 response should not fail")
    }
}

/// Generate a mock response with template rendering
async fn generate_mock_response(
    mock_def: &mockpit::engine::MockDefinition,
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
    captures: FxHashMap<String, String>,
) -> Result<Response, anyhow::Error> {
    let response_generator = &mock_def.response;

    // Generate the response body
    let body_bytes = response_generator
        .generate_dynamic(
            method.as_str(),
            path,
            query,
            headers,
            body,
            captures,
            mock_def.vars.as_ref(),
        )
        .await?;

    // Use dynamic status if provided, otherwise use mock definition's status
    let final_status = body_bytes.status.unwrap_or(response_generator.status);

    // Build response
    let mut response = Response::builder().status(final_status);

    // Add mock definition headers
    for (key, value) in &response_generator.headers {
        if let Ok(header_value) = HeaderValue::from_str(value) {
            response = response.header(key.as_str(), header_value);
        }
    }

    // Add dynamic headers if any
    if let Some(dynamic_headers) = &body_bytes.headers {
        for (key, value) in dynamic_headers {
            if let Ok(header_value) = HeaderValue::from_str(value) {
                response = response.header(key.as_str(), header_value);
            }
        }
    }

    // Add X-Mock-Id header
    response = response.header("X-Mock-Id", mock_def.id.as_str());

    Ok(response.body(Body::from(body_bytes.body))?)
}

/// Template render handler
#[derive(serde::Deserialize)]
struct RenderRequest {
    template: String,
    #[serde(default)]
    context: serde_json::Value,
}

#[allow(clippy::expect_used)]
async fn render_handler(axum::Json(req): axum::Json<RenderRequest>) -> Response {
    // Build RequestContext from the provided context
    let mut req_ctx = RequestContext::new();

    if !req.context.is_null() {
        if let Some(captures) = req.context.get("captures").and_then(|v| v.as_object()) {
            for (k, v) in captures {
                if let Some(s) = v.as_str() {
                    req_ctx.captures.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(headers) = req.context.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in headers {
                if let Some(s) = v.as_str() {
                    req_ctx.headers.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(query) = req.context.get("query").and_then(|v| v.as_object()) {
            for (k, v) in query {
                if let Some(s) = v.as_str() {
                    req_ctx.query.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(body) = req.context.get("body") {
            req_ctx.body = Some(body.to_string());
            req_ctx.body_json = Some(body.clone());
        }
    }

    match mockpit::template::render_template(&req.template, &req_ctx) {
        Ok(result) => {
            let content_type = if result.trim().starts_with('{') || result.trim().starts_with('[') {
                "application/json"
            } else {
                "text/plain"
            };

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(result))
                .expect("building render response should not fail")
        }
        Err(e) => Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Body::from(format!("Template error: {e}")))
            .expect("building error response should not fail"),
    }
}

/// List mocks handler
#[allow(clippy::expect_used)]
async fn list_mocks_handler(State(state): State<Arc<MockServerState>>) -> Response {
    let mocks = state.registry.get_enabled_mocks();

    let mock_list: Vec<serde_json::Value> = mocks
        .iter()
        .map(|m| {
            serde_json::json!({
              "id": m.id,
              "priority": m.priority,
              "enabled": m.enabled,
              "methods": m.request.methods.iter().map(Method::as_str).collect::<Vec<_>>(),
              "url_patterns_count": m.request.url_patterns.len(),
              "status": m.response.status.as_u16(),
            })
        })
        .collect();

    let body = serde_json::json!({
      "count": mock_list.len(),
      "mocks": mock_list,
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("building list response should not fail")
}
