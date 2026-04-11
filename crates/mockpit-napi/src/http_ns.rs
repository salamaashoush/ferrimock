//! HTTP namespace bindings: `http.get()`, `http.post()`, etc.

use crate::handler_bridge::js_to_handler_bridge;
pub(crate) use crate::handler_bridge::HandlerFnRef;
use crate::request_context::MockpitRequest;
use crate::types::JsHandlerResponse;
use mockpit::handler;
use mockpit::types::MockDefinition;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;

/// Internal holder for a handler-based MockDefinition.
///
/// Created by `http.get()`, `http.post()`, etc. and consumed by `MockpitServer.useHandlers()`.
#[napi]
pub struct JsHandler {
    pub(crate) inner: Option<MockDefinition>,
    /// FunctionRef for direct same-thread handler calls (interceptor fast path).
    /// Stored separately from the MockDefinition because FunctionRef is napi-specific.
    pub(crate) fn_ref: Option<Arc<HandlerFnRef>>,
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

    /// Take the FunctionRef, leaving None.
    pub(crate) fn take_fn_ref(&mut self) -> Option<Arc<HandlerFnRef>> {
        self.fn_ref.take()
    }
}

fn build_handler(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
    builder: fn(&str, mockpit::types::HandlerFn) -> MockDefinition,
) -> Result<JsHandler> {
    use mockpit::types::UrlPattern;
    use smallvec::SmallVec;

    let bridge = js_to_handler_bridge(handler_fn)?;

    // Check if path is a string or RegExp
    let mut value_type = napi::sys::ValueType::napi_undefined;
    #[allow(unsafe_code)]
    unsafe {
        napi::sys::napi_typeof(env.raw(), path.raw(), &mut value_type);
    }

    let mock_def = if value_type == napi::sys::ValueType::napi_string {
        #[allow(unsafe_code)]
        let path_str: String = unsafe { FromNapiValue::from_napi_value(env.raw(), path.raw())? };
        builder(&path_str, bridge.handler_fn)
    } else {
        // Assume RegExp — extract source and flags properties
        #[allow(unsafe_code)]
        let obj: Object = unsafe { FromNapiValue::from_napi_value(env.raw(), path.raw())? };
        let source: String = obj.get("source")?.ok_or_else(|| Error::from_reason("Not a RegExp: missing 'source'"))?;
        let flags: String = obj.get("flags")?.unwrap_or_default();

        let pattern = if flags.contains('i') {
            format!("(?i){source}")
        } else {
            source
        };
        let regex = regex::Regex::new(&pattern)
            .map_err(|e| Error::from_reason(format!("Invalid RegExp: {e}")))?;

        let mut mock = builder("*", bridge.handler_fn);
        mock.request.url_patterns = SmallVec::from_elem(UrlPattern::Regex(regex), 1);
        mock
    };

    Ok(JsHandler {
        inner: Some(mock_def),
        fn_ref: Some(bridge.fn_ref),
    })
}

/// Create a GET handler mock.
///
/// @param path - URL pattern string (e.g., `/users/:id`) or RegExp.
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "http")]
pub fn get(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::get)
}

/// Create a POST handler mock.
#[napi(namespace = "http")]
pub fn post(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::post)
}

/// Create a PUT handler mock.
#[napi(namespace = "http")]
pub fn put(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::put)
}

/// Create a DELETE handler mock.
#[napi(namespace = "http", js_name = "delete")]
pub fn delete_handler(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::delete)
}

/// Create a PATCH handler mock.
#[napi(namespace = "http")]
pub fn patch(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::patch)
}

/// Create a HEAD handler mock.
#[napi(namespace = "http")]
pub fn head(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::head)
}

/// Create an OPTIONS handler mock.
#[napi(namespace = "http")]
pub fn options(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::options)
}

/// Create a handler mock matching any HTTP method.
#[napi(namespace = "http")]
pub fn all(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    build_handler(env, path, handler_fn, handler::http::all)
}
