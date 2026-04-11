//! Lazy request context -- class with on-demand getters instead of eager object construction.
//!
//! `#[napi(object)]` creates ALL fields upfront as JS values.
//! `#[napi]` class only creates JS values when getters are called.
//! Most handlers only access `params` -- they never touch `headers` or `body`.

use mockpit::types::RequestContext;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;

/// Request context passed to handler functions.
///
/// Fields are lazily converted to JS values on first access.
/// This avoids the overhead of constructing a full JS object with
/// all headers, query params, etc. for every request.
#[napi]
pub struct MockpitRequest {
    inner: RequestContext,
}

#[napi]
impl MockpitRequest {
    pub(crate) fn new(ctx: RequestContext) -> Self {
        Self { inner: ctx }
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
}
