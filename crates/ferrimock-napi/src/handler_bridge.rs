//! Bridge JS handler functions to Rust `HandlerFn` via `ThreadsafeFunction`.
//!
//! Handlers receive the lazy resolver-info class matching their kind
//! (`RequestInfo` for HTTP, `GraphQLRequestInfo` for GraphQL) instead
//! of an eager object, so headers/query/body only convert on access.
//!
//! Two call paths:
//! - **TSFN path** (server mode): cross-thread call via ThreadsafeFunction (~22us overhead)
//! - **FunctionRef path** (interceptor mode): direct napi_call_function (~1us overhead)
//!
//! The FunctionRef path is used in `match_request` when we're already on the JS thread.
//! The deferred resolver callback has Env access, so we can borrow_back the FunctionRef
//! and call the JS handler directly without the UV event loop queue+wakeup overhead.

use crate::request_context::{HandlerKind, ResolverArg};
use crate::types::HandlerResponse;
use ferrimock::types::{DynamicResponse, HandlerFn, RequestContext};
use napi::bindgen_prelude::*;
use std::sync::Arc;

/// TSFN type: handler receives the resolver-info class for its kind,
/// returns Promise<response>.
pub type HandlerCallbackTsfn = napi::threadsafe_function::ThreadsafeFunction<
    ResolverArg,
    Promise<Option<HandlerResponse>>,
    ResolverArg,
    Status,
    false, // callee_handled
    true,  // weak
    0,     // unbounded queue
>;

/// FunctionRef for direct same-thread handler calls (interceptor fast path).
///
/// Returns `Unknown` so we can inspect the raw JS return value.
/// Sync handlers return the HandlerResponse object directly.
/// Async handlers return a Promise — we detect this with napi_is_promise
/// and chain .then() to extract the value.
pub type HandlerFnRef = FunctionRef<ResolverArg, Unknown<'static>>;

/// Result of converting a JS handler function — contains both TSFN and FunctionRef.
pub struct HandlerBridge {
    pub handler_fn: HandlerFn,
    pub fn_ref: Arc<HandlerFnRef>,
}

/// Convert a JS function into both a TSFN-based `HandlerFn` and a `FunctionRef`.
///
/// Generic over the declared argument type: the factories type the
/// callback for TS declarations, but the bridge is type-erased.
pub fn js_to_handler_bridge<Arg: JsValuesTupleIntoVec>(
    callback: Function<'_, Arg, Promise<Option<HandlerResponse>>>,
    kind: HandlerKind,
) -> Result<HandlerBridge> {
    use napi::JsValue;
    let v = callback.value();
    #[allow(unsafe_code)]
    // SAFETY: v.value is a valid napi_value from the Function parameter;
    // the Arg/Return generics are phantom (only affect call typing).
    let fn_ref: HandlerFnRef = unsafe { FromNapiValue::from_napi_value(v.env, v.value)? };
    let fn_ref = Arc::new(fn_ref);

    // Build TSFN for server mode (cross-thread calls), re-typed to the
    // erased resolver argument.
    #[allow(unsafe_code)]
    // SAFETY: same napi_value; TSFN generics only affect call typing.
    let erased: Function<'_, ResolverArg, Promise<Option<HandlerResponse>>> =
        unsafe { FromNapiValue::from_napi_value(v.env, v.value)? };
    let tsfn: HandlerCallbackTsfn = erased
        .build_threadsafe_function()
        .callee_handled::<false>()
        .weak::<true>()
        .max_queue_size::<0>()
        .build()?;

    let tsfn = Arc::new(tsfn);

    let handler_fn: HandlerFn = Arc::new(move |ctx: RequestContext| {
        let tsfn = Arc::clone(&tsfn);
        Box::pin(async move {
            let arg = ResolverArg::new(kind, ctx, None);

            match tsfn.call_async(arg).await {
                Ok(promise) => match promise.await {
                    Ok(Some(resp)) => {
                        let dynamic = DynamicResponse::from(resp);
                        // The stream stash lives on the interceptor side of
                        // the boundary; it cannot be delivered over the
                        // standalone TCP server.
                        if dynamic
                            .headers
                            .as_ref()
                            .is_some_and(|h| h.contains_key("x-ferrimock-stream-id"))
                        {
                            return Err(ferrimock::FerrimockError::msg(
                                "handler Response was routed to the interceptor stash while the \
                                 standalone HTTP server was serving; run the TCP server without \
                                 an active interceptor to get buffered Response bodies",
                            ));
                        }
                        Ok(dynamic)
                    }
                    // MSW semantics: undefined/null falls through to the
                    // next matching mock (the serve loop retries).
                    Ok(None) => Ok(DynamicResponse::fallthrough()),
                    Err(e) => Err(ferrimock::FerrimockError::msg(format!(
                        "JS handler error: {e}"
                    ))),
                },
                Err(e) => Err(ferrimock::FerrimockError::msg(format!(
                    "ThreadsafeFunction call error: {e}"
                ))),
            }
        })
    });

    Ok(HandlerBridge { handler_fn, fn_ref })
}
