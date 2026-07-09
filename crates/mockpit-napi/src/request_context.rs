//! Resolver info classes with on-demand getters instead of eager object
//! construction.
//!
//! `#[napi(object)]` creates ALL fields upfront as JS values.
//! `#[napi]` class only creates JS values when getters are called.
//! Most handlers only access `params` -- they never touch `headers` or `body`.
//!
//! `RequestInfo` is the HTTP resolver info; `GraphQLRequestInfo` is the
//! GraphQL resolver info (MSW's `{ query, variables, operationName, ... }`).
//! Both expose a `request` getter that builds a real Fetch API `Request`
//! (the global constructor), so MSW handlers destructuring
//! `{ request }` get the standard object.

use mockpit::types::RequestContext;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// Global request counter for generating unique request IDs.
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// URL captures as an MSW params object: repeatable params (`:name+` /
/// `:name*`) become `string[]`, all values percent-decoded.
pub(crate) fn msw_params_map(
    captures: &rustc_hash::FxHashMap<String, String>,
) -> HashMap<String, Either<String, Vec<String>>> {
    mockpit::types::msw_params(captures)
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                match v {
                    mockpit::types::MswParamValue::Single(s) => Either::A(s),
                    mockpit::types::MswParamValue::List(l) => Either::B(l),
                },
            )
        })
        .collect()
}

/// Shared per-request data: the context plus a body-JSON parse cache so
/// `bodyJson` parses at most once regardless of how many views read it.
pub(crate) struct RequestData {
    ctx: RequestContext,
    body_json: OnceLock<Option<serde_json::Value>>,
}

impl RequestData {
    fn body_json(&self) -> Option<&serde_json::Value> {
        self.body_json
            .get_or_init(|| {
                // Prefer an upstream parse when present (declarative paths);
                // handler contexts skip it and parse lazily here.
                if self.ctx.body_json.is_some() {
                    return self.ctx.body_json.clone();
                }
                self.ctx
                    .body
                    .as_deref()
                    .and_then(|b| serde_json::from_str(b).ok())
            })
            .as_ref()
    }

    /// Absolute URL reconstructed from the Host header, path, and query
    /// (requests carry no scheme by the time they reach the engine; http
    /// is assumed).
    fn url(&self) -> String {
        let host = self
            .ctx
            .headers
            .get("host")
            .map_or("localhost", String::as_str);
        let mut url = format!("http://{host}{}", self.ctx.path);
        if !self.ctx.query.is_empty() {
            let mut first = true;
            for (k, v) in &self.ctx.query {
                url.push(if first { '?' } else { '&' });
                first = false;
                url.push_str(k);
                if !v.is_empty() {
                    url.push('=');
                    url.push_str(v);
                }
            }
        }
        url
    }

    fn cookies(&self) -> HashMap<String, String> {
        self.ctx
            .headers
            .get("cookie")
            .map(|cookie_header| {
                cookie_header
                    .split(';')
                    .filter_map(|pair| {
                        let mut parts = pair.splitn(2, '=');
                        let name = parts.next()?.trim();
                        let value = parts.next()?.trim();
                        if name.is_empty() {
                            None
                        } else {
                            let decoded = urlencoding::decode(value)
                                .map_or_else(|_| value.to_string(), |d| d.into_owned());
                            Some((name.to_string(), decoded))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Request identity: either supplied by the caller (the interceptor
/// passes its lifecycle-event ID so `info.requestId` correlates with
/// `server.events`) or generated lazily on first read.
enum RequestId {
    Provided(String),
    Generated(u64),
}

impl RequestId {
    fn new(provided: Option<String>) -> Self {
        match provided {
            Some(id) => Self::Provided(id),
            None => Self::Generated(REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)),
        }
    }

    fn as_string(&self) -> String {
        match self {
            Self::Provided(id) => id.clone(),
            Self::Generated(n) => format!("req:{n:x}"),
        }
    }
}

/// Build a real Fetch API `Request` from the shared request data by
/// calling the global constructor.
fn build_fetch_request<'env>(env: &'env Env, data: &RequestData) -> Result<Object<'env>> {
    let global = env.get_global()?;
    let request_ctor: Function<'env, FnArgs<(String, Object<'env>)>, Object<'env>> =
        global.get_named_property("Request")?;

    let mut init = Object::new(env)?;
    init.set("method", data.ctx.method.as_str())?;

    let mut headers = Object::new(env)?;
    for (k, v) in &data.ctx.headers {
        headers.set(k.as_str(), v.as_str())?;
    }
    init.set("headers", headers)?;

    let method = data.ctx.method.as_str();
    if method != "GET" && method != "HEAD" {
        if let Some(body) = &data.ctx.body {
            init.set("body", body.as_str())?;
        } else if let Some(bytes) = &data.ctx.body_bytes {
            // Non-UTF8 body: pass exact bytes so binary request bodies
            // (protobuf, multipart with files) survive the crossing.
            init.set(
                "body",
                napi::bindgen_prelude::Uint8Array::from(bytes.to_vec()),
            )?;
        }
    }

    let instance = request_ctor.new_instance(FnArgs {
        data: (data.url(), init),
    })?;
    instance.coerce_to_object()
}

/// HTTP resolver info: MSW's `{ request, params, cookies, requestId }`.
///
/// Fields are lazily converted to JS values on first access.
#[napi]
pub struct RequestInfo {
    inner: Arc<RequestData>,
    request_id: RequestId,
}

#[napi]
impl RequestInfo {
    pub(crate) fn new(ctx: RequestContext, request_id: Option<String>) -> Self {
        Self {
            inner: Arc::new(RequestData {
                ctx,
                body_json: OnceLock::new(),
            }),
            request_id: RequestId::new(request_id),
        }
    }

    /// Unique request identifier.
    #[napi(getter)]
    pub fn request_id(&self) -> String {
        self.request_id.as_string()
    }

    /// A real Fetch API `Request` for this request (MSW's `info.request`).
    /// Built on access; destructure it once per handler call.
    #[napi(getter, ts_return_type = "Request")]
    pub fn request<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        build_fetch_request(env, &self.inner)
    }

    /// Path parameters captured from the URL pattern. Repeatable params
    /// (`:name+` / `:name*`) surface as arrays (MSW semantics).
    #[napi(getter)]
    pub fn params(&self) -> HashMap<String, Either<String, Vec<String>>> {
        msw_params_map(&self.inner.ctx.captures)
    }

    /// Parsed cookies from the Cookie request header.
    #[napi(getter)]
    pub fn cookies(&self) -> HashMap<String, String> {
        self.inner.cookies()
    }
}

/// GraphQL resolver info (MSW's `{ query, variables, operationName,
/// cookies, request, requestId }`).
#[napi]
pub struct GraphQLRequestInfo {
    inner: Arc<RequestData>,
    request_id: RequestId,
}

#[napi]
impl GraphQLRequestInfo {
    pub(crate) fn new(ctx: RequestContext, request_id: Option<String>) -> Self {
        Self {
            inner: Arc::new(RequestData {
                ctx,
                body_json: OnceLock::new(),
            }),
            request_id: RequestId::new(request_id),
        }
    }

    /// Unique request identifier.
    #[napi(getter)]
    pub fn request_id(&self) -> String {
        self.request_id.as_string()
    }

    /// The GraphQL document string.
    #[napi(getter)]
    pub fn query(&self) -> Option<String> {
        self.inner
            .body_json()?
            .get("query")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }

    #[napi(getter)]
    pub fn variables(&self) -> serde_json::Value {
        self.inner
            .body_json()
            .and_then(|v| v.get("variables"))
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()))
    }

    /// Explicit operationName field, falling back to the name declared in
    /// the query document.
    #[napi(getter)]
    pub fn operation_name(&self) -> Option<String> {
        let body = self.inner.body_json()?;
        if let Some(name) = body.get("operationName").and_then(|v| v.as_str()) {
            return Some(name.to_string());
        }
        let doc = body.get("query").and_then(|v| v.as_str())?;
        mockpit::engine::MockMatcher::operation_name_from_query(doc).map(str::to_string)
    }

    /// A real Fetch API `Request` for this request (MSW's `info.request`).
    #[napi(getter, ts_return_type = "Request")]
    pub fn request<'env>(&self, env: &'env Env) -> Result<Object<'env>> {
        build_fetch_request(env, &self.inner)
    }

    /// Parsed cookies from the Cookie request header.
    #[napi(getter)]
    pub fn cookies(&self) -> HashMap<String, String> {
        self.inner.cookies()
    }
}

/// Which resolver info class the handler receives.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    Http,
    GraphQL,
}

/// Type-erased resolver argument: converts to the class matching the
/// handler's kind when crossing into JS.
pub enum ResolverArg {
    Http(RequestInfo),
    GraphQL(GraphQLRequestInfo),
}

impl ResolverArg {
    pub fn new(kind: HandlerKind, ctx: RequestContext, request_id: Option<String>) -> Self {
        match kind {
            HandlerKind::Http => Self::Http(RequestInfo::new(ctx, request_id)),
            HandlerKind::GraphQL => Self::GraphQL(GraphQLRequestInfo::new(ctx, request_id)),
        }
    }
}

impl ToNapiValue for ResolverArg {
    #[allow(unsafe_code)]
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        // SAFETY: delegates to the napi-generated class conversions.
        unsafe {
            match val {
                ResolverArg::Http(req) => RequestInfo::to_napi_value(env, req),
                ResolverArg::GraphQL(info) => GraphQLRequestInfo::to_napi_value(env, info),
            }
        }
    }
}
