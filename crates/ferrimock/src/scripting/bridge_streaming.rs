//! Bridges persisted JS `sse()`/`ws.link()` resolvers into the engine's
//! streaming handler types.
//!
//! The per-connection forwarding logic (upstream passthrough, MSW
//! preventDefault semantics) lives in [`crate::streaming`]; this module
//! only supplies the dispatch callbacks that run listeners as VM jobs.
//!
//! Unlike request/response handlers these run UNARMED (no interrupt
//! budget): the engine's timeout poisons the whole VM when any bytecode
//! outlives the armed deadline, which is engine-fatal around a
//! long-lived connection. A synchronous infinite loop in a streaming
//! callback therefore wedges that one file's engine — per-file isolation
//! bounds the blast radius and a hot reload replaces it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rquickjs::function::Func;
use rquickjs::{CatchResultExt, Class, Ctx, Object, Promise, TypedArray, Value};
use tokio::sync::mpsc;

use crate::streaming::{
    SseUpstreamCmd, SseUpstreamEvent, WsDispatchFn, WsDriverEvent, WsUpstreamCmd,
};
use crate::types::{
    RequestContext, SseHandlerFn, SseMessage, SseSinkMsg, WsConnection, WsFrame, WsHandlerFn,
    WsOutbound,
};
use crate::{FerrimockError, vm_with};

use super::bindings::request::RequestInfo;
use super::bindings::sse::{SseClient, SseServer};
use super::bindings::ws::{WsClient, WsServer};
use super::bridge::{caught_to_error, restore_handler};
use super::slots::with_slots;
use super::vm::VmHandle;

/// Await a possibly-promise JS value inside the VM closure (mirror of
/// bridge.rs's macro; rquickjs futures are single-threaded so it must
/// stay inline).
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

fn poisoned_error() -> FerrimockError {
    FerrimockError::Script(
        "script engine is poisoned (previous timeout/OOM); reload the script file".to_string(),
    )
}

#[allow(clippy::needless_pass_by_value)]
fn prevent_default(this: rquickjs::function::This<Object<'_>>) -> rquickjs::Result<()> {
    this.set("defaultPrevented", true)
}

/// Build the JS `data` value for a frame: string or Uint8Array.
fn frame_to_js<'js>(ctx: &Ctx<'js>, frame: &WsFrame) -> rquickjs::Result<Value<'js>> {
    match frame {
        WsFrame::Text(text) => Ok(rquickjs::String::from_str(ctx.clone(), text)?.into_value()),
        WsFrame::Binary(bytes) => Ok(TypedArray::<u8>::new_copy(ctx.clone(), &bytes[..])?
            .as_value()
            .clone()),
    }
}

/// Build an Event-shaped object with working `preventDefault()`.
fn base_event<'js>(ctx: &Ctx<'js>, event_type: &str) -> rquickjs::Result<Object<'js>> {
    let event = Object::new(ctx.clone())?;
    event.set("type", event_type)?;
    event.set("defaultPrevented", false)?;
    event.set("preventDefault", Func::from(prevent_default))?;
    Ok(event)
}

// ---------------------------------------------------------------------------
// SSE
// ---------------------------------------------------------------------------

/// Run one SSE upstream-event dispatch on the VM (EventSource
/// semantics: named frames dispatch only to same-named listeners,
/// unnamed frames as `message`); returns `defaultPrevented`.
async fn dispatch_sse_event(
    vm: &VmHandle,
    conn_id: u64,
    event_name: String,
    frame: Option<SseMessage>,
) -> Result<bool, FerrimockError> {
    let vm = vm.clone();
    vm_with!(vm => |ctx| {
        let listeners = match with_slots(&ctx, |slots| slots.sse_listeners(conn_id, &event_name)) {
            Ok(l) => l,
            Err(e) => return Err(FerrimockError::Script(e.to_string())),
        };
        if listeners.is_empty() {
            return Ok(false);
        }
        let event = match base_event(&ctx, &event_name) {
            Ok(e) => e,
            Err(e) => return Err(FerrimockError::Script(e.to_string())),
        };
        if let Some(frame) = &frame {
            let result = event
                .set("data", frame.data.as_str())
                .and_then(|()| event.set("lastEventId", frame.id.as_deref().unwrap_or("")));
            if let Err(e) = result {
                return Err(FerrimockError::Script(e.to_string()));
            }
        }
        for listener in listeners {
            let func = match listener.restore(&ctx) {
                Ok(f) => f,
                Err(e) => return Err(FerrimockError::Script(format!("restore listener: {e}"))),
            };
            let result: Value<'_> = match func.call((event.clone(),)).catch(&ctx) {
                Ok(v) => v,
                Err(e) => return Err(caught_to_error(&e)),
            };
            let _ = await_js!(&ctx, result);
        }
        Ok(event.get("defaultPrevented").unwrap_or(false))
    })
    .await
    .and_then(|inner| inner)
}

/// Build an [`SseHandlerFn`] dispatching to the resolver in `slot`.
/// `upstream_url` is the handler's absolute http(s) URL when it has one
/// — the real endpoint `server.connect()` dials.
pub fn build_sse_handler_fn(
    vm: VmHandle,
    slot: u64,
    poisoned: Arc<AtomicBool>,
    bundle: Arc<super::bundle::CompiledBundle>,
    upstream_url: Option<String>,
) -> SseHandlerFn {
    Arc::new(
        move |request: RequestContext, tx: mpsc::UnboundedSender<SseSinkMsg>| {
            let vm = vm.clone();
            let poisoned = Arc::clone(&poisoned);
            let bundle = Arc::clone(&bundle);
            let upstream_url = upstream_url.clone();
            Box::pin(async move {
                if poisoned.load(Ordering::Relaxed) {
                    return Err(poisoned_error());
                }

                let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<SseUpstreamCmd>();
                let resolver_tx = tx.clone();
                let resolver_cmd = cmd_tx.clone();
                let resolver_url = upstream_url.clone();

                let init: Result<u64, FerrimockError> = vm_with!(vm => |ctx| {
                    let conn_id =
                        match with_slots(&ctx, super::slots::HandlerSlots::new_sse_connection) {
                            Ok(id) => id,
                            Err(e) => return Err(FerrimockError::Script(e.to_string())),
                        };

                    let func = match restore_handler(&ctx, slot) {
                        Ok(f) => f,
                        Err(e) => return Err(e),
                    };

                    let info =
                        match Class::instance(ctx.clone(), RequestInfo::new(request)).catch(&ctx) {
                            Ok(i) => i.as_value().clone(),
                            Err(e) => return Err(caught_to_error(&e)),
                        };
                    let Some(info_obj) = info.as_object() else {
                        return Err(FerrimockError::Script("resolver info is not an object".into()));
                    };

                    let client =
                        match Class::instance(ctx.clone(), SseClient::new(resolver_tx)).catch(&ctx)
                        {
                            Ok(c) => c.as_value().clone(),
                            Err(e) => return Err(caught_to_error(&e)),
                        };
                    let server = match Class::instance(
                        ctx.clone(),
                        SseServer::new(conn_id, resolver_cmd, resolver_url),
                    )
                    .catch(&ctx)
                    {
                        Ok(s) => s.as_value().clone(),
                        Err(e) => return Err(caught_to_error(&e)),
                    };
                    if let Err(e) = info_obj
                        .set("client", client)
                        .and_then(|()| info_obj.set("server", server))
                    {
                        return Err(FerrimockError::Script(e.to_string()));
                    }

                    let pending: Value<'_> = match func.call((info.clone(),)).catch(&ctx) {
                        Ok(v) => v,
                        Err(e) => return Err(caught_to_error(&e)),
                    };
                    let _ = await_js!(&ctx, pending);
                    Ok(conn_id)
                })
                .await
                .and_then(|inner| inner);

                let conn_id = match init {
                    Ok(id) => id,
                    Err(e) => return Err(super::bundle::remap_error(e, &bundle)),
                };

                // Host loop: lives until the client disconnects (the sink
                // receiver drops) so `server.connect()` — even one issued
                // asynchronously after the resolver returned — can pump
                // upstream frames into the connection.
                let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<SseUpstreamEvent>();
                let mut pump: Option<tokio::task::JoinHandle<()>> = None;

                let result: Result<(), FerrimockError> = loop {
                    tokio::select! {
                        () = tx.closed() => break Ok(()),
                        cmd = cmd_rx.recv() => match cmd {
                            Some(SseUpstreamCmd::Connect) => {
                                if pump.is_none() && let Some(url) = &upstream_url {
                                    pump = Some(tokio::spawn(crate::streaming::run_sse_upstream(
                                        url.clone(),
                                        None,
                                        evt_tx.clone(),
                                    )));
                                }
                            }
                            Some(SseUpstreamCmd::Close) => {
                                if let Some(handle) = pump.take() {
                                    handle.abort();
                                }
                            }
                            None => {}
                        },
                        event = evt_rx.recv() => match event {
                            Some(SseUpstreamEvent::Open) => {
                                if let Err(e) =
                                    dispatch_sse_event(&vm, conn_id, "open".to_string(), None).await
                                {
                                    break Err(e);
                                }
                            }
                            Some(SseUpstreamEvent::Frame(frame)) => {
                                // Bare retry frames only adjust the pump's
                                // reconnect delay; EventSource dispatches
                                // and forwards nothing for them.
                                if frame.retry.is_some()
                                    && frame.data.is_empty()
                                    && frame.id.is_none()
                                    && frame.event.is_none()
                                {
                                    continue;
                                }
                                let event_name = frame
                                    .event
                                    .clone()
                                    .unwrap_or_else(|| "message".to_string());
                                match dispatch_sse_event(&vm, conn_id, event_name, Some(frame.clone()))
                                    .await
                                {
                                    Ok(prevented) => {
                                        if !prevented {
                                            let forwarded = SseMessage {
                                                event: frame
                                                    .event
                                                    .filter(|name| name != "message"),
                                                ..frame
                                            };
                                            let _ = tx.send(SseSinkMsg::Message(forwarded));
                                        }
                                    }
                                    Err(e) => break Err(e),
                                }
                            }
                            Some(SseUpstreamEvent::Error(message)) => {
                                tracing::warn!("sse upstream error: {message}");
                                if let Err(e) =
                                    dispatch_sse_event(&vm, conn_id, "error".to_string(), None).await
                                {
                                    break Err(e);
                                }
                            }
                            Some(SseUpstreamEvent::Reconnecting) => {
                                // EventSource surfaces a lost connection as
                                // an error event; the pump redials on its
                                // own with Last-Event-ID.
                                if let Err(e) =
                                    dispatch_sse_event(&vm, conn_id, "error".to_string(), None).await
                                {
                                    break Err(e);
                                }
                            }
                            None => {}
                        },
                    }
                };

                if let Some(handle) = pump.take() {
                    handle.abort();
                }

                // Persistent leak guard: drop the connection's listeners.
                let vm_cleanup = vm.clone();
                let _ = vm_with!(vm_cleanup => |ctx| {
                    let _ = with_slots(&ctx, |slots| slots.remove_sse_connection(conn_id));
                    Ok::<(), FerrimockError>(())
                })
                .await;

                result.map_err(|e| super::bundle::remap_error(e, &bundle))
            })
        },
    )
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

/// Build the `{ client, server, params, info }` argument for connection
/// listeners.
fn build_connection_arg<'js>(
    ctx: &Ctx<'js>,
    conn_id: u64,
    outbound: mpsc::UnboundedSender<WsOutbound>,
    cmd: mpsc::UnboundedSender<WsUpstreamCmd>,
    link_url: Option<String>,
    client_url: String,
    captures: &rustc_hash::FxHashMap<String, String>,
    protocols: &[String],
) -> rquickjs::Result<Object<'js>> {
    let client = Class::instance(ctx.clone(), WsClient::new(conn_id, outbound, client_url))?;
    let server = Class::instance(ctx.clone(), WsServer::new(conn_id, cmd, link_url))?;
    let params = Object::new(ctx.clone())?;
    for (k, v) in crate::types::msw_params(captures) {
        match v {
            crate::types::MswParamValue::Single(s) => params.set(k, s)?,
            crate::types::MswParamValue::List(l) => params.set(k, l)?,
        }
    }
    let info = Object::new(ctx.clone())?;
    let protocol_list = rquickjs::Array::new(ctx.clone())?;
    for (index, protocol) in protocols.iter().enumerate() {
        protocol_list.set(index, protocol.as_str())?;
    }
    info.set("protocols", protocol_list)?;

    let arg = Object::new(ctx.clone())?;
    arg.set("client", client)?;
    arg.set("server", server)?;
    arg.set("params", params)?;
    arg.set("info", info)?;
    Ok(arg)
}

/// Run one connection-event dispatch on the VM: call every listener for
/// `event_key` with a fresh event object; returns `defaultPrevented`.
async fn dispatch_ws_event(
    vm: &VmHandle,
    conn_id: u64,
    event_key: &'static str,
    event_type: &'static str,
    frame: Option<WsFrame>,
    close: Option<(Option<u16>, Option<String>)>,
) -> Result<bool, FerrimockError> {
    let vm = vm.clone();
    vm_with!(vm => |ctx| {
        let listeners = match with_slots(&ctx, |slots| slots.ws_connection_listeners(conn_id, event_key)) {
            Ok(l) => l,
            Err(e) => return Err(FerrimockError::Script(e.to_string())),
        };
        if listeners.is_empty() {
            return Ok(false);
        }
        let event = match base_event(&ctx, event_type) {
            Ok(e) => e,
            Err(e) => return Err(FerrimockError::Script(e.to_string())),
        };
        if let Some(frame) = &frame {
            let data = match frame_to_js(&ctx, frame) {
                Ok(v) => v,
                Err(e) => return Err(FerrimockError::Script(e.to_string())),
            };
            if let Err(e) = event.set("data", data) {
                return Err(FerrimockError::Script(e.to_string()));
            }
        }
        if let Some((code, reason)) = &close {
            let result = event
                .set("code", code.unwrap_or(1000))
                .and_then(|()| event.set("reason", reason.as_deref().unwrap_or("")))
                .and_then(|()| event.set("wasClean", code.is_none_or(|c| c == 1000)));
            if let Err(e) = result {
                return Err(FerrimockError::Script(e.to_string()));
            }
        }
        for listener in listeners {
            let func = match listener.restore(&ctx) {
                Ok(f) => f,
                Err(e) => return Err(FerrimockError::Script(format!("restore listener: {e}"))),
            };
            let result: Value<'_> = match func.call((event.clone(),)).catch(&ctx) {
                Ok(v) => v,
                Err(e) => return Err(caught_to_error(&e)),
            };
            let _ = await_js!(&ctx, result);
        }
        Ok(event.get("defaultPrevented").unwrap_or(false))
    })
    .await
    .and_then(|inner| inner)
}

/// Build a [`WsHandlerFn`] dispatching connections to the link's
/// listeners in `link_slot`. The forwarding loop is
/// [`crate::streaming::drive_ws_connection`]; this bridge only turns
/// driver events into VM jobs.
pub fn build_ws_handler_fn(
    vm: VmHandle,
    link_slot: u64,
    poisoned: Arc<AtomicBool>,
    bundle: Arc<super::bundle::CompiledBundle>,
    link_url: Option<String>,
) -> WsHandlerFn {
    Arc::new(move |connection: WsConnection| {
        let vm = vm.clone();
        let poisoned = Arc::clone(&poisoned);
        let bundle = Arc::clone(&bundle);
        let link_url = link_url.clone();
        Box::pin(async move {
            if poisoned.load(Ordering::Relaxed) {
                return Err(poisoned_error());
            }

            // Set while dispatching the Connection event; every later
            // event addresses this connection's listener table.
            let conn_cell = Arc::new(std::sync::OnceLock::<u64>::new());

            let dispatch_vm = vm.clone();
            let dispatch_cell = Arc::clone(&conn_cell);
            let dispatch_link_url = link_url.clone();
            let dispatch: WsDispatchFn = Arc::new(move |event: WsDriverEvent| {
                let vm = dispatch_vm.clone();
                let conn_cell = Arc::clone(&dispatch_cell);
                let link_url = dispatch_link_url.clone();
                Box::pin(async move {
                    match event {
                        WsDriverEvent::Connection(seed) => {
                            // Reconstruct the client-facing URL from the
                            // handshake.
                            let client_url = {
                                let host = seed
                                    .request
                                    .headers
                                    .get("host")
                                    .map_or("localhost", String::as_str);
                                format!("ws://{host}{}", seed.request.uri)
                            };
                            let captures = seed.request.captures.clone();
                            let protocols = seed.protocols.clone();
                            let outbound = seed.outbound.clone();
                            let upstream = seed.upstream.clone();

                            let conn_id = vm_with!(vm => |ctx| {
                                let conn_id = match with_slots(
                                    &ctx,
                                    super::slots::HandlerSlots::new_ws_connection,
                                ) {
                                    Ok(id) => id,
                                    Err(e) => return Err(FerrimockError::Script(e.to_string())),
                                };

                                let arg = match build_connection_arg(
                                    &ctx,
                                    conn_id,
                                    outbound,
                                    upstream,
                                    link_url,
                                    client_url,
                                    &captures,
                                    &protocols,
                                ) {
                                    Ok(arg) => arg,
                                    Err(e) => return Err(FerrimockError::Script(e.to_string())),
                                };

                                let listeners =
                                    match with_slots(&ctx, |slots| slots.ws_link_listeners(link_slot)) {
                                        Ok(l) => l,
                                        Err(e) => return Err(FerrimockError::Script(e.to_string())),
                                    };
                                for listener in listeners {
                                    let func = match listener.restore(&ctx) {
                                        Ok(f) => f,
                                        Err(e) => {
                                            return Err(FerrimockError::Script(format!(
                                                "restore listener: {e}"
                                            )));
                                        }
                                    };
                                    let result: Value<'_> =
                                        match func.call((arg.clone(),)).catch(&ctx) {
                                            Ok(v) => v,
                                            Err(e) => return Err(caught_to_error(&e)),
                                        };
                                    let _ = await_js!(&ctx, result);
                                }
                                Ok(conn_id)
                            })
                            .await
                            .and_then(|inner| inner)?;

                            let _ = conn_cell.set(conn_id);
                            Ok(false)
                        }
                        other => {
                            let Some(&conn_id) = conn_cell.get() else {
                                return Ok(false);
                            };
                            match other {
                                WsDriverEvent::Connection(_) => Ok(false),
                                WsDriverEvent::Message(frame) => {
                                    dispatch_ws_event(
                                        &vm,
                                        conn_id,
                                        "message",
                                        "message",
                                        Some(frame),
                                        None,
                                    )
                                    .await
                                }
                                WsDriverEvent::Close { code, reason } => {
                                    dispatch_ws_event(
                                        &vm,
                                        conn_id,
                                        "close",
                                        "close",
                                        None,
                                        Some((code, reason)),
                                    )
                                    .await
                                }
                                WsDriverEvent::ServerOpen => {
                                    dispatch_ws_event(
                                        &vm,
                                        conn_id,
                                        "server-open",
                                        "open",
                                        None,
                                        None,
                                    )
                                    .await
                                }
                                WsDriverEvent::ServerMessage(frame) => {
                                    dispatch_ws_event(
                                        &vm,
                                        conn_id,
                                        "server-message",
                                        "message",
                                        Some(frame),
                                        None,
                                    )
                                    .await
                                }
                                WsDriverEvent::ServerError(_) => {
                                    dispatch_ws_event(
                                        &vm,
                                        conn_id,
                                        "server-error",
                                        "error",
                                        None,
                                        None,
                                    )
                                    .await
                                }
                                WsDriverEvent::ServerClose => {
                                    dispatch_ws_event(
                                        &vm,
                                        conn_id,
                                        "server-close",
                                        "close",
                                        None,
                                        Some((Some(1000), None)),
                                    )
                                    .await
                                }
                            }
                        }
                    }
                })
            });

            let result =
                crate::streaming::drive_ws_connection(connection, link_url, dispatch).await;

            // Persistent leak guard: drop the connection's listeners.
            if let Some(&conn_id) = conn_cell.get() {
                let vm_cleanup = vm.clone();
                let _ = vm_with!(vm_cleanup => |ctx| {
                    let _ = with_slots(&ctx, |slots| slots.remove_ws_connection(conn_id));
                    Ok::<(), FerrimockError>(())
                })
                .await;
            }

            result.map_err(|e| super::bundle::remap_error(e, &bundle))
        })
    })
}
