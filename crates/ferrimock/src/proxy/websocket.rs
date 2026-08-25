//! Bridging a WebSocket through the proxy.
//!
//! This is the path a dev server's hot-module-reload channel takes, so it is
//! the one that decides whether the proxy is usable at all: a broken bridge
//! looks like "my edits stopped showing up" rather than like a proxy bug.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use http::{HeaderMap, Request, StatusCode, header};
use std::sync::Arc;
use tokio_tungstenite::tungstenite;

use super::headers;
use super::route;
use super::state::ProxyState;
use axum::extract::FromRequestParts;

/// Handshake headers the upstream connection generates for itself. Copying
/// the browser's would send a `Sec-WebSocket-Key` that the upstream's own
/// accept value no longer answers, and the handshake fails.
const HANDSHAKE_OWNED: [&str; 5] = [
    "sec-websocket-key",
    "sec-websocket-version",
    "sec-websocket-accept",
    "connection",
    "upgrade",
];

/// Proxy a WebSocket handshake and then relay frames both ways.
///
/// The upstream is connected *before* the client gets its 101, because the
/// subprotocol the client asked for is chosen by the upstream and has to be
/// echoed in the response. Answering first and connecting after would mean
/// guessing, and a client that gets a subprotocol nobody selected closes the
/// socket immediately.
pub async fn handle(state: &Arc<ProxyState>, request: Request<axum::body::Body>) -> Response {
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(str::to_string);
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let Some(route) = route::resolve(&state.config.routes, host.as_deref(), &path) else {
        return plain(
            StatusCode::NOT_FOUND,
            "no route forwards this WebSocket path",
        );
    };

    let target_url = build_ws_url(
        &route.target.ws_origin(),
        &route.target.base_path,
        &effective_path(route, &path),
        query.as_deref(),
    );

    let handshake = match build_handshake(&target_url, request.headers(), route.preserve_host) {
        Ok(handshake) => handshake,
        Err(message) => return plain(StatusCode::BAD_GATEWAY, &message),
    };

    // The browser's own handshake is extracted before the upstream is dialled,
    // so a request that turns out not to be upgradable fails here rather than
    // after a connection has been opened on its behalf.
    let (mut parts, body) = request.into_parts();
    let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(upgrade) => upgrade,
        Err(rejection) => return rejection.into_response(),
    };
    drop(body);

    let (upstream, upstream_response) = match tokio_tungstenite::connect_async(handshake).await {
        Ok(connected) => connected,
        Err(error) => {
            tracing::warn!("websocket upstream {target_url} refused the handshake: {error}");
            return plain(
                StatusCode::BAD_GATEWAY,
                &format!("websocket upstream refused the handshake: {error}"),
            );
        }
    };

    // The subprotocol is the upstream's choice, and a client handed one nobody
    // selected closes the socket at once -- which is why the upstream is
    // dialled before this response is built rather than after.
    let negotiated = upstream_response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let upgrade = match negotiated {
        Some(protocol) => upgrade.protocols([protocol]),
        None => upgrade,
    };

    upgrade.on_upgrade(move |client| async move {
        relay(client, upstream).await;
    })
}

/// Relay frames until either side goes away.
///
/// Both directions are driven by one task rather than two, so a closed socket
/// ends the whole bridge instead of leaving the other half parked on a
/// half-open connection until its keepalive notices.
async fn relay<U>(client: WebSocket, upstream: U)
where
    U: futures::Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
        + futures::Sink<tungstenite::Message, Error = tungstenite::Error>
        + Send
        + 'static,
{
    let (mut client_out, mut client_in) = client.split();
    let (mut upstream_out, mut upstream_in) = upstream.split();

    loop {
        tokio::select! {
            // Biased so a burst on one side cannot starve the other. Left to
            // the default random choice, a dev server's reload storm can hold
            // back the browser's ping and the connection times out mid-reload.
            biased;

            frame = client_in.next() => match frame {
                Some(Ok(message)) => {
                    let closing = matches!(message, Message::Close(_));
                    if upstream_out.send(to_tungstenite(message)).await.is_err() || closing {
                        break;
                    }
                }
                Some(Err(_)) | None => break,
            },

            frame = upstream_in.next() => match frame {
                Some(Ok(message)) => {
                    let closing = message.is_close();
                    if client_out.send(to_axum(message)).await.is_err() || closing {
                        break;
                    }
                }
                Some(Err(_)) | None => break,
            },
        }
    }

    // Both halves are told, so neither side is left waiting on a peer that is
    // never going to speak again.
    let _ = client_out.close().await;
    let _ = upstream_out.close().await;
}

/// axum and tungstenite model the same frames as two enums, so relaying one
/// means naming the pairs. Ping and pong carry payloads that have to survive:
/// several clients echo the body back and check it.
fn to_tungstenite(message: Message) -> tungstenite::Message {
    match message {
        Message::Text(text) => tungstenite::Message::Text(text.as_str().into()),
        Message::Binary(bytes) => tungstenite::Message::Binary(bytes),
        Message::Ping(bytes) => tungstenite::Message::Ping(bytes),
        Message::Pong(bytes) => tungstenite::Message::Pong(bytes),
        Message::Close(frame) => {
            tungstenite::Message::Close(frame.map(|frame| tungstenite::protocol::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.as_str().into(),
            }))
        }
    }
}

fn to_axum(message: tungstenite::Message) -> Message {
    match message {
        tungstenite::Message::Text(text) => Message::Text(text.as_str().into()),
        tungstenite::Message::Binary(bytes) => Message::Binary(bytes),
        tungstenite::Message::Ping(bytes) => Message::Ping(bytes),
        tungstenite::Message::Pong(bytes) => Message::Pong(bytes),
        tungstenite::Message::Close(frame) => {
            Message::Close(frame.map(|frame| axum::extract::ws::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.as_str().into(),
            }))
        }
        // A raw frame never reaches a read half; tungstenite only produces it
        // on the write side.
        tungstenite::Message::Frame(_) => Message::Binary(bytes::Bytes::new()),
    }
}

/// Build the upstream handshake, carrying the browser's application headers
/// but none of its connection-specific ones.
fn build_handshake(
    url: &str,
    incoming: &HeaderMap,
    preserve_host: bool,
) -> Result<Request<()>, String> {
    use tungstenite::client::IntoClientRequest;

    let mut handshake = url
        .into_client_request()
        .map_err(|e| format!("cannot address websocket upstream '{url}': {e}"))?;

    for (name, value) in incoming {
        let lowered = name.as_str();
        if HANDSHAKE_OWNED.contains(&lowered) {
            continue;
        }
        // `Host` is generated from the target URL unless the route says the
        // upstream should see the browser's.
        if lowered == "host" && !preserve_host {
            continue;
        }
        handshake.headers_mut().insert(name.clone(), value.clone());
    }

    Ok(handshake)
}

/// The path a route forwards, after any prefix strip.
fn effective_path(route: &super::config::RouteConfig, path: &str) -> String {
    if route.strip_prefix && route.prefix != "/" {
        let stripped = path.strip_prefix(route.prefix.as_str()).unwrap_or(path);
        if stripped.is_empty() {
            "/".to_string()
        } else {
            stripped.to_string()
        }
    } else {
        path.to_string()
    }
}

/// Join a `ws://` origin, a target base path, a request path and a query.
fn build_ws_url(origin: &str, base_path: &str, path: &str, query: Option<&str>) -> String {
    let mut url = String::with_capacity(origin.len() + base_path.len() + path.len() + 16);
    url.push_str(origin);
    url.push_str(base_path);
    url.push_str(path);
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    url
}

/// A plain-text failure, used before the connection is upgraded.
fn plain(status: StatusCode, message: &str) -> Response {
    (status, message.to_string()).into_response()
}

/// Whether the proxy should treat this request as a WebSocket handshake.
pub fn is_upgrade(request: &Request<axum::body::Body>) -> bool {
    headers::is_websocket_upgrade(request.method(), request.headers())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::proxy::config::{RouteConfig, Target};

    #[test]
    fn a_ws_url_carries_the_query_that_hmr_tokens_ride_on() {
        assert_eq!(
            build_ws_url("ws://localhost:5173", "", "/hmr", Some("token=abc")),
            "ws://localhost:5173/hmr?token=abc"
        );
        assert_eq!(
            build_ws_url("ws://localhost:5173", "", "/hmr", None),
            "ws://localhost:5173/hmr"
        );
    }

    #[test]
    fn a_ws_url_includes_the_targets_base_path() {
        assert_eq!(
            build_ws_url("wss://example.com", "/v2", "/socket", None),
            "wss://example.com/v2/socket"
        );
    }

    #[test]
    fn a_tls_target_produces_a_wss_origin() {
        let target = Target::parse("https://example.com").unwrap();
        assert_eq!(target.ws_origin(), "wss://example.com");
        let plain = Target::parse("http://localhost:5173").unwrap();
        assert_eq!(plain.ws_origin(), "ws://localhost:5173");
    }

    #[test]
    fn stripping_a_prefix_applies_to_websockets_too() {
        let route = RouteConfig::parse("/api=http://localhost:8080")
            .unwrap()
            .stripping_prefix();
        assert_eq!(effective_path(&route, "/api/socket"), "/socket");
        assert_eq!(effective_path(&route, "/api"), "/");
    }

    #[test]
    fn every_frame_kind_survives_the_round_trip_between_the_two_message_types() {
        let cases = [
            Message::Text("hello".into()),
            Message::Binary(bytes::Bytes::from_static(b"\x00\x01")),
            Message::Ping(bytes::Bytes::from_static(b"ping-payload")),
            Message::Pong(bytes::Bytes::from_static(b"pong-payload")),
        ];

        for message in cases {
            let round_tripped = to_axum(to_tungstenite(message.clone()));
            assert_eq!(round_tripped, message, "{message:?} did not survive");
        }
    }

    #[test]
    fn a_close_frame_keeps_its_code_and_reason() {
        let closed = Message::Close(Some(axum::extract::ws::CloseFrame {
            code: 1001,
            reason: "going away".into(),
        }));

        match to_axum(to_tungstenite(closed)) {
            Message::Close(Some(frame)) => {
                assert_eq!(frame.code, 1001);
                assert_eq!(frame.reason.as_str(), "going away");
            }
            other => panic!("expected a close frame, got {other:?}"),
        }
    }

    #[test]
    fn the_upstream_handshake_drops_the_browsers_key_and_keeps_its_cookies() {
        let mut incoming = HeaderMap::new();
        incoming.insert(
            "sec-websocket-key",
            http::HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
        );
        incoming.insert(header::COOKIE, http::HeaderValue::from_static("sid=42"));
        incoming.insert(
            header::HOST,
            http::HeaderValue::from_static("localhost:3010"),
        );

        let handshake = build_handshake("ws://localhost:5173/hmr", &incoming, false).unwrap();

        assert_eq!(handshake.headers()[header::COOKIE], "sid=42");
        assert_eq!(handshake.headers()[header::HOST], "localhost:5173");
        assert_ne!(
            handshake.headers()["sec-websocket-key"],
            "dGhlIHNhbXBsZSBub25jZQ=="
        );
    }

    #[test]
    fn preserve_host_reaches_the_upstream_handshake() {
        let mut incoming = HeaderMap::new();
        incoming.insert(header::HOST, http::HeaderValue::from_static("app.local"));

        let handshake = build_handshake("ws://localhost:5173/hmr", &incoming, true).unwrap();
        assert_eq!(handshake.headers()[header::HOST], "app.local");
    }
}
