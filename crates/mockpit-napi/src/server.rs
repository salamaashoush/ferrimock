//! MockpitServer class — the main entry point for Node.js users.

use crate::handler_bridge::HandlerFnRef;
use crate::http_ns::JsHandler;
use crate::request_context::MockpitRequest;
use crate::types::JsHandlerResponse;
use mockpit::engine::types::ResponseGeneratorExt;
use mockpit::engine::{MockMatcher, MockRegistry};
use mockpit::types::{BodySource, DynamicResponse, RequestContext};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use std::sync::Arc;

/// High-performance HTTP mock server.
///
/// Supports both MSW-style handler functions and declarative YAML/JSON mocks.
/// All mocks (handler-based and declarative) live in the same registry with
/// the same priority and matching system.
///
/// @example
/// ```ts
/// import { http, MockResponse, MockpitServer } from '@mockpit/node'
///
/// const server = new MockpitServer()
///
/// server.useHandlers([
///   http.get('/api/users/:id', ({ params }) => {
///     return MockResponse.json({ id: params.id, name: 'John' })
///   }),
/// ])
///
/// const url = await server.listen(3000)
/// // ... use the mock server ...
/// await server.close()
/// ```
#[napi]
pub struct MockpitServer {
    registry: Arc<MockRegistry>,
    /// FunctionRef map for interceptor fast path: mock_id -> handler FunctionRef.
    /// Used by match_request to call JS handlers directly without TSFN overhead.
    handler_refs: Arc<std::sync::RwLock<HashMap<String, Arc<HandlerFnRef>>>>,
    shutdown_tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    port: Arc<std::sync::atomic::AtomicU16>,
}

#[napi]
impl MockpitServer {
    /// Create a new mock server instance.
    #[napi(constructor)]
    pub fn new() -> Self {
        let registry = Arc::new(MockRegistry::new());
        Self {
            registry,
            handler_refs: Arc::new(std::sync::RwLock::new(HashMap::new())),
            shutdown_tx: Arc::new(std::sync::Mutex::new(None)),
            port: Arc::new(std::sync::atomic::AtomicU16::new(0)),
        }
    }

    /// Register handler-based mocks.
    ///
    /// Handlers are added to the same registry as declarative mocks.
    /// They participate in the same priority-based matching system.
    ///
    /// @param handlers - Array of handlers created by `http.get()`, `http.post()`, etc.
    #[napi]
    pub fn use_handlers(&mut self, handlers: Vec<&mut JsHandler>) -> Result<()> {
        for handler in handlers {
            let fn_ref = handler.take_fn_ref();
            let mock_def = handler.take()?;
            let mock_id = mock_def.id.to_string();
            self.registry.add_mock(mock_def);

            if let Some(fn_ref) = fn_ref {
                self.handler_refs.write().unwrap().insert(mock_id, fn_ref);
            }
        }
        Ok(())
    }

    /// Add runtime handlers (MSW's `server.use()`).
    ///
    /// Runtime handlers take priority over initial handlers (priority 200 vs 100).
    /// They are added to the same registry and participate in matching.
    ///
    /// @param handlers - Array of handlers created by `http.get()`, `http.post()`, etc.
    #[napi(js_name = "use")]
    pub fn use_runtime(&mut self, handlers: Vec<&mut JsHandler>) -> Result<()> {
        for handler in handlers {
            let fn_ref = handler.take_fn_ref();
            let mut mock_def = handler.take()?;
            // Runtime handlers get higher priority than initial handlers
            mock_def.priority = 200;
            let mock_id = mock_def.id.to_string();
            self.registry.add_mock(mock_def);

            if let Some(fn_ref) = fn_ref {
                self.handler_refs.write().unwrap().insert(mock_id, fn_ref);
            }
        }
        Ok(())
    }

    /// Re-enable consumed one-time handlers (MSW's `server.restoreHandlers()`).
    ///
    /// One-time handlers (`{ once: true }`) are disabled after first match.
    /// This method re-enables them so they can match again.
    #[napi]
    pub fn restore_handlers(&self) -> Result<()> {
        let all_mocks = self.registry.get_all_mocks();
        for mock in &all_mocks {
            if mock.once && !mock.enabled {
                let _ = self.registry.enable_mock(mock.id.as_str());
            }
        }
        Ok(())
    }

    /// Remove all handler-based mocks (those with IDs starting with "handler:").
    ///
    /// Declarative mocks loaded from files are not affected.
    #[napi]
    pub fn reset_handlers(&self) -> Result<()> {
        let handler_ids: Vec<String> = self
            .registry
            .get_all_mocks()
            .iter()
            .filter(|m| m.id.starts_with("handler:"))
            .map(|m| m.id.to_string())
            .collect();

        for id in &handler_ids {
            self.registry.remove_mock(id);
            self.handler_refs.write().unwrap().remove(id);
        }
        Ok(())
    }

    /// Load declarative mocks from a directory containing YAML/JSON/HAR files.
    ///
    /// @param dirPath - Path to a directory containing mock definition files.
    /// @returns Number of mocks loaded.
    #[napi]
    pub async fn load_mocks(&self, dir_path: String) -> Result<u32> {
        let count = self
            .registry
            .load_from_directory(&dir_path)
            .await
            .map_err(|e| Error::from_reason(format!("Failed to load mocks: {e}")))?;
        #[allow(clippy::cast_possible_truncation)]
        Ok(count as u32)
    }

    /// Load mocks from a single file (YAML, JSON, or HAR).
    ///
    /// @param filePath - Path to a .yaml, .yml, .json, or .har file.
    /// @returns Number of mocks loaded.
    #[napi]
    pub async fn load_mock_file(&self, file_path: String) -> Result<u32> {
        let path = std::path::Path::new(&file_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let count = if ext == "har" {
            use mockpit::config::HarLoader;
            let loader = HarLoader::new();
            let mock_configs = loader
                .load_from_file(path)
                .await
                .map_err(|e| Error::from_reason(format!("Failed to load HAR file: {e}")))?;
            let mut count = 0usize;
            for config in mock_configs {
                let mock_def = config
                    .into_mock_definition()
                    .await
                    .map_err(|e| Error::from_reason(format!("Failed to create mock: {e}")))?;
                self.registry.add_mock(mock_def);
                count += 1;
            }
            count
        } else {
            self.registry
                .load_collection_file(path)
                .await
                .map_err(|e| Error::from_reason(format!("Failed to load mock file: {e}")))?
        };

        #[allow(clippy::cast_possible_truncation)]
        Ok(count as u32)
    }

    /// Add a single mock from a JSON configuration object.
    ///
    /// @param config - Mock configuration as JSON (same format as YAML mock files).
    /// @returns The mock ID.
    #[napi]
    pub async fn add_mock(&self, config: serde_json::Value) -> Result<String> {
        let mock_config: mockpit::config::MockConfig = serde_json::from_value(config)
            .map_err(|e| Error::from_reason(format!("Invalid mock config: {e}")))?;

        let mock_def = mock_config
            .into_mock_definition()
            .await
            .map_err(|e| Error::from_reason(format!("Failed to create mock: {e}")))?;

        let id = mock_def.id.to_string();
        self.registry.add_mock(mock_def);
        Ok(id)
    }

    /// Remove a mock by ID.
    ///
    /// @param id - The mock ID to remove.
    /// @returns `true` if the mock was found and removed.
    #[napi]
    pub fn remove_mock(&self, id: String) -> bool {
        self.registry.remove_mock(&id).is_some()
    }

    /// Get the number of registered mocks.
    #[napi(getter)]
    pub fn mock_count(&self) -> u32 {
        self.registry.len() as u32
    }

    /// List all registered handlers.
    ///
    /// Returns an array of handler info objects with id and method/path info.
    /// Equivalent to MSW's `server.listHandlers()`.
    #[napi]
    pub fn list_handlers(&self) -> Vec<HandlerInfo> {
        self.registry
            .get_all_mocks()
            .iter()
            .map(|m| HandlerInfo {
                id: m.id.to_string(),
                methods: m.request.methods.iter().map(|m| m.to_string()).collect(),
                enabled: m.enabled,
            })
            .collect()
    }

    /// Start the mock server on the given port.
    ///
    /// @param port - Port number (default: 0 for random available port).
    /// @returns The URL the server is listening on (e.g., `http://127.0.0.1:3000`).
    #[napi]
    pub async fn listen(&self, port: Option<u32>) -> Result<String> {
        {
            let guard = self.shutdown_tx.lock().map_err(|e| Error::from_reason(e.to_string()))?;
            if guard.is_some() {
                return Err(Error::from_reason("Server is already running"));
            }
        }

        let port = port.unwrap_or(0) as u16;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let registry = Arc::clone(&self.registry);
        let matcher = MockMatcher::new((*registry).clone());

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .map_err(|e| Error::from_reason(format!("Failed to bind: {e}")))?;

        let actual_port = listener
            .local_addr()
            .map_err(|e| Error::from_reason(format!("Failed to get address: {e}")))?
            .port();

        self.port.store(actual_port, std::sync::atomic::Ordering::Relaxed);

        let state = Arc::new(ServerState { matcher });

        let app = axum::Router::new()
            .fallback(mock_handler)
            .with_state(state);

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        *self.shutdown_tx.lock().map_err(|e| Error::from_reason(e.to_string()))? = Some(shutdown_tx);
        Ok(format!("http://127.0.0.1:{actual_port}"))
    }

    /// Stop the mock server.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let tx = self.shutdown_tx.lock()
            .map_err(|e| Error::from_reason(e.to_string()))?
            .take();
        if let Some(tx) = tx {
            let _ = tx.send(());
        }
        self.port.store(0, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Check if the server is running.
    #[napi(getter)]
    pub fn is_running(&self) -> bool {
        self.shutdown_tx.lock().is_ok_and(|g| g.is_some())
    }

    /// Get the port the server is listening on.
    #[napi(getter)]
    pub fn port(&self) -> Option<u32> {
        let p = self.port.load(std::sync::atomic::Ordering::Relaxed);
        if p == 0 { None } else { Some(u32::from(p)) }
    }

    /// Match a request against the mock registry and generate the response.
    ///
    /// **Optimization**: For handler mocks, uses `FunctionRef` to call the JS
    /// handler directly from the deferred resolver callback (JS thread).
    /// This eliminates the ~22us TSFN queue+wakeup overhead, replacing it with
    /// a direct `napi_call_function` (~1us).
    ///
    /// Flow:
    /// 1. Rust matching on tokio (~12us)
    /// 2. Deferred resolver on JS thread:
    ///    - Declarative: response already built
    ///    - Handler: FunctionRef direct call (~1us)
    ///
    /// Returns null if no mock matches.
    #[napi]
    pub fn match_request<'env>(
        &self,
        env: &'env Env,
        method: String,
        path: String,
        query: Option<String>,
        headers: Option<HashMap<String, String>>,
        body: Option<String>,
    ) -> Result<PromiseRaw<'env, MaybePromise>> {
        let registry = Arc::clone(&self.registry);
        let handler_refs = Arc::clone(&self.handler_refs);

        env.spawn_future_with_callback(
            // === Phase 1: Rust matching on tokio ===
            async move {
                let http_method: http::Method = method
                    .parse()
                    .map_err(|e| Error::from_reason(format!("Invalid method: {e}")))?;

                let mut header_map = http::HeaderMap::new();
                if let Some(ref h) = headers {
                    for (name, value) in h {
                        if let (Ok(n), Ok(v)) = (
                            http::header::HeaderName::try_from(name.as_str()),
                            http::header::HeaderValue::try_from(value.as_str()),
                        ) {
                            header_map.insert(n, v);
                        }
                    }
                }

                let body_bytes = body.as_deref().map(str::as_bytes);

                let matcher = MockMatcher::new((*registry).clone());
                let mock_match = matcher.find_match(
                    &http_method,
                    &path,
                    query.as_deref(),
                    &header_map,
                    body_bytes,
                );

                let Some(mock_match) = mock_match else {
                    return Ok(MatchPhaseResult::NoMatch);
                };

                let mock_def = &mock_match.mock;
                let captures = mock_match.captures;
                let is_handler = matches!(&mock_def.response.body, BodySource::Handler(_));

                if is_handler {
                    // Handler mock — build context, defer handler call to JS thread
                    let mut context = RequestContext::from_request(
                        method.as_str(),
                        &path,
                        query.as_deref(),
                        &header_map,
                        body_bytes,
                    );
                    context.captures = captures;
                    Ok(MatchPhaseResult::HandlerMatch {
                        mock_id: mock_def.id.to_string(),
                        status: mock_def.response.status,
                        def_headers: mock_def.response.headers.clone(),
                        context,
                    })
                } else {
                    // Declarative — generate response fully on tokio
                    let dynamic = mock_def
                        .response
                        .generate_dynamic(
                            method.as_str(),
                            &path,
                            query.as_deref(),
                            &header_map,
                            body_bytes,
                            captures,
                            mock_def.vars.as_ref(),
                        )
                        .await
                        .map_err(|e| Error::from_reason(format!("Response generation failed: {e}")))?;

                    Ok(MatchPhaseResult::DeclarativeResponse(
                        build_matched_response(&mock_def.id, mock_def.response.status, &mock_def.response.headers, dynamic),
                    ))
                }
            },
            // === Phase 2: Deferred resolver on JS thread ===
            move |env, result| -> Result<MaybePromise> {
                match result {
                    MatchPhaseResult::NoMatch => Ok(MaybePromise::resolved(env, None)?),
                    MatchPhaseResult::DeclarativeResponse(resp) => Ok(MaybePromise::resolved(env, Some(resp))?),
                    MatchPhaseResult::HandlerMatch {
                        mock_id,
                        status: default_status,
                        def_headers,
                        context,
                    } => {
                        let refs = handler_refs.read()
                            .map_err(|e| Error::from_reason(e.to_string()))?;
                        let fn_ref = refs.get(&mock_id).ok_or_else(|| {
                            Error::from_reason(format!("No FunctionRef for handler: {mock_id}"))
                        })?;

                        // Direct napi_call_function via FunctionRef — ~1us vs ~22us TSFN
                        let func = fn_ref.borrow_back(env)?;
                        let req = MockpitRequest::new(context);
                        let raw_result: Unknown = func.call(req)?;

                        // Check if the handler returned a Promise (async handler)
                        let mut is_promise = false;
                        #[allow(unsafe_code)]
                        unsafe {
                            napi::sys::napi_is_promise(
                                env.raw(),
                                raw_result.raw(),
                                &mut is_promise,
                            );
                        }

                        if is_promise {
                            // Async handler — chain .then() to convert the resolved value.
                            // napi_resolve_deferred with a Promise auto-flattens per JS spec.
                            #[allow(unsafe_code)]
                            let promise_raw: PromiseRaw<'_, Option<JsHandlerResponse>> = unsafe {
                                FromNapiValue::from_napi_value(env.raw(), raw_result.raw())?
                            };
                            let chained = promise_raw.then(move |ctx| {
                                match ctx.value {
                                    Some(js_resp) => {
                                        let dynamic = DynamicResponse::from(js_resp);
                                        Ok(Some(build_matched_response(&mock_id, default_status, &def_headers, dynamic)))
                                    }
                                    None => Ok(Some(MatchedResponse {
                                        status: 200,
                                        headers: HashMap::new(),
                                        body: String::new(),
                                        mock_id: mock_id.to_string(),
                                    })),
                                }
                            })?;
                            Ok(MaybePromise(chained.value().value))
                        } else {
                            // Sync handler — extract directly, no Promise overhead
                            #[allow(unsafe_code)]
                            let resp: Option<JsHandlerResponse> = unsafe {
                                FromNapiValue::from_napi_value(env.raw(), raw_result.raw())?
                            };
                            match resp {
                                Some(js_resp) => {
                                    let dynamic = DynamicResponse::from(js_resp);
                                    Ok(MaybePromise::resolved(env, Some(build_matched_response(&mock_id, default_status, &def_headers, dynamic)))?)
                                }
                                None => Ok(MaybePromise::resolved(env, Some(MatchedResponse {
                                    status: 200,
                                    headers: HashMap::new(),
                                    body: String::new(),
                                    mock_id: mock_id.to_string(),
                                }))?),
                            }
                        }
                    }
                }
            },
        )
    }
}

/// Handler info returned by `listHandlers()`.
#[napi(object)]
pub struct HandlerInfo {
    pub id: String,
    pub methods: Vec<String>,
    pub enabled: bool,
}

/// Result of matching a request against the mock registry.
#[napi(object)]
pub struct MatchedResponse {
    pub status: u32,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub mock_id: String,
}

// -- Internal types --

/// Wrapper around a raw napi_value that may be either a direct value or a Promise.
/// `ToNapiValue` passes through the raw pointer, so if it's a Promise,
/// `napi_resolve_deferred` auto-flattens per the JS spec.
struct MaybePromise(napi::sys::napi_value);

// SAFETY: MaybePromise holds a raw napi_value that stays valid within the
// resolver callback scope (same JS thread, same GC epoch).
#[allow(unsafe_code)]
unsafe impl Send for MaybePromise {}

impl ToNapiValue for MaybePromise {
    #[allow(unsafe_code)]
    unsafe fn to_napi_value(_env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        Ok(val.0)
    }
}

impl MaybePromise {
    /// Create from a sync value by converting to napi_value.
    fn resolved(env: &Env, value: Option<MatchedResponse>) -> Result<Self> {
        #[allow(unsafe_code)]
        let raw = unsafe { ToNapiValue::to_napi_value(env.raw(), value)? };
        Ok(MaybePromise(raw))
    }
}

/// Phase 1 result, sent from tokio to the JS-thread resolver.
enum MatchPhaseResult {
    NoMatch,
    DeclarativeResponse(MatchedResponse),
    HandlerMatch {
        mock_id: String,
        status: http::StatusCode,
        def_headers: FxHashMap<String, String>,
        context: RequestContext,
    },
}

/// Build a MatchedResponse from a DynamicResponse + mock metadata.
fn build_matched_response(
    mock_id: &str,
    default_status: http::StatusCode,
    def_headers: &FxHashMap<String, String>,
    dynamic: DynamicResponse,
) -> MatchedResponse {
    let status = dynamic.status.unwrap_or(default_status).as_u16();
    let mut headers: HashMap<String, String> = def_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Some(dyn_headers) = dynamic.headers {
        headers.extend(dyn_headers);
    }
    let body = String::from_utf8(dynamic.body.to_vec()).unwrap_or_default();
    MatchedResponse {
        status: u32::from(status),
        headers,
        body,
        mock_id: mock_id.to_string(),
    }
}

// -- Internal server implementation --

#[derive(Clone)]
struct ServerState {
    matcher: MockMatcher,
}

/// Catch-all handler that matches incoming requests against the mock registry.
async fn mock_handler(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use axum::body::Body;
    use axum::response::IntoResponse;
    use http::header;

    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path();
    let query = uri.query();
    let headers = req.headers().clone();

    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .ok();
    let body_ref = body_bytes
        .as_ref()
        .filter(|b| !b.is_empty())
        .map(|b| b.as_ref());

    let mock_match = state
        .matcher
        .find_match(&method, path, query, &headers, body_ref);

    if let Some(mock_match) = mock_match {
        let mock_def = &mock_match.mock;
        let captures = mock_match.captures;

        let dynamic = mock_def
            .response
            .generate_dynamic(
                method.as_str(),
                path,
                query,
                &headers,
                body_ref,
                captures,
                mock_def.vars.as_ref(),
            )
            .await;

        match dynamic {
            Ok(resp) => {
                let status = resp.status.unwrap_or(mock_def.response.status);
                let mut response = axum::http::Response::builder().status(status);

                for (key, value) in &mock_def.response.headers {
                    response = response.header(key.as_str(), value.as_str());
                }

                if let Some(dyn_headers) = &resp.headers {
                    for (key, value) in dyn_headers {
                        response = response.header(key.as_str(), value.as_str());
                    }
                }

                response
                    .header("x-mockpit-id", mock_def.id.as_str())
                    .body(Body::from(resp.body))
                    .unwrap_or_else(|_| {
                        (http::StatusCode::INTERNAL_SERVER_ERROR, "Response build error")
                            .into_response()
                    })
            }
            Err(e) => {
                let error_body = serde_json::json!({
                    "error": "Mock response generation failed",
                    "mock_id": mock_def.id.as_str(),
                    "details": e.to_string()
                });

                (
                    http::StatusCode::INTERNAL_SERVER_ERROR,
                    [(header::CONTENT_TYPE, "application/json")],
                    error_body.to_string(),
                )
                    .into_response()
            }
        }
    } else {
        let error_body = serde_json::json!({
            "error": "No matching mock found",
            "method": method.as_str(),
            "path": path,
            "query": query
        });

        (
            http::StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            error_body.to_string(),
        )
            .into_response()
    }
}
