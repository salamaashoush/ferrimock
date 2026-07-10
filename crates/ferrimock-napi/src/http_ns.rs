//! HTTP namespace bindings: `http.get()`, `http.post()`, etc.

pub(crate) use crate::handler_bridge::HandlerFnRef;
use crate::handler_bridge::{HandlerBridge, js_to_handler_bridge};
use crate::request_context::{HandlerKind, RequestInfo};
use crate::types::HandlerResponse;
use ferrimock::handler;
use ferrimock::types::MockDefinition;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;

/// Handler registration options (MSW's third argument).
#[napi(object)]
#[derive(Default)]
pub struct RequestHandlerOptions {
    /// Deactivate the handler after its first successful response.
    pub once: Option<bool>,
}

/// Internal holder for a handler-based MockDefinition.
///
/// Created by `http.get()`, `http.post()`, etc. and consumed by `FerrimockServer.useHandlers()`.
#[napi]
pub struct RequestHandler {
    pub(crate) inner: Option<MockDefinition>,
    /// FunctionRef for direct same-thread handler calls (interceptor fast path).
    /// Stored separately from the MockDefinition because FunctionRef is napi-specific.
    pub(crate) fn_ref: Option<Arc<HandlerFnRef>>,
    /// The predicate as the user wrote it (`/users/:id`, a full URL, or a
    /// RegExp display form) — surfaced through `listHandlers()` since the
    /// engine only keeps the compiled pattern.
    pub(crate) pattern: Option<String>,
}

#[napi]
impl RequestHandler {
    /// Get the mock ID for this handler.
    #[napi(getter)]
    pub fn id(&self) -> Option<String> {
        self.inner.as_ref().map(|m| m.id.to_string())
    }

    /// The predicate as the user wrote it (path string or RegExp display).
    #[napi(getter)]
    pub fn pattern(&self) -> Option<String> {
        self.pattern.clone()
    }
}

impl RequestHandler {
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

pub(crate) fn finish_handler(
    bridge: HandlerBridge,
    mut mock_def: MockDefinition,
    options: Option<RequestHandlerOptions>,
    pattern: Option<String>,
) -> RequestHandler {
    mock_def.once = options.and_then(|o| o.once).unwrap_or(false);
    RequestHandler {
        inner: Some(mock_def),
        fn_ref: Some(bridge.fn_ref),
        pattern,
    }
}

/// JS RegExp flags filtered to the set the regex crate honors inline
/// (`i`, `m`, `s`); `g`/`y` don't affect matching and `u`/`v` are the
/// regex crate's default Unicode semantics.
pub(crate) fn compile_js_regex(source: &str, flags: &str) -> Result<regex::Regex> {
    let inline: String = flags.chars().filter(|c| "ims".contains(*c)).collect();
    let pattern = if inline.is_empty() {
        source.to_string()
    } else {
        format!("(?{inline}){source}")
    };
    regex::Regex::new(&pattern).map_err(|e| Error::from_reason(format!("Invalid RegExp: {e}")))
}

/// Extract `(source, flags)` when the value is a JS RegExp.
pub(crate) fn as_regexp(env: &Env, value: &Unknown) -> Result<Option<(String, String)>> {
    let mut value_type = napi::sys::ValueType::napi_undefined;
    #[allow(unsafe_code)]
    unsafe {
        napi::sys::napi_typeof(env.raw(), value.raw(), &mut value_type);
    }
    if value_type != napi::sys::ValueType::napi_object {
        return Ok(None);
    }
    #[allow(unsafe_code)]
    let obj: Object = unsafe { FromNapiValue::from_napi_value(env.raw(), value.raw())? };
    let Some(source) = obj.get::<String>("source")? else {
        return Ok(None);
    };
    let flags: String = obj.get("flags")?.unwrap_or_default();
    Ok(Some((source, flags)))
}

fn build_handler(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, RequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
    builder: fn(&str, ferrimock::types::HandlerFn) -> MockDefinition,
) -> Result<RequestHandler> {
    use ferrimock::types::UrlPattern;
    use smallvec::SmallVec;

    let bridge = js_to_handler_bridge(handler_fn, HandlerKind::Http)?;

    let (mock_def, pattern) = if let Some((source, flags)) = as_regexp(env, &path)? {
        let regex = compile_js_regex(&source, &flags)?;
        let mut mock = builder("*", bridge.handler_fn.clone());
        mock.request.url_patterns = SmallVec::from_elem(UrlPattern::Regex(regex), 1);
        (mock, format!("/{source}/{flags}"))
    } else {
        #[allow(unsafe_code)]
        let path_str: String = unsafe { FromNapiValue::from_napi_value(env.raw(), path.raw())? };
        let mock = builder(&path_str, bridge.handler_fn.clone());
        (mock, path_str)
    };

    Ok(finish_handler(bridge, mock_def, options, Some(pattern)))
}

macro_rules! http_method {
    ($name:ident, $builder:path, $doc:literal) => {
        #[doc = $doc]
        ///
        /// @param path - URL pattern string (`/users/:id`, full URL) or RegExp.
        /// @param handler - Function receiving the resolver info, returning a response,
        ///   or null/undefined to fall through to the next matching mock.
        /// @param options - Optional `{ once: true }` for one-time handlers.
        #[napi(namespace = "http")]
        pub fn $name(
            env: &Env,
            path: Unknown,
            handler_fn: Function<'_, RequestInfo, Promise<Option<HandlerResponse>>>,
            options: Option<RequestHandlerOptions>,
        ) -> Result<RequestHandler> {
            build_handler(env, path, handler_fn, options, $builder)
        }
    };
}

http_method!(get, handler::http::get, "Create a GET handler mock.");
http_method!(post, handler::http::post, "Create a POST handler mock.");
http_method!(put, handler::http::put, "Create a PUT handler mock.");
http_method!(patch, handler::http::patch, "Create a PATCH handler mock.");
http_method!(head, handler::http::head, "Create a HEAD handler mock.");
http_method!(
    options,
    handler::http::options,
    "Create an OPTIONS handler mock."
);
http_method!(
    all,
    handler::http::all,
    "Create a handler mock matching any HTTP method."
);

/// Create a DELETE handler mock.
///
/// @param path - URL pattern string (`/users/:id`, full URL) or RegExp.
/// @param handler - Function receiving the resolver info, returning a response,
///   or null/undefined to fall through to the next matching mock.
/// @param options - Optional `{ once: true }` for one-time handlers.
#[napi(namespace = "http", js_name = "delete")]
pub fn delete_handler(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, RequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
) -> Result<RequestHandler> {
    build_handler(env, path, handler_fn, options, handler::http::delete)
}
