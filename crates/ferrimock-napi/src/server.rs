//! FerrimockServer class — the main entry point for Node.js users.

use crate::handler_bridge::HandlerFnRef;
use crate::http_ns::RequestHandler;
use crate::request_context::{HandlerKind, ResolverArg};
use crate::types::HandlerResponse;
use ferrimock::engine::types::ResponseGeneratorExt;
use ferrimock::engine::{MockMatcher, MockRegistry};
use ferrimock::types::{BodySource, DynamicResponse, RequestContext};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use std::sync::Arc;

/// Scope tag for handlers registered via `use()` so `resetRuntimeHandlers`
/// removes only them (MSW's resetHandlers keeps initial handlers).
const RUNTIME_SCOPE: &str = "ferrimock:runtime";

/// High-performance HTTP mock server.
///
/// Supports both MSW-style handler functions and declarative YAML/JSON mocks.
/// All mocks (handler-based and declarative) live in the same registry with
/// the same priority and matching system.
///
/// @example
/// ```ts
/// import { http, HttpResponse, FerrimockServer } from '@ferrimock/node'
///
/// const server = new FerrimockServer()
///
/// server.useHandlers([
///   http.get('/api/users/:id', ({ params }) => {
///     return HttpResponse.json({ id: params.id, name: 'John' })
///   }),
/// ])
///
/// const url = await server.listen(3000)
/// // ... use the mock server ...
/// await server.close()
/// ```
#[napi]
pub struct FerrimockServer {
    registry: Arc<MockRegistry>,
    /// Single long-lived matcher reused across all `match_request` calls.
    /// Shares the registry internals (Arc), so newly added mocks are visible.
    /// Its LRU cache warms across requests; cleared on any mock mutation.
    matcher: MockMatcher,
    /// FunctionRef map for interceptor fast path: mock_id -> handler FunctionRef.
    /// Used by match_request to call JS handlers directly without TSFN overhead.
    handler_refs: Arc<std::sync::RwLock<HashMap<String, Arc<HandlerFnRef>>>>,
    /// The predicate as the user wrote it, per mock id — the engine only
    /// keeps compiled patterns, but `listHandlers()` surfaces MSW's
    /// display form ("GET /users/:id", "query GetUser (origin: *)").
    handler_patterns: Arc<std::sync::RwLock<HashMap<String, String>>>,
    shutdown_tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    port: Arc<std::sync::atomic::AtomicU16>,
}

#[napi]
impl FerrimockServer {
    /// Create a new mock server instance.
    #[napi(constructor)]
    pub fn new() -> Self {
        let registry = Arc::new(MockRegistry::new());
        let matcher = MockMatcher::new((*registry).clone());
        Self {
            registry,
            matcher,
            handler_refs: Arc::new(std::sync::RwLock::new(HashMap::new())),
            handler_patterns: Arc::new(std::sync::RwLock::new(HashMap::new())),
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
    pub fn use_handlers(&mut self, handlers: Vec<&mut RequestHandler>) -> Result<()> {
        for handler in handlers {
            let fn_ref = handler.take_fn_ref();
            let pattern = handler.pattern.clone();
            let mock_def = handler.take()?;
            let mock_id = mock_def.id.to_string();
            self.registry.add_mock(mock_def);

            if let Some(pattern) = pattern {
                self.handler_patterns
                    .write()
                    .unwrap()
                    .insert(mock_id.clone(), pattern);
            }
            if let Some(fn_ref) = fn_ref {
                self.handler_refs.write().unwrap().insert(mock_id, fn_ref);
            }
        }
        self.matcher.clear_cache();
        Ok(())
    }

    /// Add runtime handlers (MSW's `server.use()`).
    ///
    /// Runtime handlers take priority over initial handlers (priority 200 vs 100)
    /// and are scoped so `resetRuntimeHandlers()` removes only them.
    ///
    /// @param handlers - Array of handlers created by `http.get()`, `http.post()`, etc.
    #[napi(js_name = "use")]
    pub fn use_runtime(&mut self, handlers: Vec<&mut RequestHandler>) -> Result<()> {
        for handler in handlers {
            let fn_ref = handler.take_fn_ref();
            let pattern = handler.pattern.clone();
            let mut mock_def = handler.take()?;
            // Runtime handlers get higher priority than initial handlers
            mock_def.priority = 200;
            mock_def.scope = Some(RUNTIME_SCOPE.into());
            let mock_id = mock_def.id.to_string();
            self.registry.add_mock(mock_def);

            if let Some(pattern) = pattern {
                self.handler_patterns
                    .write()
                    .unwrap()
                    .insert(mock_id.clone(), pattern);
            }
            if let Some(fn_ref) = fn_ref {
                self.handler_refs.write().unwrap().insert(mock_id, fn_ref);
            }
        }
        self.matcher.clear_cache();
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
        self.matcher.clear_cache();
        Ok(())
    }

    /// MSW's `server.resetHandlers()`: remove runtime handlers added via
    /// `use()` and restore initial handlers (re-enabling consumed one-time
    /// handlers). Handlers registered via `useHandlers()` stay.
    #[napi]
    pub fn reset_runtime_handlers(&self) -> Result<()> {
        let runtime_ids: Vec<String> = self
            .registry
            .get_all_mocks()
            .iter()
            .filter(|m| m.scope.as_deref() == Some(RUNTIME_SCOPE))
            .map(|m| m.id.to_string())
            .collect();
        for id in &runtime_ids {
            self.registry.remove_mock(id);
            self.handler_refs.write().unwrap().remove(id);
        }
        self.matcher.clear_cache();
        self.restore_handlers()
    }

    /// Remove ALL handler-based mocks (initial and runtime). Used by
    /// MSW's `server.resetHandlers(...nextHandlers)` overload, which
    /// replaces the initial set. Declarative mocks loaded from files are
    /// not affected.
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
        self.matcher.clear_cache();
        Ok(())
    }

    /// Load declarative mocks from a directory containing YAML/JSON/HAR files.
    ///
    /// @param dirPath - Path to a directory containing mock definition files.
    /// @returns Number of mocks loaded.
    #[napi]
    pub async fn load_mocks(&self, dir_path: String) -> Result<u32> {
        // Scripts stay with the JS side: ferrimock's loader runs
        // .js/.mjs mock files on V8 in this same process.
        let options = ferrimock::engine::registry::DirLoadOptions {
            load_scripts: false,
        };
        let count = self
            .registry
            .load_from_directory_with(&dir_path, options)
            .await
            .map_err(|e| Error::from_reason(format!("Failed to load mocks: {e}")))?;
        self.matcher.clear_cache();
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
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let count = if ext == "har" {
            use ferrimock::config::HarLoader;
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

        self.matcher.clear_cache();
        #[allow(clippy::cast_possible_truncation)]
        Ok(count as u32)
    }

    /// Add a single mock from a JSON configuration object.
    ///
    /// @param config - Mock configuration as JSON (same format as YAML mock files).
    /// @returns The mock ID.
    #[napi]
    pub async fn add_mock(&self, config: serde_json::Value) -> Result<String> {
        let mock_config: ferrimock::config::MockConfig = serde_json::from_value(config)
            .map_err(|e| Error::from_reason(format!("Invalid mock config: {e}")))?;

        let mock_def = mock_config
            .into_mock_definition()
            .await
            .map_err(|e| Error::from_reason(format!("Failed to create mock: {e}")))?;

        let id = mock_def.id.to_string();
        self.registry.add_mock(mock_def);
        self.matcher.clear_cache();
        Ok(id)
    }

    /// Remove a mock by ID.
    ///
    /// @param id - The mock ID to remove.
    /// @returns `true` if the mock was found and removed.
    #[napi]
    pub fn remove_mock(&self, id: String) -> bool {
        let removed = self.registry.remove_mock(&id).is_some();
        if removed {
            self.matcher.clear_cache();
        }
        removed
    }

    /// Get the number of registered mocks.
    #[napi(getter)]
    pub fn mock_count(&self) -> u32 {
        self.registry.len() as u32
    }

    /// Whether any registered mock matches on the request body (body or GraphQL
    /// matcher). Lets the interceptor skip reading the request body when no mock
    /// could use it.
    #[napi(getter)]
    pub fn needs_request_body(&self) -> bool {
        self.registry.needs_request_body()
    }

    /// Whether any registered mock needs request headers (header matchers,
    /// handler mocks, or header-referencing templates). The interceptor
    /// skips marshalling headers when false.
    #[napi(getter)]
    pub fn needs_request_headers(&self) -> bool {
        self.registry.needs_request_headers()
    }

    /// List all registered handlers.
    ///
    /// Returns an array of handler info objects with id, method/path info,
    /// and MSW's display strings: `pattern` is the predicate as the user
    /// wrote it and `header` is MSW's `info.header` ("GET /users/:id",
    /// "query GetUser (origin: *)"). Equivalent to MSW's
    /// `server.listHandlers()`. WebSocket mocks carry `kind: "websocket"`
    /// (MSW's WebSocketHandler tag).
    #[napi]
    pub fn list_handlers(&self) -> Vec<HandlerInfo> {
        let patterns = self.handler_patterns.read().unwrap();
        self.registry
            .get_all_mocks_in_registration_order()
            .iter()
            .map(|m| {
                // Declarative mocks never register a display pattern; a
                // single exact URL is still a faithful display form.
                let pattern = patterns.get(m.id.as_str()).cloned().or_else(|| {
                    match m.request.url_patterns.as_slice() {
                        [ferrimock::types::UrlPattern::Exact(path)] => Some(path.clone()),
                        _ => None,
                    }
                });
                let methods: Vec<String> =
                    m.request.methods.iter().map(|m| m.to_string()).collect();
                let header = pattern.as_ref().map(|pattern| {
                    if m.request.graphql_matcher.is_some() {
                        // GraphQL patterns already carry MSW's full header.
                        pattern.clone()
                    } else {
                        // MSW displays method-agnostic handlers as /.+/.
                        let method = methods.first().map_or("/.+/", String::as_str);
                        format!("{method} {pattern}")
                    }
                });
                HandlerInfo {
                    id: m.id.to_string(),
                    methods,
                    enabled: m.enabled,
                    kind: m
                        .streaming
                        .as_ref()
                        .filter(|s| s.is_ws())
                        .map(|_| "websocket".to_string()),
                    pattern,
                    header,
                    match_count: u32::try_from(self.registry.match_count(m.id.as_str()))
                        .unwrap_or(u32::MAX),
                }
            })
            .collect()
    }

    /// Requests a mock has served since the last reset.
    ///
    /// Counting is always on — unlike MSW, asserting a handler ran needs no
    /// spy inside the resolver, and it works for declarative mocks too.
    #[napi]
    pub fn match_count(&self, id: String) -> u32 {
        u32::try_from(self.registry.match_count(&id)).unwrap_or(u32::MAX)
    }

    /// Every mock that has served a request, busiest first.
    #[napi]
    pub fn match_counts(&self) -> Vec<MockMatchCount> {
        self.registry
            .match_counts()
            .into_iter()
            .map(|(mock_id, count)| MockMatchCount {
                mock_id,
                count: u32::try_from(count).unwrap_or(u32::MAX),
            })
            .collect()
    }

    /// Reset every match count — call between tests.
    #[napi]
    pub fn reset_match_counts(&self) {
        self.registry.reset_match_counts();
    }

    /// Every WebSocket mock matching a connection handshake, in
    /// precedence order — the interceptor lane dispatches an intercepted
    /// connection to ALL matching `ws` handlers (MSW semantics). No side
    /// effects (no `once` consumption, no call tracking).
    ///
    /// @param url - The connection URL (`wss://host/path`).
    #[napi]
    pub fn match_ws_connections(&self, url: String) -> Result<Vec<WsConnectionMatch>> {
        let uri: http::Uri = url
            .parse()
            .map_err(|e| Error::from_reason(format!("Invalid WebSocket URL: {e}")))?;
        let host = uri
            .authority()
            .map(std::string::ToString::to_string)
            .unwrap_or_default();
        let path = uri.path().to_string();
        let query = uri.query().map(str::to_string);
        // The intercepted connection URL carries the real scheme, so
        // HrefRegex patterns only test that reconstruction (a regex
        // pinning `ws://` must not match a `wss://` connection here).
        let scheme = uri.scheme_str().filter(|s| *s == "ws" || *s == "wss");

        let mut headers = http::HeaderMap::new();
        if let Ok(value) = http::HeaderValue::from_str(&host) {
            headers.insert(http::header::HOST, value);
        }
        headers.insert(
            http::header::UPGRADE,
            http::HeaderValue::from_static("websocket"),
        );
        headers.insert(
            http::header::CONNECTION,
            http::HeaderValue::from_static("Upgrade"),
        );

        Ok(self
            .matcher
            .find_ws_matches(&path, query.as_deref(), &headers, scheme)
            .into_iter()
            .map(|m| WsConnectionMatch {
                mock_id: m.mock.id.to_string(),
                params: crate::request_context::msw_params_map(&m.captures),
            })
            .collect())
    }

    /// Start the mock server on the given port.
    ///
    /// @param port - Port number (default: 0 for random available port).
    /// @returns The URL the server is listening on (e.g., `http://127.0.0.1:3000`).
    #[napi]
    pub async fn listen(&self, port: Option<u32>) -> Result<String> {
        {
            let guard = self
                .shutdown_tx
                .lock()
                .map_err(|e| Error::from_reason(e.to_string()))?;
            if guard.is_some() {
                return Err(Error::from_reason("Server is already running"));
            }
        }

        let port = port.unwrap_or(0) as u16;
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let matcher = self.matcher.clone();

        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .map_err(|e| Error::from_reason(format!("Failed to bind: {e}")))?;

        let actual_port = listener
            .local_addr()
            .map_err(|e| Error::from_reason(format!("Failed to get address: {e}")))?
            .port();

        self.port
            .store(actual_port, std::sync::atomic::Ordering::Relaxed);

        let state = Arc::new(ServerState { matcher });

        let app = axum::Router::new().fallback(mock_handler).with_state(state);

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        *self
            .shutdown_tx
            .lock()
            .map_err(|e| Error::from_reason(e.to_string()))? = Some(shutdown_tx);
        Ok(format!("http://127.0.0.1:{actual_port}"))
    }

    /// Stop the mock server.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let tx = self
            .shutdown_tx
            .lock()
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
    /// Returns null if no mock matches. When a handler falls through
    /// (returns null/undefined), resolves with `{ fallthrough: true,
    /// mockId }` — re-call with that ID added to `excludeIds` to try the
    /// next candidate (MSW semantics; the JS interceptor loops).
    #[allow(private_interfaces)] // MaybePromise is an internal raw-value wrapper
    #[napi(ts_return_type = "Promise<MatchedResponse | null>")]
    #[allow(clippy::too_many_arguments)]
    pub fn match_request<'env>(
        &self,
        env: &'env Env,
        method: String,
        path: String,
        query: Option<String>,
        headers: Option<HashMap<String, String>>,
        body: Option<Either<String, Uint8Array>>,
        request_id: Option<String>,
        exclude_ids: Option<Vec<String>>,
    ) -> Result<PromiseRaw<'env, MaybePromise>> {
        let handler_refs = Arc::clone(&self.handler_refs);
        // Reuse the long-lived matcher (cheap Arc-based clone) instead of
        // building a fresh one with an empty LRU per request.
        let matcher = self.matcher.clone();
        // Second clone captured by the JS-thread resolver: undoes `once`
        // consumption when the handler falls through.
        let resolver_matcher = self.matcher.clone();

        // Copy the body out of JS-owned memory before crossing to tokio
        // (a Uint8Array view must not outlive the JS callframe).
        let body: Option<Vec<u8>> = body.map(|b| match b {
            Either::A(s) => s.into_bytes(),
            Either::B(arr) => arr.to_vec(),
        });

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

                let body_bytes = body.as_deref();

                // Collect the whole fall-through chain in this one pass. Only a
                // handler can fall through, so the walk stops at the first
                // declarative candidate — nothing after it is ever reachable,
                // and generating responses for candidates that never run would
                // charge every single-match request for the rare deep chain.
                let mut exclude: Vec<String> = exclude_ids.clone().unwrap_or_default();
                let mut candidates: Vec<Candidate> = Vec::new();

                loop {
                    let Some(mock_match) = matcher.find_match_excluding(
                        &http_method,
                        &path,
                        query.as_deref(),
                        &header_map,
                        body_bytes,
                        &exclude,
                    ) else {
                        break;
                    };

                    let mock_def = &mock_match.mock;
                    let captures = mock_match.captures;

                    if matches!(&mock_def.response.body, BodySource::Handler(_)) {
                        let mut context = RequestContext::from_request_for_handler(
                            method.as_str(),
                            &path,
                            query.as_deref(),
                            &header_map,
                            body_bytes,
                        );
                        context.captures = captures;
                        exclude.push(mock_def.id.to_string());
                        candidates.push(Candidate::Handler(Box::new(HandlerCandidate {
                            mock_id: mock_def.id.to_string(),
                            status: mock_def.response.status,
                            def_headers: mock_def.response.headers.clone(),
                            context,
                            kind: if mock_def.request.graphql_matcher.is_some() {
                                HandlerKind::GraphQL
                            } else {
                                HandlerKind::Http
                            },
                            once: mock_def.once,
                        })));
                        continue;
                    }

                    // Declarative — generate on tokio and end the chain.
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
                        .map_err(|e| {
                            Error::from_reason(format!("Response generation failed: {e}"))
                        })?;

                    candidates.push(Candidate::Declarative(Box::new(build_matched_response(
                        &mock_def.id,
                        mock_def.response.status,
                        &mock_def.response.headers,
                        dynamic,
                    ))));
                    break;
                }

                if candidates.is_empty() {
                    return Ok(MatchPhaseResult::NoMatch);
                }

                Ok(MatchPhaseResult::Chain {
                    candidates,
                    request_id,
                })
            },
            // === Phase 2: Deferred resolver on JS thread ===
            move |env, result| -> Result<MaybePromise> {
                match result {
                    MatchPhaseResult::NoMatch => MaybePromise::resolved(env, None),
                    MatchPhaseResult::Chain {
                        candidates,
                        request_id,
                    } => run_chain(
                        env,
                        candidates,
                        0,
                        &handler_refs,
                        &resolver_matcher,
                        request_id.as_deref(),
                    ),
                }
            },
        )
    }
}

/// Walk the fall-through chain on the JS thread.
///
/// The loop lives here rather than in the JS interceptor because every hop
/// back to JS costs a full `matchRequest` round trip — NAPI marshalling plus a
/// tokio spawn — while the matching it repeats costs under 100ns. Keeping the
/// walk on this side turns a chain of any depth into one crossing.
///
/// Recurses through `then` for async handlers: a returned promise flattens per
/// the JS spec, so the caller still sees a single promise regardless of how
/// many candidates were tried or which of them were async.
fn run_chain(
    env: &Env,
    mut candidates: Vec<Candidate>,
    start: usize,
    handler_refs: &Arc<std::sync::RwLock<HashMap<String, Arc<HandlerFnRef>>>>,
    matcher: &MockMatcher,
    request_id: Option<&str>,
) -> Result<MaybePromise> {
    let mut index = start;

    while index < candidates.len() {
        let candidate = std::mem::replace(&mut candidates[index], Candidate::Consumed);

        let handler = match candidate {
            Candidate::Declarative(resp) => return MaybePromise::resolved(env, Some(*resp)),
            Candidate::Consumed => {
                return Err(Error::from_reason("fall-through candidate visited twice"));
            }
            Candidate::Handler(handler) => *handler,
        };
        let HandlerCandidate {
            mock_id,
            status: default_status,
            def_headers,
            context,
            kind,
            once,
        } = handler;

        let raw_result = {
            let refs = handler_refs
                .read()
                .map_err(|e| Error::from_reason(e.to_string()))?;
            let fn_ref = refs.get(&mock_id).ok_or_else(|| {
                Error::from_reason(format!("No FunctionRef for handler: {mock_id}"))
            })?;
            // Direct napi_call_function via FunctionRef — ~1us vs ~22us TSFN
            let func = fn_ref.borrow_back(env)?;
            let req = ResolverArg::new(kind, context, request_id.map(ToString::to_string));
            let raw: Unknown = func.call(req)?;
            raw
        };

        let mut is_promise = false;
        #[allow(unsafe_code)]
        unsafe {
            napi::sys::napi_is_promise(env.raw(), raw_result.raw(), &mut is_promise);
        }

        if is_promise {
            // Async handler: the rest of the chain has to continue inside the
            // continuation, since the fall-through verdict is not known yet.
            #[allow(unsafe_code)]
            let promise_raw: PromiseRaw<'_, Option<HandlerResponse>> =
                unsafe { FromNapiValue::from_napi_value(env.raw(), raw_result.raw())? };

            let handler_refs = Arc::clone(handler_refs);
            let matcher = matcher.clone();
            let request_id = request_id.map(ToString::to_string);
            let next = index + 1;

            let chained = promise_raw.then(move |ctx| match ctx.value {
                Some(js_resp) => Ok(MaybePromise::resolved(
                    &ctx.env,
                    Some(build_matched_response(
                        &mock_id,
                        default_status,
                        &def_headers,
                        js_resp.into(),
                    )),
                )?),
                None => {
                    // MSW does not count a handler as used when it falls
                    // through, so a consumed `once` is given back.
                    if once {
                        matcher.reenable_mock(&mock_id);
                    }
                    if next < candidates.len() {
                        run_chain(
                            &ctx.env,
                            candidates,
                            next,
                            &handler_refs,
                            &matcher,
                            request_id.as_deref(),
                        )
                    } else {
                        MaybePromise::resolved(&ctx.env, None)
                    }
                }
            })?;

            return Ok(MaybePromise(chained.value().value));
        }

        // Sync handler — extract directly, no Promise overhead.
        #[allow(unsafe_code)]
        let resp: Option<HandlerResponse> =
            unsafe { FromNapiValue::from_napi_value(env.raw(), raw_result.raw())? };

        match resp {
            Some(js_resp) => {
                return MaybePromise::resolved(
                    env,
                    Some(build_matched_response(
                        &mock_id,
                        default_status,
                        &def_headers,
                        js_resp.into(),
                    )),
                );
            }
            None => {
                if once {
                    matcher.reenable_mock(&mock_id);
                }
                index += 1;
            }
        }
    }

    // Every candidate fell through: unhandled, exactly as if nothing matched.
    MaybePromise::resolved(env, None)
}

/// Handler info returned by `listHandlers()`.
#[napi(object)]
pub struct HandlerInfo {
    pub id: String,
    pub methods: Vec<String>,
    pub enabled: bool,
    /// `"websocket"` for WebSocket mocks (MSW's handler tag), absent otherwise.
    pub kind: Option<String>,
    /// The predicate as the user wrote it (`/users/:id`, a full URL, a
    /// RegExp display form, or a single exact URL for declarative mocks).
    pub pattern: Option<String>,
    /// MSW's `info.header` display ("GET /users/:id",
    /// "query GetUser (origin: *)").
    pub header: Option<String>,
    /// Requests this handler has served since the last reset.
    pub match_count: u32,
}

/// One mock's match count.
#[napi(object)]
pub struct MockMatchCount {
    pub mock_id: String,
    pub count: u32,
}

/// One WebSocket mock matched by `matchWsConnections`.
#[napi(object)]
pub struct WsConnectionMatch {
    pub mock_id: String,
    pub params: HashMap<String, Either<String, Vec<String>>>,
}

/// Result of matching a request against the mock registry.
///
/// `body` is raw bytes (`Uint8Array`) so binary responses (images, protobuf,
/// gzip) round-trip losslessly. Build a `Response` directly from it; decode with
/// `TextDecoder` when a string is needed.
#[napi(object)]
pub struct MatchedResponse {
    pub status: u32,
    /// Custom status text from the handler (Node interceptor applies it).
    pub status_text: Option<String>,
    pub headers: HashMap<String, String>,
    pub body: Uint8Array,
    pub mock_id: String,
    /// Set when a handler returned null/undefined: re-match with this
    /// mock's ID excluded (MSW fall-through).
    /// Legacy wire field. The native resolver now walks the whole fall-through
    /// chain itself, so a caller never sees a fall-through marker and this is
    /// always absent.
    pub fallthrough: Option<bool>,
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
// Short-lived stack value moved once per request into the resolver; boxing the
// large variant would add a hot-path allocation for no real benefit.
#[allow(clippy::large_enum_variant)]
enum MatchPhaseResult {
    NoMatch,
    /// Every candidate for this request, in match order. Handlers may fall
    /// through to the next; a declarative response cannot, so it is always
    /// last when present.
    Chain {
        candidates: Vec<Candidate>,
        request_id: Option<String>,
    },
}

/// A handler candidate's payload, boxed so the chain vector stays small.
struct HandlerCandidate {
    mock_id: String,
    status: http::StatusCode,
    def_headers: FxHashMap<String, String>,
    context: RequestContext,
    kind: HandlerKind,
    once: bool,
}

/// One link in the fall-through chain. Both payloads are boxed: the common
/// chain is a single element, and the vector is moved into an async
/// continuation on every hop.
enum Candidate {
    Handler(Box<HandlerCandidate>),
    Declarative(Box<MatchedResponse>),
    /// Left behind when a candidate is taken out of the chain, so a bug that
    /// revisits one is reported instead of silently re-running a handler.
    Consumed,
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
    // Raw bytes — no UTF-8 round-trip, binary-safe.
    let body = Uint8Array::from(dynamic.body.to_vec());
    MatchedResponse {
        status: u32::from(status),
        status_text: dynamic.status_text,
        headers,
        body,
        mock_id: mock_id.to_string(),
        fallthrough: None,
    }
}

// -- Internal server implementation --

#[derive(Clone)]
struct ServerState {
    matcher: MockMatcher,
}

/// Catch-all handler — delegates to the canonical
/// `services::serve::handle_request` so the standalone server and the
/// CLI share one mock implementation (including WS upgrades and SSE).
async fn mock_handler(
    axum::extract::State(state): axum::extract::State<Arc<ServerState>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    ferrimock::services::serve::handle_request(&state.matcher, req).await
}
