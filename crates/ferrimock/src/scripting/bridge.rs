//! Bridges a persisted JS handler into ferrimock's [`HandlerFn`] type.
//!
//! The closure is `Send + Sync` because it captures only a weak engine
//! reference and shared state; the JS function itself never leaves the
//! realm -- each call is one [`ferrijs::Runtime::run`] body that
//! restores the `Persistent` by slot id, invokes it with a lazy request
//! object, awaits a returned promise, and converts the resolved value.
//! The run bracket is the whole guard: the interrupt deadline kills
//! runaway bytecode at the handler budget, the backstop frees a call
//! parked on a host await, and either outcome poisons the realm.

use std::sync::{Arc, Weak};

use ferrijs::{RunOptions, ScriptError};
use ferrijs_bundle::CompiledModule;
use rquickjs::function::This;
use rquickjs::{CatchResultExt, Class, Ctx, Function, Object, Persistent, Promise, Value};

use crate::FerrimockError;
use crate::types::{DynamicResponse, HandlerFn, RequestContext};

use super::bindings::request::{GraphQLRequestInfo, RequestInfo};
use super::bindings::response::{ConvertedResponse, value_to_dynamic_response};
use super::engine::{ScriptEngine, poisoned_error, remap_error};
use super::slots::with_slots;

/// Which resolver info object the handler receives.
#[derive(Clone, Copy)]
pub enum HandlerKind {
    Http,
    GraphQL,
}

/// The engine behind a mock, if its file is still loaded. Handlers hold
/// the engine weakly so unloading a file (or replacing it on reload)
/// tears its realm down even while stale mocks linger in a registry.
pub(super) fn live_engine(
    engine: &Weak<ScriptEngine>,
) -> Result<Arc<ScriptEngine>, FerrimockError> {
    let engine = engine.upgrade().ok_or_else(|| {
        FerrimockError::Script("script engine is gone; the script file was unloaded".to_string())
    })?;
    if engine.is_poisoned() {
        return Err(poisoned_error());
    }
    Ok(engine)
}

/// A JS exception as a [`ScriptError`], its stack mapped through the
/// bundle the realm registered.
pub(super) fn caught<'js>(ctx: &Ctx<'js>, e: rquickjs::CaughtError<'js>) -> ScriptError {
    ScriptError::from_caught(ctx, e, "")
}

pub(super) fn restore_handler<'js>(
    ctx: &Ctx<'js>,
    slot: u64,
) -> Result<Function<'js>, ScriptError> {
    let persistent = with_slots(ctx, |slots| slots.get(slot))
        .map_err(|e| ScriptError::internal(e.to_string()))?
        .ok_or_else(|| ScriptError::internal(format!("script handler slot {slot} is gone")))?;
    persistent
        .restore(ctx)
        .map_err(|e| ScriptError::internal(format!("restore handler: {e}")))
}

fn request_value<'js>(
    ctx: &Ctx<'js>,
    kind: HandlerKind,
    request: RequestContext,
) -> Result<Value<'js>, ScriptError> {
    let value = match kind {
        HandlerKind::Http => Class::instance(ctx.clone(), RequestInfo::new(request))
            .catch(ctx)
            .map_err(|e| caught(ctx, e))?
            .as_value()
            .clone(),
        HandlerKind::GraphQL => Class::instance(ctx.clone(), GraphQLRequestInfo::new(request))
            .catch(ctx)
            .map_err(|e| caught(ctx, e))?
            .as_value()
            .clone(),
    };
    Ok(value)
}

/// Kick off one step of a generator resolver: create the iterator on the
/// first request (calling the generator function with the resolver
/// info), then call `next()`. The returned value may be a promise
/// (async generators) -- the caller awaits it before
/// [`finish_generator_step`].
fn begin_generator_step<'js>(
    ctx: &Ctx<'js>,
    slot: u64,
    request: RequestContext,
    kind: HandlerKind,
) -> Result<Value<'js>, ScriptError> {
    let existing = with_slots(ctx, |slots| slots.iterator(slot))
        .map_err(|e| ScriptError::internal(e.to_string()))?;
    let iterator = if let Some(persistent) = existing {
        persistent
            .restore(ctx)
            .map_err(|e| ScriptError::internal(format!("restore iterator: {e}")))?
    } else {
        let func = restore_handler(ctx, slot)?;
        let req = request_value(ctx, kind, request)?;
        let iterator: Object<'js> = func.call((req,)).catch(ctx).map_err(|e| caught(ctx, e))?;
        let persistent = Persistent::save(ctx, iterator.clone());
        with_slots(ctx, |slots| slots.set_iterator(slot, persistent))
            .map_err(|e| ScriptError::internal(e.to_string()))?;
        iterator
    };

    let next: Function<'js> = iterator
        .get("next")
        .map_err(|e| ScriptError::internal(format!("generator iterator has no next(): {e}")))?;
    next.call((This(iterator.clone()),))
        .catch(ctx)
        .map_err(|e| caught(ctx, e))
}

/// Interpret an awaited generator step: unwrap `{ value, done }`, keep
/// the last yielded value, and repeat it after exhaustion (MSW
/// generator-resolver semantics).
fn finish_generator_step<'js>(
    ctx: &Ctx<'js>,
    slot: u64,
    step: Value<'js>,
) -> Result<Value<'js>, ScriptError> {
    let step_obj = step
        .into_object()
        .ok_or_else(|| ScriptError::internal("generator step is not an object"))?;
    let done: bool = step_obj.get("done").unwrap_or(false);
    let value: Value<'js> = step_obj
        .get("value")
        .map_err(|e| ScriptError::internal(format!("generator step value: {e}")))?;

    if value.is_undefined() && done {
        // Exhausted: repeat the last yielded value.
        if let Some(last) = with_slots(ctx, |slots| slots.last_value(slot))
            .map_err(|e| ScriptError::internal(e.to_string()))?
        {
            return last
                .restore(ctx)
                .map_err(|e| ScriptError::internal(format!("restore generator value: {e}")));
        }
        return Ok(value);
    }

    if !value.is_undefined() {
        let persistent = Persistent::save(ctx, value.clone());
        with_slots(ctx, |slots| slots.set_last_value(slot, persistent))
            .map_err(|e| ScriptError::internal(e.to_string()))?;
    }
    Ok(value)
}

/// Await a possibly-promise JS value inside the run body. A macro so
/// the await stays inline (rquickjs futures are single-threaded; a
/// helper async fn would be non-Send).
macro_rules! await_js {
    ($ctx:expr, $value:expr) => {{
        let value: Value<'_> = $value;
        if let Some(promise) = value.as_promise() {
            let promise: Promise<'_> = promise.clone();
            match promise.into_future::<Value<'_>>().await.catch($ctx) {
                Ok(v) => v,
                Err(e) => return Err($crate::scripting::bridge::caught($ctx, e)),
            }
        } else {
            value
        }
    }};
}

pub(super) use await_js;

/// Bytes behind a stream chunk (string, `Uint8Array` or `ArrayBuffer`).
// 0.13 forbids running JS while a buffer borrow is alive; these copy out
// immediately.
#[allow(unsafe_code)]
fn chunk_bytes(chunk: &Value<'_>) -> Result<Vec<u8>, ScriptError> {
    if let Some(s) = chunk.as_string() {
        return s
            .to_string()
            .map(String::into_bytes)
            .map_err(|e| ScriptError::internal(format!("stream chunk: {e}")));
    }
    if let Ok(ta) = rquickjs::TypedArray::<u8>::from_value(chunk.clone()) {
        // SAFETY: copied out immediately.
        return Ok(unsafe { ta.as_bytes() }.unwrap_or_default().to_vec());
    }
    if let Some(ab) = rquickjs::ArrayBuffer::from_value(chunk.clone()) {
        // SAFETY: copied out immediately.
        return Ok(unsafe { ab.as_bytes() }.unwrap_or_default().to_vec());
    }
    Err(ScriptError::named(
        "TypeError",
        "ReadableStream chunks must be strings, ArrayBuffers, or TypedArrays",
    ))
}

/// The reason a stream's `read()` rejected with: the string a
/// `controller.error('boom')` passed, or the exception's own message.
fn rejection_message<'js>(ctx: &Ctx<'js>, e: rquickjs::CaughtError<'js>) -> String {
    match e {
        rquickjs::CaughtError::Value(v) => v
            .as_string()
            .and_then(|s| s.to_string().ok())
            .unwrap_or_else(|| format!("{v:?}")),
        other => caught(ctx, other).message,
    }
}

/// The default reader of a WHATWG `ReadableStream` and its `read`
/// method, so the drain loop can stay inline in the run body.
fn stream_reader<'js>(
    ctx: &Ctx<'js>,
    stream: Value<'js>,
) -> Result<(Object<'js>, Function<'js>), ScriptError> {
    let stream = stream
        .into_object()
        .ok_or_else(|| ScriptError::internal("response body stream is not a ReadableStream"))?;
    let get_reader: Function<'js> = stream
        .get("getReader")
        .map_err(|e| ScriptError::internal(format!("ReadableStream.getReader: {e}")))?;
    let reader: Object<'js> = get_reader
        .call((This(stream),))
        .catch(ctx)
        .map_err(|e| caught(ctx, e))?;
    let read: Function<'js> = reader
        .get("read")
        .map_err(|e| ScriptError::internal(format!("reader.read: {e}")))?;
    Ok((reader, read))
}

/// One `reader.read()` step, settled: the chunk's bytes, or `None` once
/// the stream is done.
fn read_step<'js>(
    ctx: &Ctx<'js>,
    settled: rquickjs::Result<Object<'js>>,
) -> Result<Option<Vec<u8>>, ScriptError> {
    let result = settled.catch(ctx).map_err(|e| {
        ScriptError::internal(format!(
            "response stream errored: {}",
            rejection_message(ctx, e)
        ))
    })?;
    if result.get::<_, bool>("done").unwrap_or(false) {
        return Ok(None);
    }
    let value: Value<'js> = result
        .get("value")
        .map_err(|e| ScriptError::internal(format!("stream chunk: {e}")))?;
    chunk_bytes(&value).map(Some)
}

/// Drain a WHATWG `ReadableStream` response body through its default
/// reader. Reading happens on the realm's scheduler, so an async
/// producer (a `pull` that awaits `delay()`, a `start` that enqueues
/// after a timer) keeps running while the read is parked; the run
/// bracket's deadline bounds a stream that never closes. A macro so the
/// awaits stay inline (a helper async fn would be non-Send).
macro_rules! drain_stream {
    ($ctx:expr, $stream:expr) => {{
        let (reader, read) = stream_reader($ctx, $stream)?;
        let mut body = Vec::new();
        loop {
            let step: Promise<'_> = read
                .call((This(reader.clone()),))
                .catch($ctx)
                .map_err(|e| caught($ctx, e))?;
            match read_step($ctx, step.into_future::<Object<'_>>().await)? {
                Some(chunk) => body.extend_from_slice(&chunk),
                None => break body,
            }
        }
    }};
}

/// Build a [`HandlerFn`] dispatching to the JS handler stored in `slot`.
pub fn build_handler_fn(
    engine: Weak<ScriptEngine>,
    slot: u64,
    bundle: Arc<CompiledModule>,
    kind: HandlerKind,
    is_generator: bool,
) -> HandlerFn {
    Arc::new(move |request: RequestContext| {
        let engine = Weak::clone(&engine);
        let bundle = Arc::clone(&bundle);
        Box::pin(async move {
            let engine = live_engine(&engine)?;
            let options = RunOptions {
                timeout: Some(engine.config().handler_timeout),
                ..RunOptions::default()
            };
            let run = engine
                .runtime()
                .run(
                    options,
                    Box::new(move |ctx| {
                        Box::pin(async move {
                            let pending = if is_generator {
                                begin_generator_step(&ctx, slot, request, kind)?
                            } else {
                                let func = restore_handler(&ctx, slot)?;
                                let req = request_value(&ctx, kind, request)?;
                                func.call((req,)).catch(&ctx).map_err(|e| caught(&ctx, e))?
                            };

                            let resolved = await_js!(&ctx, pending);

                            let resolved = if is_generator {
                                finish_generator_step(&ctx, slot, resolved)?
                            } else {
                                resolved
                            };

                            let (mut meta, stream) =
                                match value_to_dynamic_response(&ctx, resolved)? {
                                    ConvertedResponse::Ready(response) => return Ok(response),
                                    ConvertedResponse::Streaming { meta, stream } => (meta, stream),
                                };

                            let stream_value = stream.restore(&ctx).map_err(|e| {
                                ScriptError::internal(format!("restore stream: {e}"))
                            })?;
                            meta.body = bytes::Bytes::from(drain_stream!(&ctx, stream_value));
                            Ok::<DynamicResponse, ScriptError>(meta)
                        })
                    }),
                )
                .await;
            run.result.map_err(|e| remap_error(e, &bundle))
        })
    })
}
