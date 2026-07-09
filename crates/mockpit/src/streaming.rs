//! Streaming connection runtime shared by every handler lane.
//!
//! [`drive_ws_connection`](crate::streaming::drive_ws_connection) is the
//! per-connection host loop behind the QuickJS and NAPI WebSocket
//! handler bridges: it owns the upstream passthrough connection and the
//! MSW forwarding semantics (client frames flow upstream unless a
//! listener prevented them, upstream frames flow to the client unless
//! prevented), so a lane only supplies an event-dispatch callback. The
//! SSE half mirrors it with
//! [`run_sse_upstream`](crate::streaming::run_sse_upstream) and
//! [`SseFrameParser`](crate::streaming::SseFrameParser).
//! [`StreamingConnections`](crate::streaming::StreamingConnections)
//! tracks live connections per mock id so registry reload paths can
//! tear them down.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use dashmap::DashMap;
use lean_string::LeanString;
use tokio::sync::{mpsc, oneshot};

use crate::MockpitError;
use crate::types::{RequestContext, SseMessage, WsConnection, WsFrame, WsInbound, WsOutbound};

// ---------------------------------------------------------------------------
// Per-mock connection tracking
// ---------------------------------------------------------------------------

/// Fires when the connection's mock is removed (hot reload, handler
/// reset). Resolving with `Err` means the tracker entry vanished without
/// an explicit close — treat both as a teardown signal.
pub type KillSignal = oneshot::Receiver<()>;

/// Live streaming connections keyed by the mock that spawned them.
///
/// Drivers register on connect and hold the [`ConnectionGuard`] for the
/// connection's lifetime; [`MockRegistry::remove_mock`](crate::engine::MockRegistry::remove_mock)
/// closes every connection of the removed mock so reloaded definitions
/// never serve through stale handler Arcs.
#[derive(Default)]
pub struct StreamingConnections {
    conns: DashMap<u64, (LeanString, oneshot::Sender<()>)>,
    counter: AtomicU64,
}

impl StreamingConnections {
    /// Track a new connection; the guard deregisters on drop, the signal
    /// fires when the mock is torn down.
    pub fn register(self: &Arc<Self>, mock_id: &str) -> (ConnectionGuard, KillSignal) {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.conns.insert(id, (LeanString::from(mock_id), tx));
        (
            ConnectionGuard {
                tracker: Arc::clone(self),
                id,
            },
            rx,
        )
    }

    /// Close every live connection served by `mock_id`.
    pub fn close_mock(&self, mock_id: &str) {
        let ids: Vec<u64> = self
            .conns
            .iter()
            .filter(|entry| entry.value().0.as_str() == mock_id)
            .map(|entry| *entry.key())
            .collect();
        for id in ids {
            if let Some((_, (_, tx))) = self.conns.remove(&id) {
                let _ = tx.send(());
            }
        }
    }

    /// Close every live connection (registry clear).
    pub fn close_all(&self) {
        let ids: Vec<u64> = self.conns.iter().map(|entry| *entry.key()).collect();
        for id in ids {
            if let Some((_, (_, tx))) = self.conns.remove(&id) {
                let _ = tx.send(());
            }
        }
    }

    /// Number of live tracked connections.
    pub fn len(&self) -> usize {
        self.conns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conns.is_empty()
    }
}

/// Deregisters its connection from the tracker on drop.
pub struct ConnectionGuard {
    tracker: Arc<StreamingConnections>,
    id: u64,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.tracker.conns.remove(&self.id);
    }
}

// ---------------------------------------------------------------------------
// WebSocket connection driver
// ---------------------------------------------------------------------------

/// Commands a lane's `server` object sends to the connection's host
/// loop (which owns the upstream passthrough connection).
#[derive(Debug)]
pub enum WsUpstreamCmd {
    /// Dial the real server (the link's absolute URL).
    Connect,
    /// Relay a frame to the real server.
    Send(WsFrame),
    /// Close the upstream connection.
    Close,
}

/// Everything a lane needs to build its `client`/`server` objects for a
/// new connection, delivered in [`WsDriverEvent::Connection`].
pub struct WsConnectionSeed {
    /// The handshake request; captures carry URL params.
    pub request: RequestContext,
    /// Subprotocols the client offered in the handshake.
    pub protocols: Vec<String>,
    /// Frames/closes toward the intercepted client.
    pub outbound: mpsc::UnboundedSender<WsOutbound>,
    /// Commands toward the upstream passthrough connection.
    pub upstream: mpsc::UnboundedSender<WsUpstreamCmd>,
}

/// One event dispatched to a lane's listeners.
///
/// The dispatch return value reports whether a listener called
/// `preventDefault()` — it gates the driver's forwarding
/// (client→upstream for [`Message`](Self::Message), upstream→client for
/// [`ServerMessage`](Self::ServerMessage), and the client close on
/// [`ServerClose`](Self::ServerClose)).
pub enum WsDriverEvent {
    Connection(Box<WsConnectionSeed>),
    Message(WsFrame),
    Close {
        code: Option<u16>,
        reason: Option<String>,
    },
    ServerOpen,
    ServerMessage(WsFrame),
    ServerError(String),
    ServerClose,
}

/// Lane callback invoked once per connection event, returning whether a
/// listener prevented the default action.
pub type WsDispatchFn = Arc<
    dyn Fn(WsDriverEvent) -> Pin<Box<dyn Future<Output = Result<bool, MockpitError>> + Send>>
        + Send
        + Sync,
>;

/// Events flowing from the upstream passthrough connection into the
/// per-connection host loop.
enum UpstreamEvent {
    Open,
    Frame(WsFrame),
    Error(String),
    Closed,
}

/// Subprotocols offered in the handshake's `Sec-WebSocket-Protocol`.
pub fn requested_protocols(request: &RequestContext) -> Vec<String> {
    request
        .headers
        .get("sec-websocket-protocol")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(feature = "server")]
async fn dial_ws_upstream(
    url: String,
    evt_tx: mpsc::UnboundedSender<UpstreamEvent>,
) -> Option<mpsc::UnboundedSender<WsUpstreamCmd>> {
    let Some(mut upstream) = crate::server::ws::Upstream::connect(&url).await else {
        let _ = evt_tx.send(UpstreamEvent::Error(format!("connect to {url} failed")));
        return None;
    };
    let _ = evt_tx.send(UpstreamEvent::Open);

    // Dedicated pump: owns the upstream halves; commands arrive on the
    // returned sender, frames flow back through evt_tx.
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<WsUpstreamCmd>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                frame = upstream.recv() => {
                    if let Some(frame) = frame {
                        let _ = evt_tx.send(UpstreamEvent::Frame(frame));
                    } else {
                        let _ = evt_tx.send(UpstreamEvent::Closed);
                        break;
                    }
                },
                cmd = cmd_rx.recv() => match cmd {
                    Some(WsUpstreamCmd::Send(frame)) => upstream.send(frame).await,
                    Some(WsUpstreamCmd::Close) | None => break,
                    Some(WsUpstreamCmd::Connect) => {}
                },
            }
        }
    });
    Some(cmd_tx)
}

#[cfg(not(feature = "server"))]
async fn dial_ws_upstream(
    url: String,
    evt_tx: mpsc::UnboundedSender<UpstreamEvent>,
) -> Option<mpsc::UnboundedSender<WsUpstreamCmd>> {
    let _ = evt_tx.send(UpstreamEvent::Error(format!(
        "ws upstream passthrough to {url} requires the `server` feature"
    )));
    None
}

/// Per-connection host loop shared by every WebSocket handler lane.
///
/// Dispatches [`WsDriverEvent::Connection`] first (the lane registers
/// its listeners), then pumps client frames, upstream commands, and
/// upstream events until the client closes or the dispatch callback
/// fails. Events dispatch sequentially — each one is awaited before the
/// next is delivered, so listener ordering matches frame arrival.
pub async fn drive_ws_connection(
    connection: WsConnection,
    upstream_url: Option<String>,
    dispatch: WsDispatchFn,
) -> Result<(), MockpitError> {
    let WsConnection {
        request,
        outbound,
        mut inbound,
    } = connection;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<WsUpstreamCmd>();
    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<UpstreamEvent>();

    let protocols = requested_protocols(&request);
    let seed = WsConnectionSeed {
        request,
        protocols,
        outbound: outbound.clone(),
        upstream: cmd_tx.clone(),
    };
    dispatch(WsDriverEvent::Connection(Box::new(seed))).await?;

    let mut upstream_cmd: Option<mpsc::UnboundedSender<WsUpstreamCmd>> = None;

    loop {
        tokio::select! {
            message = inbound.recv() => match message {
                Some(WsInbound::Frame(frame)) => {
                    let prevented =
                        dispatch(WsDriverEvent::Message(frame.clone())).await?;
                    if !prevented && let Some(up) = &upstream_cmd {
                        let _ = up.send(WsUpstreamCmd::Send(frame));
                    }
                }
                Some(WsInbound::Closed { code, reason }) => {
                    let _ = dispatch(WsDriverEvent::Close { code, reason }).await;
                    return Ok(());
                }
                None => {
                    let _ = dispatch(WsDriverEvent::Close { code: None, reason: None }).await;
                    return Ok(());
                }
            },
            cmd = cmd_rx.recv() => match cmd {
                Some(WsUpstreamCmd::Connect) => {
                    if upstream_cmd.is_none() && let Some(url) = &upstream_url {
                        upstream_cmd = dial_ws_upstream(url.clone(), evt_tx.clone()).await;
                    }
                }
                Some(other) => {
                    if let Some(up) = &upstream_cmd {
                        let _ = up.send(other);
                    }
                }
                None => {}
            },
            event = evt_rx.recv() => match event {
                Some(UpstreamEvent::Open) => {
                    dispatch(WsDriverEvent::ServerOpen).await?;
                }
                Some(UpstreamEvent::Frame(frame)) => {
                    let prevented =
                        dispatch(WsDriverEvent::ServerMessage(frame.clone())).await?;
                    if !prevented {
                        let _ = outbound.send(WsOutbound::Frame(frame));
                    }
                }
                Some(UpstreamEvent::Error(message)) => {
                    tracing::warn!("ws upstream error: {message}");
                    dispatch(WsDriverEvent::ServerError(message)).await?;
                }
                Some(UpstreamEvent::Closed) => {
                    upstream_cmd = None;
                    // Forward the real server's close to the client
                    // unless a listener prevented it.
                    let prevented = dispatch(WsDriverEvent::ServerClose).await?;
                    if !prevented {
                        let _ = outbound.send(WsOutbound::Close { code: Some(1000), reason: None });
                        return Ok(());
                    }
                }
                None => {}
            },
        }
    }
}

// ---------------------------------------------------------------------------
// SSE upstream passthrough
// ---------------------------------------------------------------------------

/// Commands a lane's `server` object sends to the SSE host loop.
#[derive(Debug)]
pub enum SseUpstreamCmd {
    Connect,
    Close,
}

/// Events flowing from the upstream SSE connection into the host loop.
#[derive(Debug)]
pub enum SseUpstreamEvent {
    Open,
    Frame(SseMessage),
    /// The stream dropped after opening; the pump redials after the
    /// current retry delay (EventSource semantics). Lanes surface it as
    /// an `error` event; another [`Open`](Self::Open) follows when the
    /// redial succeeds.
    Reconnecting,
    Error(String),
}

/// Incremental SSE frame parser (handles `\n`, `\r\n`, and `\r` line
/// separators). Mirrors the wire shape the drivers emit: `data:` per
/// line, `event`/`id`/`retry` fields, comment lines skipped.
#[derive(Default)]
pub struct SseFrameParser {
    buffer: String,
}

impl SseFrameParser {
    pub fn push(&mut self, chunk: &str) -> Vec<SseMessage> {
        self.buffer.push_str(chunk);
        let normalized = self.buffer.replace("\r\n", "\n").replace('\r', "\n");
        let mut parts: Vec<&str> = normalized.split("\n\n").collect();
        let rest = parts.pop().unwrap_or_default().to_string();
        let frames = parts
            .iter()
            .filter_map(|block| parse_block(block))
            .collect();
        self.buffer = rest;
        frames
    }
}

fn parse_block(block: &str) -> Option<SseMessage> {
    let mut id: Option<String> = None;
    let mut event: Option<String> = None;
    let mut retry: Option<u32> = None;
    let mut data: Vec<&str> = Vec::new();
    let mut saw_field = false;

    for line in block.split('\n') {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "id" => {
                id = Some(value.to_string());
                saw_field = true;
            }
            "event" => {
                event = Some(value.to_string());
                saw_field = true;
            }
            "data" => {
                data.push(value);
                saw_field = true;
            }
            "retry" => {
                retry = value.parse().ok();
                saw_field = true;
            }
            _ => {}
        }
    }

    if !saw_field {
        return None;
    }
    if data.is_empty() && id.is_none() && retry.is_none() {
        return None;
    }
    Some(SseMessage {
        id,
        event,
        data: data.join("\n"),
        retry,
    })
}

#[cfg(feature = "server")]
fn sse_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

/// EventSource default reconnection delay, overridden by `retry:` frames.
#[cfg(feature = "server")]
const SSE_DEFAULT_RETRY_MS: u64 = 3000;

/// Wait out the reconnection delay. Returns false when the client sink
/// closed — the pump must stop instead of redialing.
#[cfg(feature = "server")]
async fn sse_reconnect_delay(
    evt_tx: &mpsc::UnboundedSender<SseUpstreamEvent>,
    retry_ms: u64,
) -> bool {
    if evt_tx.send(SseUpstreamEvent::Reconnecting).is_err() {
        return false;
    }
    tokio::select! {
        () = tokio::time::sleep(std::time::Duration::from_millis(retry_ms)) => true,
        () = evt_tx.closed() => false,
    }
}

/// Dial an upstream SSE endpoint and pump parsed frames into `evt_tx`.
///
/// Mirrors EventSource reconnect semantics (the Node lane's
/// `MockpitEventSource`): once a connection has opened, a dropped or
/// ended stream redials after the current `retry:` delay (default 3s)
/// with the last seen `id:` sent as `Last-Event-ID`. Terminal
/// conditions: an HTTP error status or a non-`text/event-stream`
/// content type on any attempt, or a dial failure before the first open
/// (so callers can answer 502). Stops as soon as `evt_tx`'s receiver
/// drops, including mid-wait.
#[cfg(feature = "server")]
pub async fn run_sse_upstream(
    url: String,
    last_event_id: Option<String>,
    evt_tx: mpsc::UnboundedSender<SseUpstreamEvent>,
) {
    let mut last_id = last_event_id;
    let mut retry_ms = SSE_DEFAULT_RETRY_MS;
    let mut opened = false;

    loop {
        let mut request = sse_client()
            .get(&url)
            .header(http::header::ACCEPT, "text/event-stream");
        if let Some(id) = &last_id {
            request = request.header("last-event-id", id.clone());
        }

        let mut response = match request.send().await {
            Ok(response) => response,
            Err(e) if !opened => {
                let _ = evt_tx.send(SseUpstreamEvent::Error(format!(
                    "sse upstream connect to {url} failed: {e}"
                )));
                return;
            }
            Err(e) => {
                tracing::debug!("sse upstream {url} redial failed: {e}");
                if !sse_reconnect_delay(&evt_tx, retry_ms).await {
                    return;
                }
                continue;
            }
        };

        let content_type = response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !response.status().is_success() || !content_type.contains("text/event-stream") {
            let _ = evt_tx.send(SseUpstreamEvent::Error(format!(
                "sse upstream {url} responded {} ({content_type})",
                response.status()
            )));
            return;
        }

        if evt_tx.send(SseUpstreamEvent::Open).is_err() {
            return;
        }
        opened = true;

        let mut parser = SseFrameParser::default();
        loop {
            let chunk = tokio::select! {
                chunk = response.chunk() => chunk,
                () = evt_tx.closed() => return,
            };
            match chunk {
                Ok(Some(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    for frame in parser.push(&text) {
                        if let Some(id) = &frame.id {
                            last_id = Some(id.clone());
                        }
                        if let Some(retry) = frame.retry {
                            retry_ms = u64::from(retry);
                        }
                        if evt_tx.send(SseUpstreamEvent::Frame(frame)).is_err() {
                            return;
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    tracing::debug!("sse upstream {url} read failed: {e}");
                    break;
                }
            }
        }

        if !sse_reconnect_delay(&evt_tx, retry_ms).await {
            return;
        }
    }
}

#[cfg(not(feature = "server"))]
pub async fn run_sse_upstream(
    url: String,
    _last_event_id: Option<String>,
    evt_tx: mpsc::UnboundedSender<SseUpstreamEvent>,
) {
    let _ = evt_tx.send(SseUpstreamEvent::Error(format!(
        "sse upstream passthrough to {url} requires the `server` feature"
    )));
}
