//! Resolver info objects passed to scripted handlers (MSW shapes).
//!
//! [`RequestInfo`] is the HTTP resolver info: `{ request, params,
//! cookies, requestId }`. [`GraphQLRequestInfo`] is the GraphQL resolver
//! info: `{ query, variables, operationName, cookies, request,
//! requestId }`. `request` is a Fetch-shaped [`Request`] view with
//! case-insensitive [`Headers`]. Everything materializes lazily — most
//! handlers only touch `params`.

// rquickjs method targets must take FromJs params owned and the
// macro-injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use rquickjs::{Class, Ctx, Exception, JsLifetime, Object, Persistent, Value, class::Trace};
use rustc_hash::FxHashMap;

use crate::types::RequestContext;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn cookies_to_js<'js>(ctx: &Ctx<'js>, inner: &RequestContext) -> rquickjs::Result<Value<'js>> {
    let obj = Object::new(ctx.clone())?;
    if let Some(cookie_header) = inner.headers.get("cookie") {
        for pair in cookie_header.split(';') {
            let mut parts = pair.splitn(2, '=');
            let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
                continue;
            };
            let name = name.trim();
            if !name.is_empty() {
                let value = value.trim();
                let decoded = urlencoding::decode(value)
                    .map_or_else(|_| value.to_string(), std::borrow::Cow::into_owned);
                obj.set(name, decoded)?;
            }
        }
    }
    Ok(obj.into_value())
}

/// Absolute URL reconstructed from the Host header, path, and query
/// (requests carry no scheme by the time they reach the engine; http is
/// assumed).
fn build_url(inner: &RequestContext) -> String {
    let host = inner
        .headers
        .get("host")
        .map_or("localhost", String::as_str);
    let mut url = format!("http://{host}{}", inner.path);
    if !inner.query.is_empty() {
        let mut first = true;
        for (k, v) in &inner.query {
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

/// Parse the body as JSON with QuickJS's C parser, caching the resulting
/// JS value so repeated `json()` calls parse once.
fn parsed_body_json<'js>(
    ctx: &Ctx<'js>,
    inner: &RequestContext,
    cache: &RefCell<Option<Persistent<Value<'static>>>>,
) -> rquickjs::Result<Value<'js>> {
    if let Some(cached) = cache.borrow().as_ref() {
        return cached.clone().restore(ctx);
    }
    let parsed = match &inner.body {
        Some(body) if !body.is_empty() => ctx
            .json_parse(body.as_str())
            .unwrap_or_else(|_| Value::new_null(ctx.clone())),
        _ => Value::new_null(ctx.clone()),
    };
    *cache.borrow_mut() = Some(Persistent::save(ctx, parsed.clone()));
    Ok(parsed)
}

fn cached_instance<'js, F>(
    ctx: &Ctx<'js>,
    cache: &RefCell<Option<Persistent<Value<'static>>>>,
    create: F,
) -> rquickjs::Result<Value<'js>>
where
    F: FnOnce(&Ctx<'js>) -> rquickjs::Result<Value<'js>>,
{
    if let Some(cached) = cache.borrow().as_ref() {
        return cached.clone().restore(ctx);
    }
    let value = create(ctx)?;
    *cache.borrow_mut() = Some(Persistent::save(ctx, value.clone()));
    Ok(value)
}

fn fetch_request_view<'js>(
    ctx: &Ctx<'js>,
    inner: &Rc<RequestContext>,
    cache: &RefCell<Option<Persistent<Value<'static>>>>,
) -> rquickjs::Result<Value<'js>> {
    let inner = Rc::clone(inner);
    cached_instance(ctx, cache, move |ctx| {
        Ok(Class::instance(ctx.clone(), Request::new(inner))?
            .as_value()
            .clone())
    })
}

/// HTTP resolver info: MSW's `{ request, params, cookies, requestId }`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "RequestInfo")]
pub struct RequestInfo {
    #[qjs(skip_trace)]
    inner: Rc<RequestContext>,
    #[qjs(skip_trace)]
    request_id: u64,
    #[qjs(skip_trace)]
    request_cache: RefCell<Option<Persistent<Value<'static>>>>,
}

impl RequestInfo {
    pub fn new(inner: RequestContext) -> Self {
        Self {
            inner: Rc::new(inner),
            request_id: REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed),
            request_cache: RefCell::new(None),
        }
    }
}

#[rquickjs::methods]
impl RequestInfo {
    #[qjs(get, rename = "requestId")]
    pub fn request_id(&self) -> String {
        format!("req:{:x}", self.request_id)
    }

    /// Fetch Request view (`info.request`), created on first access and
    /// cached for the rest of the handler call.
    #[qjs(get)]
    pub fn request<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        fetch_request_view(&ctx, &self.inner, &self.request_cache)
    }

    /// Path parameters captured from the URL pattern. Repeatable params
    /// (`:name+` / `:name*`) surface as arrays (MSW semantics).
    #[qjs(get)]
    pub fn params<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let obj = Object::new(ctx.clone())?;
        for (k, v) in crate::types::msw_params(&self.inner.captures) {
            match v {
                crate::types::MswParamValue::Single(s) => obj.set(k, s)?,
                crate::types::MswParamValue::List(l) => obj.set(k, l)?,
            }
        }
        Ok(obj.into_value())
    }

    /// Parsed cookies from the Cookie request header.
    #[qjs(get)]
    pub fn cookies<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        cookies_to_js(&ctx, &self.inner)
    }
}

/// GraphQL resolver info: MSW's `{ query, variables, operationName,
/// cookies, request, requestId }`.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "GraphQLRequestInfo")]
pub struct GraphQLRequestInfo {
    #[qjs(skip_trace)]
    inner: Rc<RequestContext>,
    #[qjs(skip_trace)]
    request_id: u64,
    #[qjs(skip_trace)]
    body_json_cache: RefCell<Option<Persistent<Value<'static>>>>,
    #[qjs(skip_trace)]
    request_cache: RefCell<Option<Persistent<Value<'static>>>>,
}

impl GraphQLRequestInfo {
    pub fn new(inner: RequestContext) -> Self {
        Self {
            inner: Rc::new(inner),
            request_id: REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed),
            body_json_cache: RefCell::new(None),
            request_cache: RefCell::new(None),
        }
    }

    fn body_field<'js>(&self, ctx: &Ctx<'js>, field: &str) -> rquickjs::Result<Value<'js>> {
        let parsed = parsed_body_json(ctx, &self.inner, &self.body_json_cache)?;
        match parsed.as_object() {
            Some(obj) => obj.get(field),
            None => Ok(Value::new_undefined(ctx.clone())),
        }
    }
}

#[rquickjs::methods]
impl GraphQLRequestInfo {
    #[qjs(get, rename = "requestId")]
    pub fn request_id(&self) -> String {
        format!("req:{:x}", self.request_id)
    }

    /// The GraphQL document string.
    #[qjs(get)]
    pub fn query<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        self.body_field(&ctx, "query")
    }

    #[qjs(get)]
    pub fn variables<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let value = self.body_field(&ctx, "variables")?;
        if value.is_undefined() || value.is_null() {
            return Ok(Object::new(ctx)?.into_value());
        }
        Ok(value)
    }

    /// Explicit operationName field, falling back to the name declared in
    /// the query document.
    #[qjs(get, rename = "operationName")]
    pub fn operation_name<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let explicit = self.body_field(&ctx, "operationName")?;
        if !explicit.is_undefined() && !explicit.is_null() {
            return Ok(explicit);
        }
        let doc = self.body_field(&ctx, "query")?;
        let name = doc.as_string().and_then(|s| {
            let doc = s.to_string().ok()?;
            crate::engine::MockMatcher::operation_name_from_query(&doc).map(str::to_string)
        });
        match name {
            Some(name) => Ok(rquickjs::String::from_str(ctx, &name)?.into_value()),
            None => Ok(Value::new_undefined(ctx)),
        }
    }

    #[qjs(get)]
    pub fn cookies<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        cookies_to_js(&ctx, &self.inner)
    }

    #[qjs(get)]
    pub fn request<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        fetch_request_view(&ctx, &self.inner, &self.request_cache)
    }
}

/// Fetch-Request-shaped view of the request (`info.request`).
///
/// `json()`/`text()`/`arrayBuffer()` return plain values rather than
/// Promises — `await` on them behaves identically, and the body is
/// already buffered.
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Request")]
pub struct Request {
    #[qjs(skip_trace)]
    inner: Rc<RequestContext>,
    #[qjs(skip_trace)]
    body_json_cache: RefCell<Option<Persistent<Value<'static>>>>,
    #[qjs(skip_trace)]
    headers_cache: RefCell<Option<Persistent<Value<'static>>>>,
}

impl Request {
    fn new(inner: Rc<RequestContext>) -> Self {
        Self {
            inner,
            body_json_cache: RefCell::new(None),
            headers_cache: RefCell::new(None),
        }
    }
}

#[rquickjs::methods]
impl Request {
    #[qjs(get)]
    pub fn url(&self) -> String {
        build_url(&self.inner)
    }

    #[qjs(get)]
    pub fn method(&self) -> String {
        self.inner.method.clone()
    }

    #[qjs(get)]
    pub fn headers<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let inner = Rc::clone(&self.inner);
        cached_instance(&ctx, &self.headers_cache, move |ctx| {
            Ok(
                Class::instance(ctx.clone(), Headers::from_map(&inner.headers))?
                    .as_value()
                    .clone(),
            )
        })
    }

    /// The raw body stream is not exposed; use `text()`/`json()`.
    // Instance getters require &self even when unused (a self-less fn
    // would become a static on the constructor).
    #[qjs(get)]
    #[allow(clippy::unused_self)]
    pub fn body<'js>(&self, ctx: Ctx<'js>) -> Value<'js> {
        Value::new_null(ctx)
    }

    #[qjs(get, rename = "bodyUsed")]
    #[allow(clippy::unused_self)]
    pub fn body_used(&self) -> bool {
        false
    }

    pub fn json<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let parsed = parsed_body_json(&ctx, &self.inner, &self.body_json_cache)?;
        if parsed.is_null() || parsed.is_undefined() {
            return Err(Exception::throw_type(&ctx, "Failed to parse body as JSON"));
        }
        Ok(parsed)
    }

    pub fn text(&self) -> String {
        match (&self.inner.body, &self.inner.body_bytes) {
            (Some(s), _) => s.clone(),
            (None, Some(b)) => String::from_utf8_lossy(b).into_owned(),
            (None, None) => String::new(),
        }
    }

    #[qjs(rename = "arrayBuffer")]
    pub fn array_buffer<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let bytes = self.inner.body_as_bytes().unwrap_or_default();
        Ok(rquickjs::ArrayBuffer::new_copy(ctx, bytes)?
            .as_value()
            .clone())
    }

    /// Parse the body as multipart/form-data or
    /// application/x-www-form-urlencoded (from the Content-Type header).
    #[qjs(rename = "formData")]
    pub fn form_data<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        let content_type = self
            .inner
            .headers
            .get("content-type")
            .map(String::as_str)
            .unwrap_or_default();
        // body_as_bytes: binary multipart bodies only exist as body_bytes.
        let body = self.inner.body_as_bytes().unwrap_or_default();
        let entries = super::form_data::parse_body(content_type, body)
            .map_err(|e| Exception::throw_type(&ctx, &format!("Failed to parse form data: {e}")))?;
        Ok(
            Class::instance(ctx, super::form_data::FormData::from_entries(entries))?
                .as_value()
                .clone(),
        )
    }

    /// The body is fully buffered, so a clone shares the same data.
    #[qjs(rename = "clone")]
    pub fn clone_js<'js>(&self, ctx: Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        Ok(Class::instance(ctx, Request::new(Rc::clone(&self.inner)))?
            .as_value()
            .clone())
    }
}

/// Case-insensitive mutable Headers (`request.headers`, `new Headers(init)`).
///
/// Stores lowercased name/value pairs in insertion order; iteration
/// follows the Fetch spec (names sorted, values combined with `", "`,
/// except `set-cookie` which stays one entry per value).
#[derive(Trace, JsLifetime)]
#[rquickjs::class(rename = "Headers")]
pub struct Headers {
    #[qjs(skip_trace)]
    entries: RefCell<Vec<(String, String)>>,
}

/// Parse a Headers init value: another `Headers`, an array of `[name,
/// value]` pairs, or a plain object. Names are lowercased.
pub fn headers_from_init<'js>(
    ctx: &Ctx<'js>,
    value: Value<'js>,
) -> rquickjs::Result<Vec<(String, String)>> {
    use rquickjs::convert::Coerced;

    if let Some(obj) = value.as_object()
        && let Some(headers) = Class::<Headers>::from_object(obj)
    {
        return Ok(headers.borrow().snapshot());
    }
    if let Some(arr) = value.as_array() {
        let mut entries = Vec::with_capacity(arr.len());
        for item in arr.iter::<rquickjs::Array<'_>>() {
            let pair = item?;
            if pair.len() != 2 {
                return Err(Exception::throw_type(
                    ctx,
                    "Headers init array entries must be [name, value] pairs",
                ));
            }
            let name: Coerced<String> = pair.get(0)?;
            let value: Coerced<String> = pair.get(1)?;
            entries.push((name.0.to_lowercase(), value.0));
        }
        return Ok(entries);
    }
    if let Some(obj) = value.as_object() {
        let mut entries = Vec::new();
        for entry in obj.props::<String, Coerced<String>>() {
            let (name, value) = entry?;
            entries.push((name.to_lowercase(), value.0));
        }
        return Ok(entries);
    }
    Err(Exception::throw_type(
        ctx,
        "Headers init must be a Headers, an array of pairs, or an object",
    ))
}

impl Headers {
    /// Multi-valued Set-Cookie travels newline-joined through
    /// single-valued header maps; split it back into separate entries.
    pub fn from_map(map: &FxHashMap<String, String>) -> Self {
        let mut entries = Vec::with_capacity(map.len());
        for (name, value) in map {
            let name = name.to_lowercase();
            if name == "set-cookie" {
                for v in value.split('\n') {
                    entries.push((name.clone(), v.to_string()));
                }
            } else {
                entries.push((name, value.clone()));
            }
        }
        Self {
            entries: RefCell::new(entries),
        }
    }

    pub fn from_entries(entries: Vec<(String, String)>) -> Self {
        Self {
            entries: RefCell::new(entries),
        }
    }

    pub fn snapshot(&self) -> Vec<(String, String)> {
        self.entries.borrow().clone()
    }

    /// Fetch-spec iteration order: names sorted, values combined with
    /// `", "`, except `set-cookie` (one entry per value).
    fn combined(&self) -> Vec<(String, String)> {
        let entries = self.entries.borrow();
        let mut names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        let mut out = Vec::with_capacity(names.len());
        for name in names {
            if name == "set-cookie" {
                for (n, v) in entries.iter() {
                    if n == name {
                        out.push((n.clone(), v.clone()));
                    }
                }
            } else {
                let joined = entries
                    .iter()
                    .filter(|(n, _)| n == name)
                    .map(|(_, v)| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push((name.to_string(), joined));
            }
        }
        out
    }
}

#[rquickjs::methods]
impl Headers {
    #[qjs(constructor)]
    pub fn new<'js>(
        ctx: Ctx<'js>,
        init: rquickjs::function::Opt<Value<'js>>,
    ) -> rquickjs::Result<Self> {
        let entries = match init.0 {
            None => Vec::new(),
            Some(v) if v.is_undefined() || v.is_null() => Vec::new(),
            Some(v) => headers_from_init(&ctx, v)?,
        };
        Ok(Self {
            entries: RefCell::new(entries),
        })
    }

    pub fn get(&self, name: String) -> Option<String> {
        let name = name.to_lowercase();
        let entries = self.entries.borrow();
        let mut values = entries
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
            .peekable();
        values.peek()?;
        Some(values.collect::<Vec<_>>().join(", "))
    }

    pub fn has(&self, name: String) -> bool {
        let name = name.to_lowercase();
        self.entries.borrow().iter().any(|(n, _)| *n == name)
    }

    pub fn set(&self, name: String, value: String) {
        let name = name.to_lowercase();
        let mut entries = self.entries.borrow_mut();
        entries.retain(|(n, _)| *n != name);
        entries.push((name, value));
    }

    pub fn append(&self, name: String, value: String) {
        self.entries.borrow_mut().push((name.to_lowercase(), value));
    }

    pub fn delete(&self, name: String) {
        let name = name.to_lowercase();
        self.entries.borrow_mut().retain(|(n, _)| *n != name);
    }

    #[qjs(rename = "forEach")]
    pub fn for_each(&self, callback: rquickjs::Function<'_>) -> rquickjs::Result<()> {
        for (k, v) in self.combined() {
            callback.call::<_, ()>((v.as_str(), k.as_str()))?;
        }
        Ok(())
    }

    pub fn entries(&self) -> Vec<Vec<String>> {
        self.combined()
            .into_iter()
            .map(|(k, v)| vec![k, v])
            .collect()
    }

    pub fn keys(&self) -> Vec<String> {
        self.combined().into_iter().map(|(k, _)| k).collect()
    }

    pub fn values(&self) -> Vec<String> {
        self.combined().into_iter().map(|(_, v)| v).collect()
    }

    #[qjs(rename = "getSetCookie")]
    pub fn get_set_cookie(&self) -> Vec<String> {
        self.entries
            .borrow()
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .map(|(_, v)| v.clone())
            .collect()
    }
}
