//! Bridge JS handler functions to Rust `HandlerFn` via `ThreadsafeFunction`.
//!
//! Uses MockpitRequest (lazy class) instead of JsRequestContext (eager object)
//! to avoid constructing a full JS object with all headers/query/body per request.
//!
//! Two call paths:
//! - **TSFN path** (server mode): cross-thread call via ThreadsafeFunction (~22us overhead)
//! - **FunctionRef path** (interceptor mode): direct napi_call_function (~1us overhead)
//!
//! The FunctionRef path is used in `match_request` when we're already on the JS thread.
//! The deferred resolver callback has Env access, so we can borrow_back the FunctionRef
//! and call the JS handler directly without the UV event loop queue+wakeup overhead.

use crate::request_context::MockpitRequest;
use crate::types::JsHandlerResponse;
use mockpit::types::{DynamicResponse, HandlerFn, RequestContext};
use napi::bindgen_prelude::*;
use std::sync::Arc;

/// TSFN type: handler receives MockpitRequest (lazy class), returns Promise<response>.
pub type HandlerCallbackTsfn = napi::threadsafe_function::ThreadsafeFunction<
    MockpitRequest,
    Promise<Option<JsHandlerResponse>>,
    MockpitRequest,
    Status,
    false, // callee_handled
    true,  // weak
    0,     // unbounded queue
>;

/// FunctionRef for direct same-thread handler calls (interceptor fast path).
///
/// Returns `Unknown` so we can inspect the raw JS return value.
/// Sync handlers return the JsHandlerResponse object directly.
/// Async handlers return a Promise — we detect this with napi_is_promise
/// and chain .then() to extract the value.
pub type HandlerFnRef = FunctionRef<MockpitRequest, Unknown<'static>>;

/// Result of converting a JS handler function — contains both TSFN and FunctionRef.
pub struct HandlerBridge {
    pub handler_fn: HandlerFn,
    pub fn_ref: Arc<HandlerFnRef>,
}

/// Convert a JS function into both a TSFN-based `HandlerFn` and a `FunctionRef`.
pub fn js_to_handler_bridge(
    callback: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<HandlerBridge> {
    // Create FunctionRef with Unknown return type so we can inspect the raw value.
    // The underlying napi_ref is type-erased — phantom Return only affects call().
    use napi::JsValue;
    let v = callback.value();
    #[allow(unsafe_code)]
    // SAFETY: v.value is a valid napi_value from the Function parameter
    let fn_ref: HandlerFnRef = unsafe { FromNapiValue::from_napi_value(v.env, v.value)? };
    let fn_ref = Arc::new(fn_ref);

    // Build TSFN for server mode (cross-thread calls)
    let tsfn: HandlerCallbackTsfn = callback
        .build_threadsafe_function()
        .callee_handled::<false>()
        .weak::<true>()
        .max_queue_size::<0>()
        .build()?;

    let tsfn = Arc::new(tsfn);

    let handler_fn: HandlerFn = Arc::new(move |ctx: RequestContext| {
        let tsfn = Arc::clone(&tsfn);
        Box::pin(async move {
            // MockpitRequest is a thin wrapper -- no HashMap cloning here.
            // Fields are converted to JS values lazily when the handler accesses them.
            let req = MockpitRequest::new(ctx);

            match tsfn.call_async(req).await {
                Ok(promise) => match promise.await {
                    Ok(Some(resp)) => Ok(DynamicResponse::from(resp)),
                    Ok(None) => Ok(DynamicResponse::body_only(bytes::Bytes::new())),
                    Err(e) => Err(mockpit::MockpitError::msg(format!("JS handler error: {e}"))),
                },
                Err(e) => Err(mockpit::MockpitError::msg(format!(
                    "ThreadsafeFunction call error: {e}"
                ))),
            }
        })
    });

    Ok(HandlerBridge { handler_fn, fn_ref })
}

/// Legacy: Convert a JS function into a Rust `HandlerFn` only (TSFN path).
/// Used by code that doesn't need the FunctionRef fast path.
pub fn js_to_handler_fn(
    callback: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<HandlerFn> {
    Ok(js_to_handler_bridge(callback)?.handler_fn)
}
