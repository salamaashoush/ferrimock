//! Bridge JS handler functions to Rust `HandlerFn` via `ThreadsafeFunction`.
//!
//! Following ferridriver's TSFN patterns:
//! - `callee_handled::<false>()` — modern async pattern
//! - `weak::<true>()` — doesn't block Node.js exit
//! - `max_queue_size::<0>()` — unbounded queue

use crate::types::{JsHandlerResponse, JsRequestContext};
use mockpit::types::{DynamicResponse, HandlerFn, RequestContext};
use napi::bindgen_prelude::*;
use std::sync::Arc;

/// The concrete TSFN type for handler callbacks.
///
/// JS handler receives `JsRequestContext`, returns `Promise<JsHandlerResponse | null>`.
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
///
/// The returned `HandlerFn` can be used in `BodySource::Handler` and goes through
/// the standard `MockRegistry` / `MockMatcher` pipeline.
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
            let js_ctx = JsRequestContext::from(&ctx);
            match tsfn.call_async(js_ctx).await {
                Ok(promise) => match promise.await {
                    Ok(Some(resp)) => Ok(DynamicResponse::from(resp)),
                    Ok(None) => {
                        // Handler returned null/undefined = empty 200 response
                        Ok(DynamicResponse::body_only(bytes::Bytes::new()))
                    }
                    Err(e) => Err(anyhow::anyhow!("JS handler error: {e}")),
                },
                Err(e) => Err(anyhow::anyhow!("ThreadsafeFunction call error: {e}")),
            }
        })
    }))
}
