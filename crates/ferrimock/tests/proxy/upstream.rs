//! A real HTTP server for the proxy tests to forward to.
//!
//! Deliberately not a mock: the behaviours under test (chunked framing, an
//! event stream that arrives over time, a WebSocket that echoes, a body large
//! enough that buffering it would show up) are all things only a real server
//! on a real socket exhibits.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Frame, Incoming};
use hyper::{Request, Response, StatusCode};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

type ServerBody = BoxBody<Bytes, Infallible>;

/// A running test upstream.
pub struct Upstream {
    pub addr: SocketAddr,
    /// How many requests have reached it. The point of most mock assertions
    /// is that this does *not* move.
    pub hits: Arc<AtomicUsize>,
    /// Every request line the upstream saw, as `METHOD path`.
    pub seen: Arc<Mutex<Vec<String>>>,
    /// Headers of the most recent request.
    pub last_headers: Arc<Mutex<http::HeaderMap>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Upstream {
    /// The origin a route should point at.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// How many requests reached this upstream.
    pub fn hit_count(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// A header from the most recent request.
    pub fn last_header(&self, name: &str) -> Option<String> {
        self.last_headers
            .lock()
            .unwrap()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

/// Start the test upstream on a random port.
pub async fn start() -> Upstream {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let hits = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let last_headers = Arc::new(Mutex::new(http::HeaderMap::new()));
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let state = (
        Arc::clone(&hits),
        Arc::clone(&seen),
        Arc::clone(&last_headers),
    );

    tokio::spawn(async move {
        loop {
            let stream = tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => stream,
                    Err(_) => continue,
                },
                _ = &mut shutdown_rx => break,
            };

            let (hits, seen, last_headers) = state.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |request| {
                    let hits = Arc::clone(&hits);
                    let seen = Arc::clone(&seen);
                    let last_headers = Arc::clone(&last_headers);
                    async move { route(request, hits, seen, last_headers).await }
                });

                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection_with_upgrades(hyper_util::rt::TokioIo::new(stream), service)
                .await;
            });
        }
    });

    Upstream {
        addr,
        hits,
        seen,
        last_headers,
        shutdown: Some(shutdown_tx),
    }
}

async fn route(
    request: Request<Incoming>,
    hits: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
    last_headers: Arc<Mutex<http::HeaderMap>>,
) -> Result<Response<ServerBody>, Infallible> {
    hits.fetch_add(1, Ordering::SeqCst);
    seen.lock()
        .unwrap()
        .push(format!("{} {}", request.method(), request.uri().path()));
    *last_headers.lock().unwrap() = request.headers().clone();

    let path = request.uri().path().to_string();

    Ok(match path.as_str() {
        "/sse" => event_stream(),
        "/ws" => return Ok(websocket_echo(request)),
        "/big" => big_body(),
        "/slow-headers" => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            json(StatusCode::OK, &serde_json::json!({"late": true}))
        }
        "/gzip" => gzip_body(),
        _ => echo(request).await,
    })
}

/// Reflect the request back, which is how a test asserts on what the proxy
/// actually sent rather than on what it meant to send.
async fn echo(request: Request<Incoming>) -> Response<ServerBody> {
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(str::to_string);
    let version = format!("{:?}", request.version());
    let headers: std::collections::BTreeMap<String, String> = request
        .headers()
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect();

    let body = request
        .into_body()
        .collect()
        .await
        .map(http_body_util::Collected::to_bytes)
        .unwrap_or_default();

    json(
        StatusCode::OK,
        &serde_json::json!({
            "method": method,
            "path": path,
            "query": query,
            "version": version,
            "headers": headers,
            "body_len": body.len(),
            "body": String::from_utf8_lossy(&body),
        }),
    )
}

/// Three events spread over time, so a test can tell streaming from
/// collecting: a proxy that buffers delivers all three at once, at the end.
fn event_stream() -> Response<ServerBody> {
    let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, Infallible>>(4);

    tokio::spawn(async move {
        for index in 0..3 {
            if tx
                .send(Ok(Frame::data(Bytes::from(format!(
                    "data: event-{index}\n\n"
                )))))
                .await
                .is_err()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    });

    // `futures` rather than `tokio-stream`: one fewer dev-dependency, and the
    // unfold is the whole adapter.
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|frame| (frame, rx))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "text/event-stream")
        .header(http::header::CACHE_CONTROL, "no-cache")
        .body(BoxBody::new(StreamBody::new(stream)))
        .unwrap()
}

/// Eight megabytes, which is past every buffering cap the tests set.
fn big_body() -> Response<ServerBody> {
    let payload = Bytes::from(vec![b'x'; 8 * 1024 * 1024]);
    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/octet-stream")
        .body(BoxBody::new(Full::new(payload)))
        .unwrap()
}

/// A body that claims an encoding, to prove the proxy forwards it untouched
/// when it is not reading it.
fn gzip_body() -> Response<ServerBody> {
    Response::builder()
        .status(StatusCode::OK)
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::CONTENT_ENCODING, "gzip")
        .body(BoxBody::new(Full::new(Bytes::from_static(
            b"not-really-gzip",
        ))))
        .unwrap()
}

/// Accept the handshake and echo every frame back with a prefix, so a test can
/// tell a relayed frame from one the proxy invented.
fn websocket_echo(mut request: Request<Incoming>) -> Response<ServerBody> {
    use tokio_tungstenite::tungstenite;

    let key = request
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let Some(key) = key else {
        return json(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({"error": "no key"}),
        );
    };

    tokio::spawn(async move {
        let Ok(upgraded) = hyper::upgrade::on(&mut request).await else {
            return;
        };
        let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
            hyper_util::rt::TokioIo::new(upgraded),
            tungstenite::protocol::Role::Server,
            None,
        )
        .await;

        while let Some(Ok(message)) = socket.next().await {
            match message {
                tungstenite::Message::Text(text) => {
                    if socket
                        .send(tungstenite::Message::text(format!("echo:{text}")))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                tungstenite::Message::Close(_) => return,
                _ => {}
            }
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(http::header::CONNECTION, "Upgrade")
        .header(http::header::UPGRADE, "websocket")
        .header(
            "sec-websocket-accept",
            tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes()),
        )
        .body(BoxBody::new(Full::new(Bytes::new())))
        .unwrap()
}

fn json(status: StatusCode, value: &serde_json::Value) -> Response<ServerBody> {
    Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(BoxBody::new(Full::new(Bytes::from(value.to_string()))))
        .unwrap()
}
