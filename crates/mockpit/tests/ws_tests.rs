#![cfg(feature = "server")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! End-to-end WebSocket mock tests: start a real mock server, connect
//! with tokio-tungstenite, and drive declarative `ws:` behaviors.

use futures::{SinkExt, StreamExt};
use mockpit::services::serve::{ServeInput, start};
use tokio_tungstenite::tungstenite::Message;

async fn serve_yaml(yaml: &str) -> (mockpit::services::serve::ServeHandle, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("mocks.yaml"), yaml).expect("write mocks");
    let handle = start(ServeInput {
        port: 0,
        mock_file: Some(dir.path().join("mocks.yaml").to_string_lossy().into_owned()),
        ..ServeInput::default()
    })
    .await
    .expect("server start");
    let ws_url = handle.url.replace("http://", "ws://");
    (handle, ws_url)
}

type WsClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(url: &str) -> WsClient {
    let (client, response) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connect");
    assert_eq!(response.status(), 101);
    client
}

async fn next_text(client: &mut WsClient) -> String {
    loop {
        match client.next().await.expect("frame").expect("frame ok") {
            Message::Text(text) => return text.to_string(),
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected text frame, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn on_connect_actions_and_message_rules() {
    let (_handle, ws_url) = serve_yaml(
        r#"
mocks:
  - id: chat
    match: { url: "/ws/chat" }
    ws:
      on_connect:
        - send: { type: welcome }
      on_message:
        - match: { exact: ping }
          actions: [ { send: pong } ]
        - match: { json_path: "$.type", equals: subscribe }
          actions: [ { send: '{"ok":true}' } ]
        - match: { regex: "^rex:" }
          actions: [ { send: matched-regex } ]
        - match: { exact: bye }
          actions: [ { close: { code: 4000, reason: done } } ]
"#,
    )
    .await;

    let mut client = connect(&format!("{ws_url}/ws/chat")).await;

    let welcome: serde_json::Value =
        serde_json::from_str(&next_text(&mut client).await).expect("welcome json");
    assert_eq!(welcome["type"], "welcome");

    client.send(Message::text("ping")).await.expect("send");
    assert_eq!(next_text(&mut client).await, "pong");

    client
        .send(Message::text(r#"{"type":"subscribe","chan":"a"}"#))
        .await
        .expect("send");
    assert_eq!(next_text(&mut client).await, r#"{"ok":true}"#);

    client.send(Message::text("rex:hello")).await.expect("send");
    assert_eq!(next_text(&mut client).await, "matched-regex");

    client.send(Message::text("bye")).await.expect("send");
    let close = loop {
        if let Message::Close(frame) = client.next().await.expect("frame").expect("frame ok") {
            break frame.expect("close frame");
        }
    };
    assert_eq!(u16::from(close.code), 4000);
    assert_eq!(close.reason.as_str(), "done");
}

#[tokio::test(flavor = "multi_thread")]
async fn echo_mode_and_binary_send() {
    let (_handle, ws_url) = serve_yaml(
        r"
mocks:
  - id: echoer
    match: { url: '/ws/echo' }
    ws:
      echo: true
      on_message:
        - match: { exact: gimme-binary }
          actions: [ { send_binary: 'AAECAwQ=' } ]
",
    )
    .await;

    let mut client = connect(&format!("{ws_url}/ws/echo")).await;

    client.send(Message::text("anything")).await.expect("send");
    assert_eq!(next_text(&mut client).await, "anything");

    client
        .send(Message::text("gimme-binary"))
        .await
        .expect("send");
    let binary = loop {
        match client.next().await.expect("frame").expect("frame ok") {
            Message::Binary(bytes) => break bytes,
            Message::Ping(_) | Message::Pong(_) => {}
            other => panic!("expected binary, got {other:?}"),
        }
    };
    assert_eq!(&binary[..], &[0, 1, 2, 3, 4]);
}

#[tokio::test(flavor = "multi_thread")]
async fn path_params_render_in_templates_and_delay_runs() {
    let (_handle, ws_url) = serve_yaml(
        r#"
mocks:
  - id: rooms
    match: { url: "/ws/room/:roomId" }
    ws:
      on_connect:
        - delay: 30ms
        - send_template: '{"room":"{{ captures.roomId }}"}'
      on_message:
        - match: { any: true }
          actions:
            - send_template: '{"echoed": {{ body_json.n }}, "room": "{{ captures.roomId }}"}'
"#,
    )
    .await;

    let started = std::time::Instant::now();
    let mut client = connect(&format!("{ws_url}/ws/room/42")).await;

    let hello: serde_json::Value =
        serde_json::from_str(&next_text(&mut client).await).expect("json");
    assert!(started.elapsed() >= std::time::Duration::from_millis(30));
    assert_eq!(hello["room"], "42");

    client
        .send(Message::text(r#"{"n": 7}"#))
        .await
        .expect("send");
    let echoed: serde_json::Value =
        serde_json::from_str(&next_text(&mut client).await).expect("json");
    assert_eq!(echoed["echoed"], 7);
    assert_eq!(echoed["room"], "42");
}

#[tokio::test(flavor = "multi_thread")]
async fn subprotocol_negotiated_on_handshake() {
    let (_handle, ws_url) = serve_yaml(
        r"
mocks:
  - id: proto
    match: { url: '/ws/proto' }
    ws:
      subprotocol: chat.v2
      on_connect: [ { send: hi } ]
",
    )
    .await;

    let request = tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
        format!("{ws_url}/ws/proto"),
    )
    .map(|mut req| {
        req.headers_mut()
            .insert("sec-websocket-protocol", "chat.v2".parse().expect("header"));
        req
    })
    .expect("request");

    let (mut client, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect");
    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok()),
        Some("chat.v2")
    );
    assert_eq!(next_text(&mut client).await, "hi");
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_get_falls_through_and_non_upgrade_hits_other_mocks() {
    let (handle, ws_url) = serve_yaml(
        r#"
mocks:
  - id: socket
    match: { url: "/api/live" }
    ws:
      on_connect: [ { send: ws-hello } ]
  - id: rest
    match: { url: "/api/live", methods: [GET] }
    response: { body: "plain http" }
"#,
    )
    .await;

    // Plain GET (no upgrade headers) must hit the HTTP mock — the ws
    // mock carries an upgrade header matcher.
    let response = http_get(&format!("{}/api/live", handle.url)).await;
    assert_eq!(response.0, 200);
    assert!(
        response.1.contains("plain http"),
        "expected http mock body, got: {}",
        response.1
    );

    // Upgrade handshake reaches the ws mock.
    let mut client = connect(&format!("{ws_url}/api/live")).await;
    assert_eq!(next_text(&mut client).await, "ws-hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_forwarding_passthrough() {
    // Real upstream: a second mockpit server in echo mode.
    let (upstream_handle, upstream_ws) = serve_yaml(
        r"
mocks:
  - id: real
    match: { url: '/real' }
    ws:
      echo: true
      on_connect: [ { send: from-upstream } ]
",
    )
    .await;
    let _keep = &upstream_handle;

    let (_handle, ws_url) = serve_yaml(&format!(
        r"
mocks:
  - id: proxy
    match: {{ url: '/ws/proxy' }}
    ws:
      upstream: {upstream_ws}/real
      on_message:
        - match: {{ exact: local }}
          actions: [ {{ send: handled-locally }} ]
        - match: {{ any: true }}
          actions: [ forward ]
",
    ))
    .await;

    let mut client = connect(&format!("{ws_url}/ws/proxy")).await;

    // Upstream's on_connect frame is relayed down to the client.
    assert_eq!(next_text(&mut client).await, "from-upstream");

    // Rule match: answered locally, never forwarded.
    client.send(Message::text("local")).await.expect("send");
    assert_eq!(next_text(&mut client).await, "handled-locally");

    // Forward action: upstream echoes it back through the proxy.
    client.send(Message::text("roundtrip")).await.expect("send");
    assert_eq!(next_text(&mut client).await, "roundtrip");
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_message_matchers() {
    // Payloads: [0,1,2,3] exact = AAECAw== ; prefix [255,254] = //4=
    let (_handle, ws_url) = serve_yaml(
        r"
mocks:
  - id: bin-rules
    match: { url: '/ws/bin-rules' }
    ws:
      on_message:
        - match: { binary_base64: 'AAECAw==' }
          actions: [ { send: exact-bytes } ]
        - match: { binary_prefix_base64: '//4=' }
          actions: [ { send: prefix-bytes } ]
        - match: { any: true }
          actions: [ { send: fallback } ]
",
    )
    .await;

    let mut client = connect(&format!("{ws_url}/ws/bin-rules")).await;

    client
        .send(Message::Binary(vec![0u8, 1, 2, 3].into()))
        .await
        .expect("send");
    assert_eq!(next_text(&mut client).await, "exact-bytes");

    client
        .send(Message::Binary(vec![255u8, 254, 9, 9, 9].into()))
        .await
        .expect("send");
    assert_eq!(next_text(&mut client).await, "prefix-bytes");

    // Text frames never hit binary matchers; a binary frame matching
    // neither rule falls to `any`.
    client
        .send(Message::Binary(vec![42u8].into()))
        .await
        .expect("send");
    assert_eq!(next_text(&mut client).await, "fallback");
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_mock_closes_live_connections() {
    let (handle, ws_url) = serve_yaml(
        r"
mocks:
  - id: doomed
    match: { url: '/ws/doomed' }
    ws:
      echo: true
      on_connect: [ { send: hi } ]
",
    )
    .await;

    let mut client = connect(&format!("{ws_url}/ws/doomed")).await;
    assert_eq!(next_text(&mut client).await, "hi");
    assert_eq!(handle.matcher.streaming_connections().len(), 1);

    // Reload paths funnel through remove_mock: live connections must
    // close (1001 Going Away) instead of running on the stale Arc.
    handle.registry.remove_mock("doomed");

    let close = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Message::Close(frame) = client.next().await.expect("frame").expect("frame ok") {
                break frame.expect("close frame");
            }
        }
    })
    .await
    .expect("connection must close after mock removal");
    assert_eq!(u16::from(close.code), 1001);
    assert!(handle.matcher.streaming_connections().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn href_regex_matches_reconstructed_ws_urls() {
    use mockpit::engine::{MockMatcher, MockRegistry};
    use mockpit::types::{HeaderMatcher, StreamingResponse, UrlPattern, WsScript};

    let mut mock = mockpit::handler::http::get("*", |_ctx| async {
        Err(mockpit::MockpitError::msg("ws stub"))
    });
    mock.request.url_patterns = smallvec::SmallVec::from_elem(
        UrlPattern::HrefRegex(regex::Regex::new(r"wss?://chat\.example\.com/live").expect("re")),
        1,
    );
    mock.request
        .header_matchers
        .push(HeaderMatcher::regex(http::header::UPGRADE, "(?i)^websocket$").expect("matcher"));
    mock.streaming = Some(StreamingResponse::Ws(std::sync::Arc::new(WsScript {
        subprotocol: None,
        echo: true,
        upstream: None,
        on_connect: vec![],
        on_message: vec![],
    })));
    let mock_id = mock.id.to_string();

    let registry = MockRegistry::new();
    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    let mut headers = http::HeaderMap::new();
    headers.insert("host", "chat.example.com".parse().expect("host"));
    headers.insert("upgrade", http::HeaderValue::from_static("websocket"));
    headers.insert("connection", http::HeaderValue::from_static("Upgrade"));

    let matches = matcher.find_ws_matches("/live/feed", None, &headers);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].mock.id.as_str(), mock_id);

    // Wrong host: the href reconstruction no longer matches.
    let mut other = http::HeaderMap::new();
    other.insert("host", "other.example.com".parse().expect("host"));
    other.insert("upgrade", http::HeaderValue::from_static("websocket"));
    other.insert("connection", http::HeaderValue::from_static("Upgrade"));
    assert!(
        matcher
            .find_ws_matches("/live/feed", None, &other)
            .is_empty()
    );
}

/// Minimal raw HTTP/1.1 GET (avoids pulling an HTTP client dev-dep).
async fn http_get(url: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let url = url::Url::parse(url).expect("url");
    let host = url.host_str().expect("host");
    let port = url.port().expect("port");
    let mut stream = tokio::net::TcpStream::connect((host, port))
        .await
        .expect("tcp connect");
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n",
        url.path()
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    let text = String::from_utf8_lossy(&raw);
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string();
    (status, body)
}
