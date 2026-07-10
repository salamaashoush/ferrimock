//! `sse(path, resolver)`: native Server-Sent Events handler registration.
//!
//! The returned handler is a real engine mock with BOTH facets:
//! - `response.body: Handler` + FunctionRef — the interceptor lane calls
//!   the JS resolver with `(info)` and it answers with a stream-stashed
//!   `text/event-stream` Response (MSW accept-header predicate applies).
//! - `streaming: SseHandler` — `FerrimockServer.listen()`'s TCP lane calls
//!   the same JS resolver with `(info, client)` where `client` is a
//!   native sink handle, so frames stream without the stash.

use crate::handler_bridge::js_to_handler_bridge;
use crate::http_ns::{RequestHandler, RequestHandlerOptions, as_regexp, compile_js_regex};
use crate::request_context::{HandlerKind, RequestInfo};
use crate::types::HandlerResponse;
use ferrimock::types::{SseHandlerFn, SseMessage, SseSinkMsg, UrlPattern};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::mpsc;

/// One outbound SSE message: `data` objects are JSON-stringified (MSW
/// semantics), strings pass through verbatim.
#[napi(object)]
pub struct SseMessagePayload {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: Option<serde_json::Value>,
    pub retry: Option<u32>,
}

/// The live connection sink handed to TCP-lane resolvers.
#[napi]
pub struct SseClientHandle {
    tx: mpsc::UnboundedSender<SseSinkMsg>,
}

#[napi]
impl SseClientHandle {
    /// Send a message; sends after close are silent no-ops.
    #[napi]
    pub fn send(&self, payload: SseMessagePayload) {
        let data = match payload.data {
            None | Some(serde_json::Value::Null) => String::new(),
            Some(serde_json::Value::String(s)) => s,
            Some(other) => other.to_string(),
        };
        let _ = self.tx.send(SseSinkMsg::Message(SseMessage {
            id: payload.id,
            event: payload.event,
            data,
            retry: payload.retry,
        }));
    }

    /// End the stream cleanly.
    #[napi]
    pub fn close(&self) {
        let _ = self.tx.send(SseSinkMsg::Close);
    }

    /// Abort the connection (the consumer sees a network error).
    #[napi]
    pub fn error(&self) {
        let _ = self.tx.send(SseSinkMsg::Error);
    }
}

/// TSFN type for the TCP lane: the resolver receives `(info, client)`.
type SseResolverTsfn = napi::threadsafe_function::ThreadsafeFunction<
    FnArgs<(RequestInfo, SseClientHandle)>,
    Promise<Option<HandlerResponse>>,
    FnArgs<(RequestInfo, SseClientHandle)>,
    Status,
    false, // callee_handled
    true,  // weak
    0,     // unbounded queue
>;

/// Create a Server-Sent Events handler mock.
///
/// @param path - URL pattern string (`/stream`, absolute URL) or RegExp.
/// @param handler - Resolver receiving `(info, client?)`: with a native
///   `client` (TCP lane) it drives the sink; without one (interceptor
///   lane) it returns a `text/event-stream` Response.
/// @param options - Optional `{ once: true }` for one-time handlers.
#[napi]
pub fn sse(
    env: &Env,
    path: Unknown,
    handler_fn: Function<'_, RequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
) -> Result<RequestHandler> {
    use napi::JsValue;
    let raw = handler_fn.value();
    // SAFETY: same napi_value; TSFN generics only affect call typing.
    #[allow(unsafe_code)]
    let two_arg: Function<
        '_,
        FnArgs<(RequestInfo, SseClientHandle)>,
        Promise<Option<HandlerResponse>>,
    > = unsafe { FromNapiValue::from_napi_value(raw.env, raw.value)? };
    let tsfn: SseResolverTsfn = two_arg
        .build_threadsafe_function()
        .callee_handled::<false>()
        .weak::<true>()
        .max_queue_size::<0>()
        .build()?;
    let tsfn = Arc::new(tsfn);

    let bridge = js_to_handler_bridge(handler_fn, HandlerKind::Http)?;

    let streaming_fn: SseHandlerFn = Arc::new(move |ctx, tx| {
        let tsfn = Arc::clone(&tsfn);
        Box::pin(async move {
            let info = RequestInfo::new(ctx, None);
            let client = SseClientHandle { tx };
            match tsfn
                .call_async(FnArgs {
                    data: (info, client),
                })
                .await
            {
                Ok(promise) => match promise.await {
                    // Resolver returning does NOT close the stream — the
                    // connection lives until the client handle closes it
                    // or the consumer disconnects.
                    Ok(_) => Ok(()),
                    Err(e) => Err(ferrimock::FerrimockError::msg(format!(
                        "sse resolver failed: {e}"
                    ))),
                },
                Err(e) => Err(ferrimock::FerrimockError::msg(format!(
                    "sse ThreadsafeFunction call failed: {e}"
                ))),
            }
        })
    });

    let (mut mock, pattern) = if let Some((source, flags)) = as_regexp(env, &path)? {
        let regex = compile_js_regex(&source, &flags)?;
        let mut mock = ferrimock::handler::http::get("*", bridge.handler_fn.clone());
        mock.request.url_patterns = smallvec::SmallVec::from_elem(UrlPattern::Regex(regex), 1);
        (mock, format!("/{source}/{flags}"))
    } else {
        // SAFETY: not a RegExp, so the value must be the path string.
        #[allow(unsafe_code)]
        let path_str: String = unsafe { FromNapiValue::from_napi_value(env.raw(), path.raw())? };
        let mock = ferrimock::handler::http::get(&path_str, bridge.handler_fn.clone());
        (mock, path_str)
    };
    mock.streaming = Some(ferrimock::types::StreamingResponse::SseHandler(
        streaming_fn,
    ));
    mock.once = options.and_then(|o| o.once).unwrap_or(false);

    Ok(RequestHandler {
        inner: Some(mock),
        fn_ref: Some(bridge.fn_ref),
        pattern: Some(pattern),
    })
}
