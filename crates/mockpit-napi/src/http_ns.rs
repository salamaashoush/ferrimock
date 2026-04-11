//! HTTP namespace bindings: `http.get()`, `http.post()`, etc.

use crate::handler_bridge::js_to_handler_fn;
use crate::types::{JsHandlerResponse, JsRequestContext};
use mockpit::handler;
use mockpit::types::MockDefinition;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Internal holder for a handler-based MockDefinition.
///
/// Created by `http.get()`, `http.post()`, etc. and consumed by `MockpitServer.useHandlers()`.
#[napi]
pub struct JsHandler {
    pub(crate) inner: Option<MockDefinition>,
}

#[napi]
impl JsHandler {
    /// Get the mock ID for this handler.
    #[napi(getter)]
    pub fn id(&self) -> Option<String> {
        self.inner.as_ref().map(|m| m.id.to_string())
    }
}

impl JsHandler {
    /// Take the inner MockDefinition, leaving None.
    pub(crate) fn take(&mut self) -> Result<MockDefinition> {
        self.inner
            .take()
            .ok_or_else(|| Error::from_reason("Handler already consumed"))
    }
}

/// Create a GET handler mock.
///
/// @param path - URL pattern (e.g., `/users/:id`). Supports `:param` captures and `*` wildcards.
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "http")]
pub fn get(
    path: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::http::get(&path, rust_handler)),
    })
}

/// Create a POST handler mock.
#[napi(namespace = "http")]
pub fn post(
    path: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::http::post(&path, rust_handler)),
    })
}

/// Create a PUT handler mock.
#[napi(namespace = "http")]
pub fn put(
    path: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::http::put(&path, rust_handler)),
    })
}

/// Create a DELETE handler mock.
#[napi(namespace = "http", js_name = "delete")]
pub fn delete_handler(
    path: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::http::delete(&path, rust_handler)),
    })
}

/// Create a PATCH handler mock.
#[napi(namespace = "http")]
pub fn patch(
    path: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::http::patch(&path, rust_handler)),
    })
}

/// Create a handler mock matching any HTTP method.
#[napi(namespace = "http")]
pub fn all(
    path: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::http::all(&path, rust_handler)),
    })
}
