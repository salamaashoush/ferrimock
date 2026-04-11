//! Lazy request context -- class with on-demand getters instead of eager object construction.
//!
//! `#[napi(object)]` creates ALL fields upfront as JS values.
//! `#[napi]` class only creates JS values when getters are called.
//! Most handlers only access `params` -- they never touch `headers` or `body`.

use mockpit::types::RequestContext;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global request counter for generating unique request IDs.
static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Request context passed to handler functions.
///
/// Fields are lazily converted to JS values on first access.
/// This avoids the overhead of constructing a full JS object with
/// all headers, query params, etc. for every request.
#[napi]
pub struct MockpitRequest {
    inner: RequestContext,
    request_id: String,
}

#[napi]
impl MockpitRequest {
    pub(crate) fn new(ctx: RequestContext) -> Self {
        let id = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: ctx,
            request_id: format!("req:{id:x}"),
        }
    }

    /// Unique request identifier.
    #[napi(getter)]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    #[napi(getter)]
    pub fn method(&self) -> &str {
        &self.inner.method
    }

    #[napi(getter)]
    pub fn path(&self) -> &str {
        &self.inner.path
    }

    #[napi(getter)]
    pub fn uri(&self) -> &str {
        &self.inner.uri
    }

    #[napi(getter)]
    pub fn params(&self) -> HashMap<String, String> {
        self.inner
            .captures
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[napi(getter)]
    pub fn query(&self) -> HashMap<String, String> {
        self.inner
            .query
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[napi(getter)]
    pub fn headers(&self) -> HashMap<String, String> {
        self.inner
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[napi(getter)]
    pub fn body(&self) -> Option<&str> {
        self.inner.body.as_deref()
    }

    #[napi(getter)]
    pub fn body_json(&self) -> Option<serde_json::Value> {
        self.inner.body_json.clone()
    }

    /// Get a single param by name (faster than accessing the full params object).
    #[napi]
    pub fn param(&self, name: String) -> Option<String> {
        self.inner.captures.get(&name).cloned()
    }

    /// Get a single header by name (faster than accessing the full headers object).
    #[napi]
    pub fn header(&self, name: String) -> Option<String> {
        self.inner.headers.get(&name).cloned()
    }

    /// Get a single query param by name.
    #[napi]
    pub fn query_param(&self, name: String) -> Option<String> {
        self.inner.query.get(&name).cloned()
    }

    /// Parsed cookies from the Cookie request header.
    #[napi(getter)]
    pub fn cookies(&self) -> HashMap<String, String> {
        self.inner
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
                            Some((name.to_string(), value.to_string()))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}
