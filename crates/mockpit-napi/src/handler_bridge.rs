//! Bridge JS handler functions to Rust `HandlerFn` via `ThreadsafeFunction`.
//!
//! Optimization: direct return type (no double Promise wrapping).
//! JsRequestContext uses minimal construction to avoid unnecessary cloning.

use crate::types::{JsHandlerResponse, JsRequestContext};
use mockpit::types::{DynamicResponse, HandlerFn, RequestContext};
use napi::bindgen_prelude::*;
use std::sync::Arc;

/// TSFN type: handler receives JsRequestContext, returns Promise<JsHandlerResponse | null>.
/// call_async() unwraps the Promise automatically -- single await, not double.
pub type HandlerCallbackTsfn = napi::threadsafe_function::ThreadsafeFunction<
    JsRequestContext,
    Promise<Option<JsHandlerResponse>>,
    JsRequestContext,
    Status,
    false, // callee_handled
    true,  // weak
    0,     // unbounded queue
>;

/// Convert a JS function into a Rust `HandlerFn`.
pub fn js_to_handler_fn(
    callback: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<HandlerFn> {
    let tsfn: HandlerCallbackTsfn = callback
        .build_threadsafe_function()
        .callee_handled::<false>()
        .weak::<true>()
        .max_queue_size::<0>()
        .build()?;

    let tsfn = Arc::new(tsfn);

    Ok(Arc::new(move |ctx: RequestContext| {
        let tsfn = Arc::clone(&tsfn);
        Box::pin(async move {
            let js_ctx = JsRequestContext::from_context_minimal(&ctx);

            // call_async sends to JS thread and awaits the resolved Promise value.
            // Single await -- no double Promise wrapping.
            match tsfn.call_async(js_ctx).await {
                Ok(promise) => match promise.await {
                    Ok(Some(resp)) => Ok(DynamicResponse::from(resp)),
                    Ok(None) => Ok(DynamicResponse::body_only(bytes::Bytes::new())),
                    Err(e) => Err(anyhow::anyhow!("JS handler error: {e}")),
                },
                Err(e) => Err(anyhow::anyhow!("ThreadsafeFunction call error: {e}")),
            }
        })
    }))
}
