//! Streaming mock kinds: WebSocket and Server-Sent Events.
//!
//! A [`MockDefinition`](super::MockDefinition) carrying a
//! [`StreamingResponse`] drives a long-lived connection instead of
//! producing a buffered [`DynamicResponse`](super::DynamicResponse) —
//! the serve layer branches on it before response generation.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use regex::Regex;
use serde_json::Value;
use tokio::sync::mpsc;

use super::RequestContext;

/// How a matched mock drives a long-lived connection.
#[derive(Clone)]
pub enum StreamingResponse {
    /// Declarative SSE playback.
    Sse(Arc<SseScript>),
    /// Handler-driven SSE (QuickJS `sse()` or programmatic).
    SseHandler(SseHandlerFn),
    /// Declarative WebSocket rules.
    Ws(Arc<WsScript>),
    /// Handler-driven WebSocket (QuickJS `ws.link()` or programmatic).
    WsHandler(WsHandlerFn),
}

impl fmt::Debug for StreamingResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sse(script) => f.debug_tuple("Sse").field(script).finish(),
            Self::SseHandler(_) => f.write_str("SseHandler(<handler>)"),
            Self::Ws(script) => f.debug_tuple("Ws").field(script).finish(),
            Self::WsHandler(_) => f.write_str("WsHandler(<handler>)"),
        }
    }
}

impl StreamingResponse {
    /// Mock kind label used by list/summary tooling.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sse(_) | Self::SseHandler(_) => "sse",
            Self::Ws(_) | Self::WsHandler(_) => "ws",
        }
    }

    pub fn is_ws(&self) -> bool {
        matches!(self, Self::Ws(_) | Self::WsHandler(_))
    }

    pub fn is_sse(&self) -> bool {
        matches!(self, Self::Sse(_) | Self::SseHandler(_))
    }
}

// ---------- SSE ----------

/// Declarative SSE playback: an ordered event list with per-event
/// delays, optional repetition, and keep-alive pings.
#[derive(Debug, Clone)]
pub struct SseScript {
    /// Initial `retry:` field (milliseconds), emitted before any event.
    pub retry: Option<u32>,
    /// Comment-ping interval keeping idle connections alive.
    pub keep_alive: Option<Duration>,
    /// How many times the event list plays.
    pub repeat: SseRepeat,
    /// Close the connection after playback finishes (`false` holds it open).
    pub close_after: bool,
    /// Real SSE endpoint to relay (exclusive with `events`): frames from
    /// the upstream stream forward to the client verbatim.
    pub upstream: Option<String>,
    pub events: Vec<SseEvent>,
}

#[derive(Debug, Clone, Copy)]
pub enum SseRepeat {
    Count(u32),
    Forever,
}

#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry: Option<u32>,
    /// Sleep before emitting this event.
    pub delay: Option<Duration>,
    pub data: SseData,
}

#[derive(Debug, Clone)]
pub enum SseData {
    /// Pre-serialized at load time (objects are compact JSON).
    Static(String),
    /// Tera template rendered per emission with the request context.
    Template { source: String, hash: u64 },
}

/// One outbound SSE message from a handler.
#[derive(Debug, Clone)]
pub struct SseMessage {
    pub id: Option<String>,
    pub event: Option<String>,
    pub data: String,
    pub retry: Option<u32>,
}

/// Messages a handler pushes into the SSE connection.
#[derive(Debug)]
pub enum SseSinkMsg {
    Message(SseMessage),
    /// End the stream cleanly.
    Close,
    /// Abort the body stream mid-flight (client sees a network error).
    Error,
}

/// Handler-driven SSE resolver. Returning does NOT close the stream —
/// the connection lives until `Close`/`Error` is sent or the sender is
/// dropped (script GC, engine teardown, client disconnect).
pub type SseHandlerFn = Arc<
    dyn Fn(
            RequestContext,
            mpsc::UnboundedSender<SseSinkMsg>,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::FerrimockError>> + Send>>
        + Send
        + Sync,
>;

// ---------- WebSocket ----------

/// Declarative WebSocket behavior: connect-time actions plus
/// first-match-wins message rules.
#[derive(Debug, Clone)]
pub struct WsScript {
    pub subprotocol: Option<String>,
    /// Echo unmatched messages back (fallback when no rule matches).
    pub echo: bool,
    /// Real upstream to dial for passthrough (`forward` actions and
    /// unmatched-message forwarding).
    pub upstream: Option<String>,
    pub on_connect: Vec<WsAction>,
    pub on_message: Vec<WsRule>,
}

#[derive(Debug, Clone)]
pub struct WsRule {
    pub matcher: WsMessageMatch,
    pub actions: Vec<WsAction>,
}

/// Message selectors. `Exact`/`Regex`/`JsonPath` apply to text frames
/// only; `Binary` is the byte-frame counterpart and `Any` matches both.
#[derive(Debug, Clone)]
pub enum WsMessageMatch {
    Any,
    Exact(String),
    Regex(Regex),
    /// JSONPath selector; `equals: None` means "path exists".
    JsonPath {
        path: String,
        equals: Option<Value>,
    },
    /// Binary frame match: whole-frame equality, or prefix match when
    /// `prefix` is set.
    Binary {
        bytes: Bytes,
        prefix: bool,
    },
}

#[derive(Debug, Clone)]
pub enum WsAction {
    Send(WsPayload),
    /// Re-send the triggering frame (no-op in `on_connect`).
    Echo,
    Delay(Duration),
    /// Relay the triggering frame to the upstream (requires `upstream`).
    Forward,
    Close {
        code: Option<u16>,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub enum WsPayload {
    Text(String),
    /// Tera template; context gets the request plus a `message` var.
    Template {
        source: String,
        hash: u64,
    },
    /// Decoded from base64 at load time.
    Binary(Bytes),
}

/// A WebSocket frame crossing the handler boundary.
#[derive(Debug, Clone)]
pub enum WsFrame {
    Text(String),
    Binary(Bytes),
}

/// Handler → socket.
#[derive(Debug)]
pub enum WsOutbound {
    Frame(WsFrame),
    Close {
        code: Option<u16>,
        reason: Option<String>,
    },
}

/// Socket → handler.
#[derive(Debug)]
pub enum WsInbound {
    Frame(WsFrame),
    Closed {
        code: Option<u16>,
        reason: Option<String>,
    },
}

/// Everything a WebSocket handler needs; the server socket pump owns
/// the other channel ends.
pub struct WsConnection {
    /// The handshake request; captures carry URL params.
    pub request: RequestContext,
    pub outbound: mpsc::UnboundedSender<WsOutbound>,
    pub inbound: mpsc::UnboundedReceiver<WsInbound>,
}

pub type WsHandlerFn = Arc<
    dyn Fn(WsConnection) -> Pin<Box<dyn Future<Output = Result<(), crate::FerrimockError>> + Send>>
        + Send
        + Sync,
>;
