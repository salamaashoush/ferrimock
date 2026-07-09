//! One [`ScriptEngine`] per script file.
//!
//! Per-file isolation makes hot reload and poison recovery trivial:
//! reloading (or recovering) a file drops its engine — the VM loop ends,
//! every `Persistent` handler frees with the runtime, and a fresh engine
//! re-evaluates just that file. No stale module cache, no cross-file
//! interference. Script state (`let counter = 0` at module scope) resets
//! on reload, matching dev-server restart semantics.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;

use crate::types::MockDefinition;
use crate::{MockpitError, Result, handler};

use super::bridge::{HandlerKind, build_handler_fn};
use super::bundle::CompiledBundle;
use super::engine::{ScriptEngine, ScriptEngineConfig};
use super::loader::evaluate_mock_module;
use super::slots::{
    ScriptGraphQLName, ScriptGraphQLOp, ScriptMockKind, ScriptMockSpec, ScriptPath,
};

struct LoadedScript {
    /// Never read — held so the VM loop (and every `Persistent` handler)
    /// lives exactly as long as this file's registry entry.
    _engine: Arc<ScriptEngine>,
    root: PathBuf,
}

/// Owns the script engines behind a registry's `.js`/`.mjs` mocks.
#[derive(Default)]
pub struct ScriptHost {
    config: parking_lot::RwLock<ScriptEngineConfig>,
    scripts: DashMap<PathBuf, LoadedScript>,
}

impl ScriptHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the engine config used for subsequently (re)loaded files.
    pub fn set_config(&self, config: ScriptEngineConfig) {
        *self.config.write() = config;
    }

    /// Evaluate a script file on a fresh engine and return its mock
    /// definitions. Reloading an already-loaded path drops the previous
    /// engine first. `root` bounds what the file may import; pass the
    /// mocks directory (falls back to the file's parent for single-file
    /// loads).
    pub async fn load_file(&self, path: &Path, root: Option<&Path>) -> Result<Vec<MockDefinition>> {
        let canonical = path
            .canonicalize()
            .map_err(|e| MockpitError::Script(format!("{}: {e}", path.display())))?;
        let root = match root {
            Some(r) => r
                .canonicalize()
                .map_err(|e| MockpitError::Script(format!("{}: {e}", r.display())))?,
            None => canonical
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| MockpitError::Script("script file has no parent dir".to_string()))?,
        };

        self.scripts.remove(&canonical);

        let config = self.config.read().clone();
        let engine = Arc::new(ScriptEngine::new(config).await?);
        let (specs, bundle) = evaluate_mock_module(&engine, &canonical, &root).await?;
        let bundle = Arc::new(bundle);
        if specs.is_empty() {
            tracing::warn!(
                target: "mockpit::script",
                "{} evaluated but registered no handlers (missing http.*/graphql.* calls?)",
                path.display()
            );
        }

        let source_file = path.to_string_lossy().into_owned();
        let defs = specs
            .into_iter()
            .map(|spec| build_mock(&engine, &bundle, spec, &source_file))
            .collect::<Result<Vec<_>>>()?;

        self.scripts.insert(
            canonical,
            LoadedScript {
                _engine: engine,
                root,
            },
        );
        Ok(defs)
    }

    /// Reload a previously loaded file with its recorded sandbox root.
    pub async fn reload_file(&self, path: &Path) -> Result<Vec<MockDefinition>> {
        let root = path
            .canonicalize()
            .ok()
            .and_then(|c| self.scripts.get(&c).map(|s| s.root.clone()));
        self.load_file(path, root.as_deref()).await
    }

    /// Drop the engine for a removed file (frees its handlers).
    pub fn unload_file(&self, path: &Path) {
        if let Ok(canonical) = path.canonicalize() {
            self.scripts.remove(&canonical);
        } else {
            // Deleted files can no longer canonicalize; match by suffix.
            self.scripts
                .retain(|loaded, _| !loaded.ends_with(path) && loaded != path);
        }
    }
}

/// Placeholder body for streaming mocks: the serve layer branches on
/// `streaming` before response generation, so this only runs if a
/// streaming mock is invoked through a non-streaming code path.
fn streaming_stub(kind: &'static str) -> crate::types::HandlerFn {
    Arc::new(move |_ctx| {
        Box::pin(async move {
            Err(MockpitError::Script(format!(
                "{kind} mock invoked as a plain HTTP handler"
            )))
        })
    })
}

fn build_mock(
    engine: &Arc<ScriptEngine>,
    bundle: &Arc<CompiledBundle>,
    spec: ScriptMockSpec,
    source_file: &str,
) -> Result<MockDefinition> {
    match spec.kind {
        ScriptMockKind::Sse { path } => {
            // Absolute http(s) paths double as the real endpoint
            // `server.connect()` dials for passthrough.
            let upstream_url = match &path {
                ScriptPath::Pattern(p) if p.starts_with("http://") || p.starts_with("https://") => {
                    Some(p.clone())
                }
                _ => None,
            };
            let handler = super::bridge_streaming::build_sse_handler_fn(
                engine.vm().clone(),
                spec.slot,
                engine.poisoned_flag(),
                Arc::clone(bundle),
                upstream_url,
            );
            let (pattern, regex) = match path {
                ScriptPath::Pattern(p) => (p, None),
                ScriptPath::Regex { source, flags } => {
                    let re = compile_js_regex(&source, &flags).map_err(|e| {
                        MockpitError::Script(format!(
                            "{source_file}: invalid RegExp path /{source}/{flags}: {e}"
                        ))
                    })?;
                    ("*".to_string(), Some(re))
                }
            };
            let mut mock = handler::http::get(&pattern, streaming_stub("sse"));
            if let Some(re) = regex {
                mock.request.url_patterns =
                    smallvec::SmallVec::from_elem(crate::types::UrlPattern::Regex(re), 1);
            }
            mock.streaming = Some(crate::types::StreamingResponse::SseHandler(handler));
            mock.once = spec.once;
            mock.source_file = Some(source_file.to_string());
            return Ok(mock);
        }
        ScriptMockKind::Ws { ref path } => {
            // Absolute ws:// URLs get a Host matcher via the http handler
            // builder's absolute-URL splitting; the link URL is kept for
            // server.connect() passthrough. RegExp links match the bare
            // path plus ws(s)://host/path reconstructions (MSW's href
            // idiom) and have no dialable upstream.
            let (pattern, href_regex, upstream_url) = match path {
                ScriptPath::Pattern(url) => {
                    let absolute = url.starts_with("ws://") || url.starts_with("wss://");
                    let pattern = url
                        .replacen("ws://", "http://", 1)
                        .replacen("wss://", "https://", 1);
                    (pattern, None, absolute.then(|| url.clone()))
                }
                ScriptPath::Regex { source, flags } => {
                    let re = compile_js_regex(source, flags).map_err(|e| {
                        MockpitError::Script(format!(
                            "{source_file}: invalid RegExp link /{source}/{flags}: {e}"
                        ))
                    })?;
                    ("*".to_string(), Some(re), None)
                }
            };
            let handler = super::bridge_streaming::build_ws_handler_fn(
                engine.vm().clone(),
                spec.slot,
                engine.poisoned_flag(),
                Arc::clone(bundle),
                upstream_url,
            );
            let mut mock = handler::http::get(&pattern, streaming_stub("ws"));
            if let Some(re) = href_regex {
                mock.request.url_patterns =
                    smallvec::SmallVec::from_elem(crate::types::UrlPattern::HrefRegex(re), 1);
            }
            let upgrade =
                crate::types::HeaderMatcher::regex(http::header::UPGRADE, "(?i)^websocket$")
                    .map_err(|e| MockpitError::Script(format!("upgrade matcher: {e}")))?;
            mock.request.header_matchers.push(upgrade);
            mock.streaming = Some(crate::types::StreamingResponse::WsHandler(handler));
            mock.source_file = Some(source_file.to_string());
            return Ok(mock);
        }
        _ => {}
    }

    let kind = match spec.kind {
        ScriptMockKind::Http { .. } => HandlerKind::Http,
        ScriptMockKind::GraphQL { .. } => HandlerKind::GraphQL,
        ScriptMockKind::Sse { .. } | ScriptMockKind::Ws { .. } => {
            return Err(MockpitError::Script(
                "streaming spec in http/graphql path".to_string(),
            ));
        }
    };
    let handler_fn = build_handler_fn(
        engine.vm().clone(),
        spec.slot,
        Arc::clone(engine.timeout_state()),
        engine.poisoned_flag(),
        engine.config().handler_timeout,
        Arc::clone(bundle),
        kind,
        spec.is_generator,
    );

    let mut mock = match spec.kind {
        ScriptMockKind::Http { method, path } => {
            let (pattern, regex) = match path {
                ScriptPath::Pattern(p) => (p, None),
                ScriptPath::Regex { source, flags } => {
                    let re = compile_js_regex(&source, &flags).map_err(|e| {
                        MockpitError::Script(format!(
                            "{source_file}: invalid RegExp path /{source}/{flags}: {e}"
                        ))
                    })?;
                    ("*".to_string(), Some(re))
                }
            };
            let mut mock = match method.as_deref() {
                Some("GET") => handler::http::get(&pattern, handler_fn),
                Some("POST") => handler::http::post(&pattern, handler_fn),
                Some("PUT") => handler::http::put(&pattern, handler_fn),
                Some("DELETE") => handler::http::delete(&pattern, handler_fn),
                Some("PATCH") => handler::http::patch(&pattern, handler_fn),
                Some("HEAD") => handler::http::head(&pattern, handler_fn),
                Some("OPTIONS") => handler::http::options(&pattern, handler_fn),
                _ => handler::http::all(&pattern, handler_fn),
            };
            if let Some(re) = regex {
                mock.request.url_patterns =
                    smallvec::SmallVec::from_elem(crate::types::UrlPattern::Regex(re), 1);
            }
            mock
        }
        ScriptMockKind::Sse { .. } | ScriptMockKind::Ws { .. } => {
            return Err(MockpitError::Script(
                "streaming spec in http/graphql path".to_string(),
            ));
        }
        ScriptMockKind::GraphQL {
            operation_type,
            operation_name,
            endpoint,
        } => {
            let mut mock = match (operation_type, operation_name) {
                (ScriptGraphQLOp::Query, Some(ScriptGraphQLName::Exact(name))) => {
                    handler::graphql::query(&name, handler_fn)
                }
                (ScriptGraphQLOp::Mutation, Some(ScriptGraphQLName::Exact(name))) => {
                    handler::graphql::mutation(&name, handler_fn)
                }
                (ScriptGraphQLOp::Query, Some(ScriptGraphQLName::Regex { source, flags })) => {
                    let re = compile_js_regex(&source, &flags).map_err(|e| {
                        MockpitError::Script(format!(
                            "{source_file}: invalid RegExp operation name /{source}/{flags}: {e}"
                        ))
                    })?;
                    handler::graphql::query_regex(re, handler_fn)
                }
                (ScriptGraphQLOp::Mutation, Some(ScriptGraphQLName::Regex { source, flags })) => {
                    let re = compile_js_regex(&source, &flags).map_err(|e| {
                        MockpitError::Script(format!(
                            "{source_file}: invalid RegExp operation name /{source}/{flags}: {e}"
                        ))
                    })?;
                    handler::graphql::mutation_regex(re, handler_fn)
                }
                _ => handler::graphql::operation(handler_fn),
            };
            if let Some(endpoint) = endpoint {
                apply_graphql_endpoint(&mut mock, &endpoint);
            }
            mock
        }
    };
    mock.once = spec.once;
    mock.source_file = Some(source_file.to_string());
    Ok(mock)
}

/// Scope a `graphql.link` mock to its endpoint: exact path pattern plus a
/// Host-header matcher for absolute URLs.
fn apply_graphql_endpoint(mock: &mut MockDefinition, endpoint: &str) {
    use crate::types::{HeaderMatcher, UrlPattern};

    let (host, path) = match UrlPattern::split_absolute_url(endpoint) {
        Some((host, path)) => (Some(host), path),
        None => (None, endpoint),
    };
    mock.request.url_patterns = smallvec::SmallVec::from_elem(UrlPattern::exact(path), 1);
    if let Some(host) = host {
        mock.request.header_matchers =
            smallvec::SmallVec::from_elem(HeaderMatcher::exact(http::header::HOST, host), 1);
    }
}

/// Compile a JS RegExp source with its matching-relevant flags (`i`,
/// `m`, `s`) mapped to the regex crate's inline flags.
fn compile_js_regex(source: &str, flags: &str) -> Result<regex::Regex, regex::Error> {
    if flags.is_empty() {
        regex::Regex::new(source)
    } else {
        regex::Regex::new(&format!("(?{flags}){source}"))
    }
}
