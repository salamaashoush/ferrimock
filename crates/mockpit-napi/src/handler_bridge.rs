//! Bridge JS handler functions to Rust `HandlerFn` via `ThreadsafeFunction`.
//!
//! Uses MockpitRequest (lazy class) instead of JsRequestContext (eager object)
//! to avoid constructing a full JS object with all headers/query/body per request.

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

/// Convert a JS function into a Rust `HandlerFn`.
pub fn js_to_handler_fn(
    callback: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
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
            // MockpitRequest is a thin wrapper -- no HashMap cloning here.
            // Fields are converted to JS values lazily when the handler accesses them.
            let req = MockpitRequest::new(ctx);

            match tsfn.call_async(req).await {
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
