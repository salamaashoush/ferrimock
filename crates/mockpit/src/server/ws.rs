//! WebSocket mock drivers: the upgrade response plus the per-connection
//! loops for declarative scripts and JS handlers.
//!
//! Message templates render with the triggering frame exposed as
//! `{{ body }}` / `{{ body_json }}` (the message is the body-analog of
//! an HTTP mock).

use std::sync::Arc;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::engine::matcher::MockMatch;
use crate::streaming::{KillSignal, StreamingConnections};
use crate::types::{
    RequestContext, StreamingResponse, WsAction, WsConnection, WsFrame, WsHandlerFn, WsInbound,
    WsMessageMatch, WsOutbound, WsPayload, WsScript,
};

/// Build the 101 response for a matched WebSocket mock.
pub fn upgrade_response(
    upgrade: WebSocketUpgrade,
    mock_match: MockMatch,
    method: &http::Method,
    uri: &http::Uri,
    headers: &http::HeaderMap,
    tracker: &Arc<StreamingConnections>,
) -> Response {
    let mock = Arc::clone(&mock_match.mock);
    let Some(streaming) = mock.streaming.clone() else {
        return (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "WebSocket mock without streaming plan",
        )
            .into_response();
    };

    let mut ctx = RequestContext::from_request(
        method.as_str(),
        &uri.to_string(),
        uri.query(),
        headers,
        None,
    );
    ctx.captures = mock_match.captures;
    ctx.vars.clone_from(&mock.vars);

    let upgrade = if let StreamingResponse::Ws(script) = &streaming {
        match &script.subprotocol {
            Some(proto) => upgrade.protocols([proto.clone()]),
            None => upgrade,
        }
    } else {
        // Handler lanes accept whatever the client offered (first one
        // wins) so strict clients see their subprotocol confirmed; the
        // handler reads the offered list from the connection info.
        let requested = crate::streaming::requested_protocols(&ctx);
        if requested.is_empty() {
            upgrade
        } else {
            upgrade.protocols(requested)
        }
    };

    let (guard, kill) = tracker.register(mock.id.as_str());
    let mock_id = mock.id.to_string();
    let mut response = upgrade.on_upgrade(move |socket| async move {
        let _guard = guard;
        run(socket, streaming, ctx, kill).await;
    });
    if let Ok(value) = http::HeaderValue::from_str(&mock_id) {
        response.headers_mut().insert("x-mock-id", value);
    }
    response
}

async fn run(
    socket: WebSocket,
    streaming: StreamingResponse,
    ctx: RequestContext,
    kill: KillSignal,
) {
    match streaming {
        StreamingResponse::Ws(script) => run_declarative(socket, &script, &ctx, kill).await,
        StreamingResponse::WsHandler(handler) => run_handler(socket, handler, ctx, kill).await,
        // handle_request only routes WS kinds here.
        StreamingResponse::Sse(_) | StreamingResponse::SseHandler(_) => {}
    }
}

fn frame_to_message(frame: WsFrame) -> Message {
    match frame {
        WsFrame::Text(text) => Message::Text(text.into()),
        WsFrame::Binary(bytes) => Message::Binary(bytes),
    }
}

fn message_to_frame(message: &Message) -> Option<WsFrame> {
    match message {
        Message::Text(text) => Some(WsFrame::Text(text.to_string())),
        Message::Binary(bytes) => Some(WsFrame::Binary(bytes.clone())),
        _ => None,
    }
}

fn close_message(code: Option<u16>, reason: Option<String>) -> Message {
    Message::Close(Some(CloseFrame {
        code: code.unwrap_or(1000),
        reason: reason.unwrap_or_default().into(),
    }))
}

/// Per-message render context: the triggering frame becomes the body.
fn message_ctx(base: &RequestContext, message: Option<&str>) -> RequestContext {
    let mut ctx = base.clone();
    if let Some(text) = message {
        ctx.body = Some(text.to_string());
        ctx.body_json = serde_json::from_str(text).ok();
    }
    ctx
}

fn render_payload(
    payload: &WsPayload,
    ctx: &RequestContext,
    message: Option<&str>,
) -> Option<WsFrame> {
    match payload {
        WsPayload::Text(text) => Some(WsFrame::Text(text.clone())),
        WsPayload::Binary(bytes) => Some(WsFrame::Binary(bytes.clone())),
        WsPayload::Template { source, hash } => {
            let render_ctx = message_ctx(ctx, message);
            match crate::template::render_template_with_hash(source, *hash, &render_ctx, None) {
                Ok(rendered) => Some(WsFrame::Text(rendered)),
                Err(e) => {
                    tracing::warn!("ws payload template failed: {e}");
                    None
                }
            }
        }
    }
}

fn rule_matches(matcher: &WsMessageMatch, frame: &WsFrame) -> bool {
    let text = match frame {
        WsFrame::Text(text) => Some(text.as_str()),
        WsFrame::Binary(_) => None,
    };
    match matcher {
        WsMessageMatch::Any => true,
        WsMessageMatch::Exact(expected) => text == Some(expected.as_str()),
        WsMessageMatch::Regex(re) => text.is_some_and(|t| re.is_match(t)),
        WsMessageMatch::JsonPath { path, equals } => text
            .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
            .and_then(|json| {
                crate::types::json_path_lookup(&json, path)
                    .map(|found| equals.as_ref().is_none_or(|want| found == want))
            })
            .unwrap_or(false),
        WsMessageMatch::Binary { bytes, prefix } => match frame {
            WsFrame::Binary(data) => {
                if *prefix {
                    data.starts_with(bytes)
                } else {
                    data == bytes
                }
            }
            WsFrame::Text(_) => false,
        },
    }
}

/// The upstream half of a passthrough connection.
pub(crate) struct Upstream {
    sink: futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
    stream: futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
}

impl Upstream {
    pub(crate) async fn connect(url: &str) -> Option<Self> {
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws, _resp)) => {
                let (sink, stream) = ws.split();
                Some(Self { sink, stream })
            }
            Err(e) => {
                tracing::warn!("ws upstream connect to {url} failed: {e}");
                None
            }
        }
    }

    pub(crate) async fn send(&mut self, frame: WsFrame) {
        use tokio_tungstenite::tungstenite::Message as TMessage;
        let msg = match frame {
            WsFrame::Text(text) => TMessage::Text(text.into()),
            WsFrame::Binary(bytes) => TMessage::Binary(bytes),
        };
        if let Err(e) = self.sink.send(msg).await {
            tracing::warn!("ws upstream send failed: {e}");
        }
    }

    /// Next upstream data frame (None = upstream gone).
    pub(crate) async fn recv(&mut self) -> Option<WsFrame> {
        use tokio_tungstenite::tungstenite::Message as TMessage;
        loop {
            match self.stream.next().await? {
                Ok(TMessage::Text(text)) => return Some(WsFrame::Text(text.to_string())),
                Ok(TMessage::Binary(bytes)) => return Some(WsFrame::Binary(bytes)),
                Ok(TMessage::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }
}

enum ActionOutcome {
    Continue,
    Close,
}

async fn run_action(
    action: &WsAction,
    socket: &mut WebSocket,
    upstream: &mut Option<Upstream>,
    ctx: &RequestContext,
    trigger: Option<&WsFrame>,
) -> ActionOutcome {
    let trigger_text = trigger.and_then(|f| match f {
        WsFrame::Text(text) => Some(text.as_str()),
        WsFrame::Binary(_) => None,
    });
    match action {
        WsAction::Send(payload) => {
            if let Some(frame) = render_payload(payload, ctx, trigger_text) {
                let _ = socket.send(frame_to_message(frame)).await;
            }
            ActionOutcome::Continue
        }
        WsAction::Echo => {
            if let Some(frame) = trigger {
                let _ = socket.send(frame_to_message(frame.clone())).await;
            }
            ActionOutcome::Continue
        }
        WsAction::Delay(duration) => {
            tokio::time::sleep(*duration).await;
            ActionOutcome::Continue
        }
        WsAction::Forward => {
            if let (Some(upstream), Some(frame)) = (upstream.as_mut(), trigger) {
                upstream.send(frame.clone()).await;
            }
            ActionOutcome::Continue
        }
        WsAction::Close { code, reason } => {
            let _ = socket.send(close_message(*code, reason.clone())).await;
            ActionOutcome::Close
        }
    }
}

pub async fn run_declarative(
    mut socket: WebSocket,
    script: &WsScript,
    ctx: &RequestContext,
    mut kill: KillSignal,
) {
    let mut upstream = match &script.upstream {
        Some(url) => Upstream::connect(url).await,
        None => None,
    };

    for action in &script.on_connect {
        if matches!(
            run_action(action, &mut socket, &mut upstream, ctx, None).await,
            ActionOutcome::Close
        ) {
            return;
        }
    }

    loop {
        let client_message = if let Some(up) = upstream.as_mut() {
            tokio::select! {
                msg = socket.recv() => Either::Client(msg),
                frame = up.recv() => Either::Upstream(frame),
                _ = &mut kill => Either::Killed,
            }
        } else {
            tokio::select! {
                msg = socket.recv() => Either::Client(msg),
                _ = &mut kill => Either::Killed,
            }
        };

        match client_message {
            // Mock removed (hot reload/reset): 1001 Going Away.
            Either::Killed => {
                let _ = socket.send(close_message(Some(1001), None)).await;
                if let Some(up) = upstream.as_mut() {
                    let _ = up.sink.close().await;
                }
                return;
            }
            Either::Upstream(Some(frame)) => {
                let _ = socket.send(frame_to_message(frame)).await;
            }
            Either::Upstream(None) => {
                // Upstream gone: keep serving rules without it.
                upstream = None;
            }
            Either::Client(message) => {
                let Some(Ok(message)) = message else { return };
                if matches!(message, Message::Close(_)) {
                    if let Some(up) = upstream.as_mut() {
                        let _ = up.sink.close().await;
                    }
                    return;
                }
                let Some(frame) = message_to_frame(&message) else {
                    continue;
                };

                let rule = script
                    .on_message
                    .iter()
                    .find(|rule| rule_matches(&rule.matcher, &frame));
                if let Some(rule) = rule {
                    for action in &rule.actions {
                        if matches!(
                            run_action(action, &mut socket, &mut upstream, ctx, Some(&frame)).await,
                            ActionOutcome::Close
                        ) {
                            return;
                        }
                    }
                } else if script.echo {
                    let _ = socket.send(frame_to_message(frame)).await;
                } else if let Some(up) = upstream.as_mut() {
                    up.send(frame).await;
                }
            }
        }
    }
}

enum Either<A, B> {
    Client(A),
    Upstream(B),
    Killed,
}

pub async fn run_handler(
    mut socket: WebSocket,
    handler: WsHandlerFn,
    ctx: RequestContext,
    mut kill: KillSignal,
) {
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<WsOutbound>();
    let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<WsInbound>();

    let connection = WsConnection {
        request: ctx,
        outbound: outbound_tx,
        inbound: inbound_rx,
    };
    let mut handler_task = tokio::spawn(handler(connection));

    loop {
        tokio::select! {
            // Mock removed (hot reload/reset): 1001 Going Away, stop the
            // handler instead of letting it run on the stale definition.
            _ = &mut kill => {
                let _ = socket.send(close_message(Some(1001), None)).await;
                handler_task.abort();
                break;
            },
            outbound = outbound_rx.recv() => match outbound {
                Some(WsOutbound::Frame(frame)) => {
                    let _ = socket.send(frame_to_message(frame)).await;
                }
                Some(WsOutbound::Close { code, reason }) => {
                    let _ = socket.send(close_message(code, reason)).await;
                    break;
                }
                // Handler dropped its sender: clean close.
                None => {
                    let _ = socket.send(close_message(Some(1000), None)).await;
                    break;
                }
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Close(frame))) => {
                    let _ = inbound_tx.send(WsInbound::Closed {
                        code: frame.as_ref().map(|f| f.code),
                        reason: frame.map(|f| f.reason.to_string()),
                    });
                    break;
                }
                Some(Ok(message)) => {
                    if let Some(frame) = message_to_frame(&message) {
                        let _ = inbound_tx.send(WsInbound::Frame(frame));
                    }
                }
                Some(Err(_)) | None => {
                    let _ = inbound_tx.send(WsInbound::Closed { code: None, reason: None });
                    break;
                }
            },
            result = &mut handler_task => {
                match result {
                    Ok(Ok(())) => {
                        // Handler finished; drain any frames it queued
                        // before its sender dropped.
                        while let Ok(outbound) = outbound_rx.try_recv() {
                            match outbound {
                                WsOutbound::Frame(frame) => {
                                    let _ = socket.send(frame_to_message(frame)).await;
                                }
                                WsOutbound::Close { code, reason } => {
                                    let _ = socket.send(close_message(code, reason)).await;
                                    return;
                                }
                            }
                        }
                        let _ = socket.send(close_message(Some(1000), None)).await;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("ws handler failed: {e}");
                        let _ = socket.send(close_message(Some(1011), None)).await;
                    }
                    Err(e) => {
                        tracing::warn!("ws handler panicked: {e}");
                        let _ = socket.send(close_message(Some(1011), None)).await;
                    }
                }
                break;
            },
        }
    }
}
