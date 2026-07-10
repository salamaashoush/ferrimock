//! SSE mock drivers: declarative event playback and handler-driven
//! streams, both delivered as `text/event-stream` axum responses.

use std::convert::Infallible;
use std::sync::Arc;

use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::streaming::{ConnectionGuard, SseUpstreamEvent, StreamingConnections};
use crate::types::{
    MockDefinition, RequestContext, SseData, SseHandlerFn, SseRepeat, SseScript, SseSinkMsg,
};

fn build_ctx(
    mock: &MockDefinition,
    method: &http::Method,
    path: &str,
    query: Option<&str>,
    headers: &http::HeaderMap,
    captures: rustc_hash::FxHashMap<String, String>,
) -> RequestContext {
    let uri = match query {
        Some(q) => format!("{path}?{q}"),
        None => path.to_string(),
    };
    let mut ctx = RequestContext::from_request(method.as_str(), &uri, query, headers, None);
    ctx.captures = captures;
    ctx.vars.clone_from(&mock.vars);
    ctx
}

fn finalize(mock: &MockDefinition, mut response: Response) -> Response {
    for (key, value) in &mock.response.headers {
        if let (Ok(name), Ok(val)) = (
            http::header::HeaderName::try_from(key.as_str()),
            http::HeaderValue::from_str(value),
        ) {
            response.headers_mut().insert(name, val);
        }
    }
    if let Ok(value) = http::HeaderValue::from_str(mock.id.as_str()) {
        response.headers_mut().insert("x-mock-id", value);
    }
    response
}

fn render_event(script_event: &crate::types::SseEvent, ctx: &RequestContext) -> Event {
    let mut event = Event::default();
    if let Some(name) = &script_event.event {
        event = event.event(name);
    }
    if let Some(id) = &script_event.id {
        event = event.id(id);
    }
    if let Some(retry) = script_event.retry {
        event = event.retry(std::time::Duration::from_millis(u64::from(retry)));
    }
    let data = match &script_event.data {
        SseData::Static(data) => data.clone(),
        SseData::Template { source, hash } => crate::template::render_template_with_hash(
            source, *hash, ctx, None,
        )
        .unwrap_or_else(|e| {
            tracing::warn!("sse event template failed: {e}");
            String::new()
        }),
    };
    event.data(data)
}

struct Playback {
    script: Arc<SseScript>,
    ctx: RequestContext,
    index: usize,
    remaining: Option<u32>,
    sent_retry: bool,
    done: bool,
    /// Deregisters from the connection tracker when the stream drops.
    _guard: ConnectionGuard,
}

/// Declarative SSE playback response.
#[allow(clippy::implicit_hasher)]
pub fn declarative_response(
    mock: &MockDefinition,
    script: Arc<SseScript>,
    method: &http::Method,
    path: &str,
    query: Option<&str>,
    headers: &http::HeaderMap,
    captures: rustc_hash::FxHashMap<String, String>,
    tracker: &Arc<StreamingConnections>,
) -> Response {
    let ctx = build_ctx(mock, method, path, query, headers, captures);
    let close_after = script.close_after;
    let keep_alive = script.keep_alive;
    let (guard, kill) = tracker.register(mock.id.as_str());

    let playback = Playback {
        remaining: match script.repeat {
            SseRepeat::Count(n) => Some(n),
            SseRepeat::Forever => None,
        },
        script,
        ctx,
        index: 0,
        sent_retry: false,
        done: false,
        _guard: guard,
    };

    let stream = futures::stream::unfold(playback, move |mut pb| async move {
        if pb.done {
            if close_after {
                return None;
            }
            // Hold the connection open (keep-alive pings service it).
            return futures::future::pending().await;
        }

        if !pb.sent_retry {
            pb.sent_retry = true;
            if let Some(retry) = pb.script.retry {
                let event =
                    Event::default().retry(std::time::Duration::from_millis(u64::from(retry)));
                return Some((Ok::<Event, Infallible>(event), pb));
            }
        }

        loop {
            if pb.index >= pb.script.events.len() {
                match &mut pb.remaining {
                    Some(n) if *n <= 1 => {
                        pb.done = true;
                    }
                    Some(n) => {
                        *n -= 1;
                        pb.index = 0;
                    }
                    None => {
                        pb.index = 0;
                    }
                }
                if pb.done || pb.script.events.is_empty() {
                    if close_after {
                        return None;
                    }
                    futures::future::pending::<()>().await;
                }
                continue;
            }

            let Some(script_event) = pb.script.events.get(pb.index) else {
                continue;
            };
            if let Some(delay) = script_event.delay {
                tokio::time::sleep(delay).await;
            }
            let event = render_event(script_event, &pb.ctx);
            pb.index += 1;
            return Some((Ok(event), pb));
        }
    });

    // Mock removal (hot reload/reset) ends the stream mid-playback.
    let stream = stream.take_until(kill);
    let response = match keep_alive {
        Some(interval) => Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(interval))
            .into_response(),
        None => Sse::new(stream).into_response(),
    };
    finalize(mock, response)
}

/// Declarative SSE upstream passthrough: relay the real endpoint's
/// frames to the client until either side ends.
#[allow(clippy::implicit_hasher)]
pub async fn upstream_response(
    mock: &MockDefinition,
    script: Arc<SseScript>,
    tracker: &Arc<StreamingConnections>,
) -> Response {
    let Some(url) = script.upstream.clone() else {
        return (
            http::StatusCode::INTERNAL_SERVER_ERROR,
            "SSE upstream mock without upstream URL",
        )
            .into_response();
    };

    let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<SseUpstreamEvent>();
    tokio::spawn(crate::streaming::run_sse_upstream(url, None, evt_tx));

    // Hold the response until the upstream either opens (stream the
    // frames) or fails (502 with the reason).
    match evt_rx.recv().await {
        Some(SseUpstreamEvent::Open) => {}
        Some(SseUpstreamEvent::Error(message)) => {
            return (http::StatusCode::BAD_GATEWAY, message).into_response();
        }
        Some(SseUpstreamEvent::Frame(_) | SseUpstreamEvent::Reconnecting) | None => {
            return (
                http::StatusCode::BAD_GATEWAY,
                "SSE upstream ended before opening",
            )
                .into_response();
        }
    }

    let (guard, kill) = tracker.register(mock.id.as_str());
    let body_stream = futures::stream::unfold((evt_rx, guard), |(mut rx, guard)| async move {
        loop {
            match rx.recv().await {
                Some(SseUpstreamEvent::Frame(msg)) => {
                    return Some((
                        Ok::<bytes::Bytes, std::io::Error>(encode_frame(&msg)),
                        (rx, guard),
                    ));
                }
                Some(SseUpstreamEvent::Error(message)) => {
                    return Some((Err(std::io::Error::other(message)), (rx, guard)));
                }
                // The pump redials on its own; the relay just keeps the
                // client stream open across reconnects.
                Some(SseUpstreamEvent::Open | SseUpstreamEvent::Reconnecting) => {}
                None => return None,
            }
        }
    })
    .take_until(kill);

    let response = http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/event-stream")
        .header(http::header::CACHE_CONTROL, "no-cache")
        .header(http::header::CONNECTION, "keep-alive")
        .body(axum::body::Body::from_stream(body_stream))
        .unwrap_or_else(|_| {
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "Response build error",
            )
                .into_response()
        });
    finalize(mock, response)
}

/// Encode one SSE frame (MSW wire shape: no space after the colon,
/// `data:` per line, blank-line terminator).
fn encode_frame(msg: &crate::types::SseMessage) -> bytes::Bytes {
    let mut out = String::new();
    if let Some(id) = &msg.id {
        out.push_str("id:");
        out.push_str(id);
        out.push('\n');
    }
    if let Some(event) = &msg.event {
        out.push_str("event:");
        out.push_str(event);
        out.push('\n');
    }
    if let Some(retry) = msg.retry {
        out.push_str("retry:");
        out.push_str(&retry.to_string());
        out.push('\n');
    }
    let pure_retry =
        msg.retry.is_some() && msg.data.is_empty() && msg.id.is_none() && msg.event.is_none();
    if !pure_retry {
        for line in msg.data.split("\r\n").flat_map(|s| s.split(['\r', '\n'])) {
            out.push_str("data:");
            out.push_str(line);
            out.push('\n');
        }
    }
    out.push('\n');
    bytes::Bytes::from(out)
}

/// Handler-driven SSE response: spawn the handler with a sink whose
/// receiver feeds the body stream.
#[allow(clippy::implicit_hasher)]
pub fn handler_response(
    mock: &MockDefinition,
    handler: SseHandlerFn,
    method: &http::Method,
    path: &str,
    query: Option<&str>,
    headers: &http::HeaderMap,
    captures: rustc_hash::FxHashMap<String, String>,
    tracker: &Arc<StreamingConnections>,
) -> Response {
    let ctx = build_ctx(mock, method, path, query, headers, captures);
    let (tx, rx) = mpsc::unbounded_channel::<SseSinkMsg>();
    let (guard, kill) = tracker.register(mock.id.as_str());

    let kill_tx = tx.clone();
    let error_tx = tx.clone();
    let handler_task = tokio::spawn(async move {
        if let Err(e) = handler(ctx, tx).await {
            tracing::warn!("sse handler failed: {e}");
            // Abort the stream — a failed handler must not leave the
            // consumer hanging on an open connection.
            let _ = error_tx.send(SseSinkMsg::Error);
        }
    });
    // Fires on mock removal (Ok) or when the connection ends and the
    // guard drops the tracker entry (Err) — either way the handler must
    // not keep running against a dead or stale connection.
    tokio::spawn(async move {
        let _ = kill.await;
        let _ = kill_tx.send(SseSinkMsg::Close);
        handler_task.abort();
    });

    // Built on a raw streaming body (not Sse<S>) so SseSinkMsg::Error can
    // abort the connection mid-stream; frames are encoded manually with
    // MSW's wire shape (no space after the colon).
    let body_stream = futures::stream::unfold((rx, guard), |(mut rx, guard)| async move {
        match rx.recv().await {
            Some(SseSinkMsg::Message(msg)) => Some((
                Ok::<bytes::Bytes, std::io::Error>(encode_frame(&msg)),
                (rx, guard),
            )),
            Some(SseSinkMsg::Error) => Some((
                Err(std::io::Error::other("ferrimock sse handler error")),
                (rx, guard),
            )),
            // Clean end: explicit Close or the handler dropped its sender.
            Some(SseSinkMsg::Close) | None => None,
        }
    });

    let response = http::Response::builder()
        .status(http::StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/event-stream")
        .header(http::header::CACHE_CONTROL, "no-cache")
        .header(http::header::CONNECTION, "keep-alive")
        .body(axum::body::Body::from_stream(body_stream))
        .unwrap_or_else(|_| {
            (
                http::StatusCode::INTERNAL_SERVER_ERROR,
                "Response build error",
            )
                .into_response()
        });
    finalize(mock, response)
}
