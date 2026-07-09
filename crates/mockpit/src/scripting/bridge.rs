//! Bridges a persisted JS handler into mockpit's [`HandlerFn`] type.
//!
//! The closure is `Send + Sync` because it captures only the VM handle
//! and shared state; the JS function itself never leaves the VM — each
//! call submits a job that restores the `Persistent` by slot id, invokes
//! it with a lazy request object, awaits a returned promise, and
//! converts the resolved value.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rquickjs::function::This;
use rquickjs::{
    CatchResultExt, CaughtError, Class, Ctx, Function, Object, Persistent, Promise, Value,
};

use crate::types::{DynamicResponse, HandlerFn, RequestContext};
use crate::{MockpitError, vm_with};

use super::bindings::request::{GraphQLRequestInfo, RequestInfo};
use super::bindings::response::{ConvertedResponse, value_to_dynamic_response};
use super::bindings::streams;
use super::engine::TimeoutState;
use super::slots::with_slots;
use super::vm::VmHandle;

/// Slack on top of the handler budget before the tokio backstop frees
/// the request. The interrupt handler kills runaway bytecode at the
/// budget itself; the backstop only catches jobs parked on a host await
/// (e.g. `delay('infinite')`), which leave the heap healthy.
const TIMEOUT_BACKSTOP_GRACE: Duration = Duration::from_secs(1);

/// Which resolver info object the handler receives.
#[derive(Clone, Copy)]
pub enum HandlerKind {
    Http,
    GraphQL,
}

pub fn caught_to_error(caught: &CaughtError<'_>) -> MockpitError {
    match caught {
        CaughtError::Exception(ex) => {
            let message = ex.message().unwrap_or_else(|| "exception".to_string());
            match ex.stack() {
                Some(stack) if !stack.is_empty() => {
                    MockpitError::Script(format!("{message}\n{stack}"))
                }
                _ => MockpitError::Script(message),
            }
        }
        CaughtError::Value(v) => MockpitError::Script(format!("script threw: {v:?}")),
        CaughtError::Error(e) => MockpitError::Script(e.to_string()),
    }
}

pub(super) fn restore_handler<'js>(
    ctx: &Ctx<'js>,
    slot: u64,
) -> Result<Function<'js>, MockpitError> {
    let persistent = with_slots(ctx, |slots| slots.get(slot))
        .map_err(|e| MockpitError::Script(e.to_string()))?
        .ok_or_else(|| MockpitError::Script(format!("script handler slot {slot} is gone")))?;
    persistent
        .restore(ctx)
        .map_err(|e| MockpitError::Script(format!("restore handler: {e}")))
}

fn request_value<'js>(
    ctx: &Ctx<'js>,
    kind: HandlerKind,
    request: RequestContext,
) -> Result<Value<'js>, MockpitError> {
    let value = match kind {
        HandlerKind::Http => Class::instance(ctx.clone(), RequestInfo::new(request))
            .catch(ctx)
            .map_err(|e| caught_to_error(&e))?
            .as_value()
            .clone(),
        HandlerKind::GraphQL => Class::instance(ctx.clone(), GraphQLRequestInfo::new(request))
            .catch(ctx)
            .map_err(|e| caught_to_error(&e))?
            .as_value()
            .clone(),
    };
    Ok(value)
}

/// Kick off one step of a generator resolver: create the iterator on the
/// first request (calling the generator function with the resolver
/// info), then call `next()`. The returned value may be a promise
/// (async generators) — the caller awaits it before
/// [`finish_generator_step`].
fn begin_generator_step<'js>(
    ctx: &Ctx<'js>,
    slot: u64,
    request: RequestContext,
    kind: HandlerKind,
) -> Result<Value<'js>, MockpitError> {
    let existing = with_slots(ctx, |slots| slots.iterator(slot))
        .map_err(|e| MockpitError::Script(e.to_string()))?;
    let iterator = if let Some(persistent) = existing {
        persistent
            .restore(ctx)
            .map_err(|e| MockpitError::Script(format!("restore iterator: {e}")))?
    } else {
        let func = restore_handler(ctx, slot)?;
        let req = request_value(ctx, kind, request)?;
        let iterator: Object<'js> = func
            .call((req,))
            .catch(ctx)
            .map_err(|e| caught_to_error(&e))?;
        let persistent = Persistent::save(ctx, iterator.clone());
        with_slots(ctx, |slots| slots.set_iterator(slot, persistent))
            .map_err(|e| MockpitError::Script(e.to_string()))?;
        iterator
    };

    let next: Function<'js> = iterator
        .get("next")
        .map_err(|e| MockpitError::Script(format!("generator iterator has no next(): {e}")))?;
    next.call((This(iterator.clone()),))
        .catch(ctx)
        .map_err(|e| caught_to_error(&e))
}

/// Interpret an awaited generator step: unwrap `{ value, done }`, keep
/// the last yielded value, and repeat it after exhaustion (MSW
/// generator-resolver semantics).
fn finish_generator_step<'js>(
    ctx: &Ctx<'js>,
    slot: u64,
    step: Value<'js>,
) -> Result<Value<'js>, MockpitError> {
    let step_obj = step
        .into_object()
        .ok_or_else(|| MockpitError::Script("generator step is not an object".to_string()))?;
    let done: bool = step_obj.get("done").unwrap_or(false);
    let value: Value<'js> = step_obj
        .get("value")
        .map_err(|e| MockpitError::Script(format!("generator step value: {e}")))?;

    if value.is_undefined() && done {
        // Exhausted: repeat the last yielded value.
        if let Some(last) = with_slots(ctx, |slots| slots.last_value(slot))
            .map_err(|e| MockpitError::Script(e.to_string()))?
        {
            return last
                .restore(ctx)
                .map_err(|e| MockpitError::Script(format!("restore generator value: {e}")));
        }
        return Ok(value);
    }

    if !value.is_undefined() {
        let persistent = Persistent::save(ctx, value.clone());
        with_slots(ctx, |slots| slots.set_last_value(slot, persistent))
            .map_err(|e| MockpitError::Script(e.to_string()))?;
    }
    Ok(value)
}

/// Await a possibly-promise JS value inside the VM closure. A macro so
/// the await stays inline (rquickjs futures are single-threaded; a
/// helper async fn would be non-Send).
macro_rules! await_js {
    ($ctx:expr, $value:expr) => {{
        let value: Value<'_> = $value;
        if let Some(promise) = value.as_promise() {
            let promise: Promise<'_> = promise.clone();
            match promise.into_future::<Value<'_>>().await.catch($ctx) {
                Ok(v) => v,
                Err(e) => return Err(caught_to_error(&e)),
            }
        } else {
            value
        }
    }};
}

/// Build a [`HandlerFn`] dispatching to the JS handler stored in `slot`.
#[allow(clippy::too_many_arguments)]
pub fn build_handler_fn(
    vm: VmHandle,
    slot: u64,
    timeout: Arc<TimeoutState>,
    poisoned: Arc<AtomicBool>,
    budget: Duration,
    bundle: Arc<super::bundle::CompiledBundle>,
    kind: HandlerKind,
    is_generator: bool,
) -> HandlerFn {
    Arc::new(move |request: RequestContext| {
        let vm = vm.clone();
        let timeout = Arc::clone(&timeout);
        let poisoned = Arc::clone(&poisoned);
        let bundle = Arc::clone(&bundle);
        Box::pin(async move {
            if poisoned.load(Ordering::Relaxed) {
                return Err(MockpitError::Script(
                    "script engine is poisoned (previous timeout/OOM); reload the script file"
                        .to_string(),
                ));
            }

            timeout.arm(budget);
            let eval = vm_with!(vm => |ctx| {
                let pending = if is_generator {
                    match begin_generator_step(&ctx, slot, request, kind) {
                        Ok(v) => v,
                        Err(e) => return Err(e),
                    }
                } else {
                    let func = match restore_handler(&ctx, slot) {
                        Ok(f) => f,
                        Err(e) => return Err(e),
                    };
                    let req = match request_value(&ctx, kind, request) {
                        Ok(r) => r,
                        Err(e) => return Err(e),
                    };
                    match func.call((req,)).catch(&ctx) {
                        Ok(v) => v,
                        Err(e) => return Err(caught_to_error(&e)),
                    }
                };

                let resolved = await_js!(&ctx, pending);

                let resolved = if is_generator {
                    match finish_generator_step(&ctx, slot, resolved) {
                        Ok(v) => v,
                        Err(e) => return Err(e),
                    }
                } else {
                    resolved
                };

                let (mut meta, stream) = match value_to_dynamic_response(&ctx, resolved) {
                    Ok(ConvertedResponse::Ready(response)) => return Ok(response),
                    Ok(ConvertedResponse::Streaming { meta, stream }) => (meta, stream),
                    Err(e) => return Err(e),
                };

                // Streamed body: drain the native ReadableStream. The
                // parked wait future is woken by enqueue/close/error, so
                // async producers (delay(), pull callbacks) progress on
                // the VM scheduler while we wait.
                let stream_value = match stream.restore(&ctx) {
                    Ok(v) => v,
                    Err(e) => return Err(MockpitError::Script(format!("restore stream: {e}"))),
                };
                let Some(handle) = streams::as_stream(&stream_value) else {
                    return Err(MockpitError::Script(
                        "response body stream is not a ReadableStream".to_string(),
                    ));
                };

                if let Some(start) = handle.start_result {
                    let start_value = match start.restore(&ctx) {
                        Ok(v) => v,
                        Err(e) => {
                            return Err(MockpitError::Script(format!("restore stream start: {e}")));
                        }
                    };
                    let _ = await_js!(&ctx, start_value);
                }

                let mut body = Vec::new();
                // Guards a sync pull that makes no progress from busy-looping:
                // park until the state changes instead of re-pulling.
                let mut can_pull = true;
                loop {
                    let step = streams::drain_step(&handle.state);
                    if !step.chunks.is_empty() {
                        can_pull = true;
                    }
                    for chunk in step.chunks {
                        body.extend_from_slice(&chunk);
                    }
                    if let Some(error) = step.errored {
                        return Err(MockpitError::Script(format!(
                            "response stream errored: {error}"
                        )));
                    }
                    if step.closed {
                        break;
                    }

                    if can_pull
                        && let (Some(pull), Some(controller)) = (&handle.pull, &handle.controller)
                    {
                        let pull_fn = match pull.clone().restore(&ctx) {
                            Ok(f) => f,
                            Err(e) => {
                                return Err(MockpitError::Script(format!(
                                    "restore stream pull: {e}"
                                )));
                            }
                        };
                        let controller_value = match controller.clone().restore(&ctx) {
                            Ok(v) => v,
                            Err(e) => {
                                return Err(MockpitError::Script(format!(
                                    "restore stream controller: {e}"
                                )));
                            }
                        };
                        let result: Value<'_> = match pull_fn.call((controller_value,)).catch(&ctx)
                        {
                            Ok(v) => v,
                            Err(e) => return Err(caught_to_error(&e)),
                        };
                        let _ = await_js!(&ctx, result);
                        can_pull = false;
                        continue;
                    }

                    // Park until enqueue/close/error (producers keep
                    // running on the VM scheduler — timers, delay(),
                    // pending pull promises). The handler timeout
                    // backstop bounds a stream that never closes.
                    streams::StreamReady::new(&handle.state).await;
                    can_pull = true;
                }

                meta.body = bytes::Bytes::from(body);
                Ok(meta)
            });

            let backstop = budget.saturating_add(TIMEOUT_BACKSTOP_GRACE);
            let outcome: Result<DynamicResponse, MockpitError> =
                match tokio::time::timeout(backstop, eval).await {
                    Ok(result) => result.and_then(|inner| inner),
                    Err(_) => Err(MockpitError::Script(format!(
                        "script handler timed out after {}ms",
                        backstop.as_millis()
                    ))),
                };

            // An interrupt force-halt leaves the heap untrustworthy.
            if timeout.timed_out.swap(false, Ordering::Relaxed) {
                poisoned.store(true, Ordering::Relaxed);
            }
            timeout.disarm(budget);
            // Bundled stack positions -> original .ts/.js positions.
            outcome.map_err(|e| super::bundle::remap_error(e, &bundle))
        })
    })
}
