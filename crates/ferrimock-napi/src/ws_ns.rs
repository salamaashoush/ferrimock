//! `ws` namespace: native WebSocket handler registration.
//!
//! `ws.handler(url, dispatch)` builds a real engine `MockDefinition`
//! whose `streaming` facet bridges connection events to the JS dispatch
//! callback over a ThreadsafeFunction. The per-connection forwarding
//! logic (upstream passthrough, preventDefault semantics) is
//! `ferrimock::streaming::drive_ws_connection` — the same driver behind
//! the QuickJS lane — so `FerrimockServer.listen()` serves JS-defined
//! `ws.link` handlers natively. The JS side (`ferrimock`) wraps this
//! into MSW's `ws.link(...).addEventListener("connection", ...)` shape.

use crate::http_ns::{RequestHandler, as_regexp, compile_js_regex};
use ferrimock::streaming::{WsDispatchFn, WsDriverEvent, WsUpstreamCmd, drive_ws_connection};
use ferrimock::types::{
    BodySource, HeaderMatcher, MockDefinition, ResponseGenerator, UrlPattern, WsConnection,
    WsFrame, WsHandlerFn, WsOutbound,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

/// Process-wide connection ids so the JS dispatcher can key its
/// per-connection state.
static CONNECTION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn frame_from_js(data: Either<String, Uint8Array>) -> WsFrame {
    match data {
        Either::A(text) => WsFrame::Text(text),
        Either::B(bytes) => WsFrame::Binary(bytes::Bytes::from(bytes.to_vec())),
    }
}

/// The intercepted client half of a live TCP-lane connection: sends go
/// straight into the server socket pump.
#[napi]
pub struct WebSocketClientHandle {
    outbound: mpsc::UnboundedSender<WsOutbound>,
    id: String,
    url: String,
}

#[napi]
impl WebSocketClientHandle {
    #[napi(getter)]
    pub fn id(&self) -> String {
        self.id.clone()
    }

    #[napi(getter)]
    pub fn url(&self) -> String {
        self.url.clone()
    }

    /// Send text (string) or binary (Uint8Array) to the client.
    #[napi]
    pub fn send(&self, data: Either<String, Uint8Array>) {
        let _ = self.outbound.send(WsOutbound::Frame(frame_from_js(data)));
    }

    #[napi]
    pub fn close(&self, code: Option<u16>, reason: Option<String>) -> Result<()> {
        if let Some(code) = code
            && !(1000..=4999).contains(&code)
        {
            return Err(Error::new(
                Status::InvalidArg,
                format!("Invalid WebSocket close code: {code}"),
            ));
        }
        let _ = self.outbound.send(WsOutbound::Close { code, reason });
        Ok(())
    }
}

/// The real upstream half of a live TCP-lane connection, idle until
/// `connect()` is called.
#[napi]
pub struct WebSocketServerHandle {
    cmd: mpsc::UnboundedSender<WsUpstreamCmd>,
    can_connect: bool,
}

#[napi]
impl WebSocketServerHandle {
    /// Dial the real server (the link's absolute ws(s):// URL) and
    /// start forwarding.
    #[napi]
    pub fn connect(&self) -> Result<()> {
        if !self.can_connect {
            return Err(Error::from_reason(
                "ws server.connect() needs an absolute link URL (ws://host/path) to know the real server",
            ));
        }
        let _ = self.cmd.send(WsUpstreamCmd::Connect);
        Ok(())
    }

    /// Send to the real server.
    #[napi]
    pub fn send(&self, data: Either<String, Uint8Array>) {
        let _ = self.cmd.send(WsUpstreamCmd::Send(frame_from_js(data)));
    }

    #[napi]
    pub fn close(&self) {
        let _ = self.cmd.send(WsUpstreamCmd::Close);
    }
}

/// One connection event crossing to the JS dispatcher. Frames cross as
/// `{ text | bytes }`; sends come back over the handle classes' channels.
pub enum WsBridgeEvent {
    Connection {
        connection_id: String,
        url: String,
        params: HashMap<String, Either<String, Vec<String>>>,
        protocols: Vec<String>,
        client: WebSocketClientHandle,
        server: WebSocketServerHandle,
    },
    Message {
        connection_id: String,
        frame: WsFrame,
    },
    Close {
        connection_id: String,
        code: Option<u16>,
        reason: Option<String>,
    },
    ServerOpen {
        connection_id: String,
    },
    ServerMessage {
        connection_id: String,
        frame: WsFrame,
    },
    ServerError {
        connection_id: String,
        message: String,
    },
    ServerClose {
        connection_id: String,
    },
}

fn set_frame(obj: &mut Object, frame: WsFrame) -> Result<()> {
    match frame {
        WsFrame::Text(text) => obj.set("data", text),
        WsFrame::Binary(bytes) => obj.set("data", Uint8Array::from(bytes.to_vec())),
    }
}

impl ToNapiValue for WsBridgeEvent {
    #[allow(unsafe_code)]
    unsafe fn to_napi_value(
        raw_env: napi::sys::napi_env,
        val: Self,
    ) -> Result<napi::sys::napi_value> {
        // raw_env is the live env of the TSFN callback.
        let env = Env::from_raw(raw_env);
        let mut obj = Object::new(&env)?;
        match val {
            WsBridgeEvent::Connection {
                connection_id,
                url,
                params,
                protocols,
                client,
                server,
            } => {
                obj.set("type", "connection")?;
                obj.set("connectionId", connection_id)?;
                obj.set("url", url)?;
                obj.set("params", params)?;
                obj.set("protocols", protocols)?;
                obj.set("client", client)?;
                obj.set("server", server)?;
            }
            WsBridgeEvent::Message {
                connection_id,
                frame,
            } => {
                obj.set("type", "message")?;
                obj.set("connectionId", connection_id)?;
                set_frame(&mut obj, frame)?;
            }
            WsBridgeEvent::Close {
                connection_id,
                code,
                reason,
            } => {
                obj.set("type", "close")?;
                obj.set("connectionId", connection_id)?;
                obj.set("code", code.unwrap_or(1000))?;
                obj.set("reason", reason.unwrap_or_default())?;
            }
            WsBridgeEvent::ServerOpen { connection_id } => {
                obj.set("type", "server-open")?;
                obj.set("connectionId", connection_id)?;
            }
            WsBridgeEvent::ServerMessage {
                connection_id,
                frame,
            } => {
                obj.set("type", "server-message")?;
                obj.set("connectionId", connection_id)?;
                set_frame(&mut obj, frame)?;
            }
            WsBridgeEvent::ServerError {
                connection_id,
                message,
            } => {
                obj.set("type", "server-error")?;
                obj.set("connectionId", connection_id)?;
                obj.set("message", message)?;
            }
            WsBridgeEvent::ServerClose { connection_id } => {
                obj.set("type", "server-close")?;
                obj.set("connectionId", connection_id)?;
            }
        }
        // SAFETY: delegates to the generated Object conversion.
        unsafe { Object::to_napi_value(raw_env, obj) }
    }
}

/// TSFN type: the JS dispatcher receives one event and resolves with
/// whether a listener called `preventDefault()`.
type WsDispatchTsfn = napi::threadsafe_function::ThreadsafeFunction<
    WsBridgeEvent,
    Promise<bool>,
    WsBridgeEvent,
    Status,
    false, // callee_handled
    true,  // weak
    0,     // unbounded queue
>;

fn build_dispatch(
    tsfn: Arc<WsDispatchTsfn>,
    connection_id: String,
    upstream_url: Option<String>,
) -> WsDispatchFn {
    Arc::new(move |event: WsDriverEvent| {
        let tsfn = Arc::clone(&tsfn);
        let connection_id = connection_id.clone();
        let upstream_url = upstream_url.clone();
        Box::pin(async move {
            let bridge_event = match event {
                WsDriverEvent::Connection(seed) => {
                    let host = seed
                        .request
                        .headers
                        .get("host")
                        .map_or("localhost", String::as_str);
                    let url = format!("ws://{host}{}", seed.request.uri);
                    WsBridgeEvent::Connection {
                        connection_id: connection_id.clone(),
                        url: url.clone(),
                        params: crate::request_context::msw_params_map(&seed.request.captures),
                        protocols: seed.protocols.clone(),
                        client: WebSocketClientHandle {
                            outbound: seed.outbound.clone(),
                            id: connection_id,
                            url,
                        },
                        server: WebSocketServerHandle {
                            cmd: seed.upstream.clone(),
                            can_connect: upstream_url.is_some(),
                        },
                    }
                }
                WsDriverEvent::Message(frame) => WsBridgeEvent::Message {
                    connection_id,
                    frame,
                },
                WsDriverEvent::Close { code, reason } => WsBridgeEvent::Close {
                    connection_id,
                    code,
                    reason,
                },
                WsDriverEvent::ServerOpen => WsBridgeEvent::ServerOpen { connection_id },
                WsDriverEvent::ServerMessage(frame) => WsBridgeEvent::ServerMessage {
                    connection_id,
                    frame,
                },
                WsDriverEvent::ServerError(message) => WsBridgeEvent::ServerError {
                    connection_id,
                    message,
                },
                WsDriverEvent::ServerClose => WsBridgeEvent::ServerClose { connection_id },
            };

            match tsfn.call_async(bridge_event).await {
                Ok(promise) => promise.await.map_err(|e| {
                    ferrimock::FerrimockError::msg(format!("ws dispatch callback failed: {e}"))
                }),
                Err(e) => Err(ferrimock::FerrimockError::msg(format!(
                    "ws dispatch ThreadsafeFunction call failed: {e}"
                ))),
            }
        })
    })
}

/// Create a WebSocket handler mock.
///
/// @param url - Link URL pattern string (`wss://host/path/:param`) or RegExp
///   (tested against the path and `ws(s)://host/path` reconstructions).
/// @param dispatch - Callback receiving `{ type, connectionId, ... }`
///   connection events, resolving with whether `preventDefault()` was called.
#[napi(namespace = "ws")]
pub fn handler(
    env: &Env,
    url: Unknown,
    dispatch: Function<'_, WsBridgeEvent, Promise<bool>>,
) -> Result<RequestHandler> {
    use napi::JsValue;
    let raw = dispatch.value();
    // SAFETY: same napi_value; TSFN generics only affect call typing.
    #[allow(unsafe_code)]
    let erased: Function<'_, WsBridgeEvent, Promise<bool>> =
        unsafe { FromNapiValue::from_napi_value(raw.env, raw.value)? };
    let tsfn: WsDispatchTsfn = erased
        .build_threadsafe_function()
        .callee_handled::<false>()
        .weak::<true>()
        .max_queue_size::<0>()
        .build()?;
    let tsfn = Arc::new(tsfn);

    let (mut mock, upstream_url, pattern) = build_ws_mock(env, &url)?;

    let handler_upstream = upstream_url.clone();
    let handler_fn: WsHandlerFn = Arc::new(move |connection: WsConnection| {
        let tsfn = Arc::clone(&tsfn);
        let upstream_url = handler_upstream.clone();
        Box::pin(async move {
            let connection_id = format!(
                "ws:{:x}",
                CONNECTION_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let dispatch = build_dispatch(tsfn, connection_id, upstream_url.clone());
            drive_ws_connection(connection, upstream_url, dispatch).await
        })
    });

    mock.streaming = Some(ferrimock::types::StreamingResponse::WsHandler(handler_fn));

    Ok(RequestHandler {
        inner: Some(mock),
        fn_ref: None,
        pattern: Some(pattern),
    })
}

/// Build the WS mock skeleton: GET + upgrade header matcher + URL
/// predicate, with a static 426 body for non-upgrade hits.
fn build_ws_mock(env: &Env, url: &Unknown) -> Result<(MockDefinition, Option<String>, String)> {
    use smallvec::SmallVec;

    let placeholder: ferrimock::types::HandlerFn = Arc::new(|_ctx| {
        Box::pin(async {
            Err(ferrimock::FerrimockError::msg(
                "ws mock invoked as plain HTTP",
            ))
        })
    });

    let (mut mock, upstream_url, display) = if let Some((source, flags)) = as_regexp(env, url)? {
        let regex = compile_js_regex(&source, &flags)?;
        let mut mock = ferrimock::handler::http::get("*", placeholder);
        mock.request.url_patterns = SmallVec::from_elem(UrlPattern::HrefRegex(regex), 1);
        (mock, None, format!("/{source}/{flags}"))
    } else {
        // SAFETY: not a RegExp, so the value must be the URL string.
        #[allow(unsafe_code)]
        let url_str: String = unsafe { FromNapiValue::from_napi_value(env.raw(), url.raw())? };
        let absolute = url_str.starts_with("ws://") || url_str.starts_with("wss://");
        let pattern = url_str
            .replacen("ws://", "http://", 1)
            .replacen("wss://", "https://", 1);
        let mock = ferrimock::handler::http::get(&pattern, placeholder);
        let display = url_str.clone();
        (mock, absolute.then_some(url_str), display)
    };

    // WS mock hit by a non-upgrade request: the streaming serve path
    // answers 426; this static body covers the interceptor lane's
    // match_request, which never upgrades.
    mock.response = ResponseGenerator::new(
        http::StatusCode::UPGRADE_REQUIRED,
        BodySource::Inline(Arc::new(bytes::Bytes::from_static(
            b"WebSocket upgrade required",
        ))),
    );

    // Scope the mock to upgrade handshakes so plain GETs on the same
    // path fall through to other mocks.
    let upgrade = HeaderMatcher::regex(http::header::UPGRADE, "(?i)^websocket$")
        .map_err(|e| Error::from_reason(format!("upgrade matcher: {e}")))?;
    mock.request.header_matchers.push(upgrade);

    Ok((mock, upstream_url, display))
}
