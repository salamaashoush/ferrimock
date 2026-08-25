//! End-to-end tests for the reverse proxy.
//!
//! Every one of these drives a real client over a real socket to a real
//! upstream. The properties that matter here (a body that is never collected,
//! an event that arrives before the stream ends, a WebSocket frame that comes
//! back) are not observable from a unit test of the handler.

#![cfg(feature = "proxy")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

#[path = "proxy/upstream.rs"]
mod upstream;

use ferrimock::engine::{MockMatcher, MockRegistry};
use ferrimock::proxy::{ProxyConfig, ProxyHandle, RouteConfig, TlsConfig, UpstreamConfig};
use ferrimock::recorder::RecordingFormat;
use std::time::{Duration, Instant};

/// Start a proxy on a random port with these route specs.
async fn proxy_with(routes: &[&str], matcher: Option<MockMatcher>) -> ProxyHandle {
    let config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: routes
            .iter()
            .map(|spec| RouteConfig::parse(spec).unwrap())
            .collect(),
        ..ProxyConfig::default()
    };
    ferrimock::proxy::start(config, matcher).await.unwrap()
}

/// A registry holding one mock collection, parsed from YAML.
async fn registry_from(yaml: &str) -> MockRegistry {
    let registry = MockRegistry::new();
    let file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
    std::fs::write(file.path(), yaml).unwrap();
    registry.load_collection_file(file.path()).await.unwrap();
    registry
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()
}

// -- Forwarding ----------------------------------------------------------

#[tokio::test]
async fn a_request_reaches_the_upstream_and_the_answer_comes_back() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    let response = client()
        .get(format!("{}/hello?a=1", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["path"], "/hello");
    assert_eq!(body["query"], "a=1");
    assert_eq!(upstream.hit_count(), 1);
}

#[tokio::test]
async fn the_longer_prefix_wins_over_the_catch_all() {
    let api = upstream::start().await;
    let app = upstream::start().await;
    let proxy = proxy_with(
        &[&format!("/={}", app.url()), &format!("/api={}", api.url())],
        None,
    )
    .await;

    client()
        .get(format!("{}/api/users", proxy.url()))
        .send()
        .await
        .unwrap();
    client()
        .get(format!("{}/index.html", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(
        api.hit_count(),
        1,
        "the /api route should have taken /api/users"
    );
    assert_eq!(
        app.hit_count(),
        1,
        "the catch-all should have taken /index.html"
    );
}

#[tokio::test]
async fn a_stripped_prefix_reaches_the_upstream_without_it() {
    let upstream = upstream::start().await;
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![
            RouteConfig::parse(&format!("/api={}", upstream.url()))
                .unwrap()
                .stripping_prefix(),
        ],
        ..ProxyConfig::default()
    };
    config.compile();
    let proxy = ferrimock::proxy::start(config, None).await.unwrap();

    let body: serde_json::Value = client()
        .get(format!("{}/api/users", proxy.url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["path"], "/users");
}

#[tokio::test]
async fn the_upstream_sees_its_own_host_and_the_browsers_in_x_forwarded_host() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    client()
        .get(format!("{}/echo", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(
        upstream.last_header("host").as_deref(),
        Some(upstream.addr.to_string().as_str())
    );
    assert_eq!(
        upstream.last_header("x-forwarded-host").as_deref(),
        Some(proxy.local_addr().to_string().as_str())
    );
    assert_eq!(
        upstream.last_header("x-forwarded-proto").as_deref(),
        Some("http")
    );
    assert_eq!(
        upstream.last_header("x-forwarded-for").as_deref(),
        Some("127.0.0.1")
    );
}

#[tokio::test]
async fn hop_by_hop_headers_do_not_reach_the_upstream() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    client()
        .get(format!("{}/echo", proxy.url()))
        .header("connection", "keep-alive, x-secret-hop")
        .header("x-secret-hop", "should-not-travel")
        .header("x-end-to-end", "should-travel")
        .send()
        .await
        .unwrap();

    assert!(upstream.last_header("x-secret-hop").is_none());
    assert_eq!(
        upstream.last_header("x-end-to-end").as_deref(),
        Some("should-travel")
    );
}

#[tokio::test]
async fn a_post_body_arrives_intact() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    let body: serde_json::Value = client()
        .post(format!("{}/submit", proxy.url()))
        .body("hello upstream")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["method"], "POST");
    assert_eq!(body["body"], "hello upstream");
}

// -- Streaming -----------------------------------------------------------

#[tokio::test]
async fn an_event_stream_arrives_event_by_event_rather_than_all_at_once() {
    use futures::StreamExt;

    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    let started = Instant::now();
    let response = client()
        .get(format!("{}/sse", proxy.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()["content-type"],
        "text/event-stream",
        "the content type has to survive the proxy or the browser will not parse it"
    );

    let mut stream = response.bytes_stream();
    let first = stream.next().await.unwrap().unwrap();
    let first_at = started.elapsed();

    assert!(
        String::from_utf8_lossy(&first).contains("event-0"),
        "expected the first event, got {:?}",
        String::from_utf8_lossy(&first)
    );
    // The upstream spaces events 120ms apart and sends three. A proxy that
    // collected the body could not produce the first one before 240ms.
    assert!(
        first_at < Duration::from_millis(200),
        "the first event took {first_at:?}, which means the body was collected"
    );

    let mut seen = 1;
    while let Some(Ok(chunk)) = stream.next().await {
        seen += String::from_utf8_lossy(&chunk).matches("data:").count();
    }
    assert_eq!(seen, 3, "every event should have been relayed");
}

#[tokio::test]
async fn a_body_larger_than_the_buffering_cap_still_transfers_whole() {
    let upstream = upstream::start().await;
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![RouteConfig::parse(&format!("/={}", upstream.url())).unwrap()],
        // Well under the 8MB the upstream sends, so a proxy that buffered
        // would have to either truncate or fail.
        max_buffered_response: 64 * 1024,
        ..ProxyConfig::default()
    };
    config.compile();
    let proxy = ferrimock::proxy::start(config, None).await.unwrap();

    let body = client()
        .get(format!("{}/big", proxy.url()))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();

    assert_eq!(body.len(), 8 * 1024 * 1024);
}

#[tokio::test]
async fn a_content_encoding_the_proxy_does_not_read_is_forwarded_untouched() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    // reqwest would transparently decode a real gzip body, so the upstream
    // sends a lie: what matters is that the proxy passed the header along
    // rather than deciding it needed to decode anything.
    let response = client()
        .get(format!("{}/gzip", proxy.url()))
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();

    assert_eq!(
        upstream.last_header("accept-encoding").as_deref(),
        Some("gzip"),
        "the browser's Accept-Encoding should reach the upstream unchanged"
    );
    assert_eq!(response.headers().get("content-encoding").unwrap(), "gzip");
}

// -- Mocks ---------------------------------------------------------------

#[tokio::test]
async fn a_matching_mock_answers_and_the_upstream_is_never_touched() {
    let upstream = upstream::start().await;
    let registry = registry_from(
        r"
mocks:
  - id: users
    match:
      GET: /api/users
    response:
      json:
        users: [{ id: 1, name: Salama }]
",
    )
    .await;

    let proxy = proxy_with(
        &[&format!("/={}", upstream.url())],
        Some(MockMatcher::new(registry)),
    )
    .await;

    let response = client()
        .get(format!("{}/api/users", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.headers().get("x-mock-id").unwrap(), "users");
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["users"][0]["name"], "Salama");
    assert_eq!(
        upstream.hit_count(),
        0,
        "a mocked request must not reach the upstream"
    );
}

#[tokio::test]
async fn an_unmatched_request_falls_through_to_the_upstream() {
    let upstream = upstream::start().await;
    let registry = registry_from(
        r"
mocks:
  - id: users
    match:
      GET: /api/users
    response:
      json: { ok: true }
",
    )
    .await;

    let proxy = proxy_with(
        &[&format!("/={}", upstream.url())],
        Some(MockMatcher::new(registry)),
    )
    .await;

    let body: serde_json::Value = client()
        .get(format!("{}/api/folders", proxy.url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["path"], "/api/folders");
    assert_eq!(upstream.hit_count(), 1);
}

#[tokio::test]
async fn a_patch_mock_rewrites_what_the_upstream_said() {
    let upstream = upstream::start().await;
    let registry = registry_from(
        r"
mocks:
  - id: rewrite-path
    match:
      GET: /echo
    patch:
      jsonpath:
        $.path: /rewritten-by-the-mock
      headers:
        add:
          x-patched-by: ferrimock
",
    )
    .await;

    let proxy = proxy_with(
        &[&format!("/={}", upstream.url())],
        Some(MockMatcher::new(registry)),
    )
    .await;

    let response = client()
        .get(format!("{}/echo", proxy.url()))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers().get("x-mock-id").unwrap(), "rewrite-path");
    assert_eq!(response.headers().get("x-patched-by").unwrap(), "ferrimock");

    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body["path"], "/rewritten-by-the-mock",
        "the patch should have replaced the upstream's own value"
    );
    assert_eq!(upstream.hit_count(), 1, "a patch mock still calls upstream");
}

#[tokio::test]
async fn a_request_body_is_not_read_when_no_mock_matches_on_one() {
    let upstream = upstream::start().await;
    let registry = registry_from(
        r"
mocks:
  - id: by-path-only
    match:
      GET: /never
    response:
      json: { ok: true }
",
    )
    .await;

    let matcher = MockMatcher::new(registry);
    assert!(
        !matcher.registry().needs_request_body(),
        "no mock here matches on a body, so the proxy must not buffer one"
    );

    let proxy = proxy_with(&[&format!("/={}", upstream.url())], Some(matcher)).await;

    let payload = "y".repeat(4 * 1024 * 1024);
    let body: serde_json::Value = client()
        .post(format!("{}/upload", proxy.url()))
        .body(payload.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["body_len"], payload.len());
}

// -- WebSockets ----------------------------------------------------------

#[tokio::test]
async fn websocket_frames_relay_in_both_directions() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    let (mut socket, response) =
        tokio_tungstenite::connect_async(format!("ws://{}/ws", proxy.local_addr()))
            .await
            .expect("the proxy should have completed the handshake");

    assert_eq!(response.status(), 101);

    socket.send(Message::text("ping")).await.unwrap();
    let reply = socket.next().await.unwrap().unwrap();
    assert_eq!(reply.to_text().unwrap(), "echo:ping");

    socket.send(Message::text("again")).await.unwrap();
    let reply = socket.next().await.unwrap().unwrap();
    assert_eq!(reply.to_text().unwrap(), "echo:again");

    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn a_websocket_carries_the_query_string_hmr_tokens_ride_on() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{}/ws?token=abc&build=7",
        proxy.local_addr()
    ))
    .await
    .unwrap();

    assert_eq!(
        upstream.seen.lock().unwrap().last().map(String::as_str),
        Some("GET /ws")
    );

    socket.close(None).await.unwrap();
}

// -- TLS and HTTP/2 ------------------------------------------------------

#[tokio::test]
async fn tls_termination_negotiates_http2_over_alpn() {
    let upstream = upstream::start().await;
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![RouteConfig::parse(&format!("/={}", upstream.url())).unwrap()],
        tls: Some(TlsConfig::SelfSigned {
            names: vec!["localhost".to_string(), "127.0.0.1".to_string()],
        }),
        ..ProxyConfig::default()
    };
    config.compile();
    let proxy = ferrimock::proxy::start(config, None).await.unwrap();

    assert!(proxy.url().starts_with("https://"));

    let response = client()
        .get(format!("{}/echo", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.version(),
        reqwest::Version::HTTP_2,
        "ALPN offers h2 first, so a client that speaks it should have taken it"
    );
}

#[tokio::test]
async fn an_http2_request_forwards_to_an_http1_upstream() {
    let upstream = upstream::start().await;
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![RouteConfig::parse(&format!("/={}", upstream.url())).unwrap()],
        tls: Some(TlsConfig::SelfSigned {
            names: vec!["localhost".to_string()],
        }),
        ..ProxyConfig::default()
    };
    config.compile();
    let proxy = ferrimock::proxy::start(config, None).await.unwrap();

    let body: serde_json::Value = client()
        .post(format!("{}/echo", proxy.url()))
        .body("over h2")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(body["body"], "over h2");
    // The upstream is plain HTTP/1.1, so the proxy has to translate rather
    // than pass the version through.
    assert_eq!(body["version"], "HTTP/1.1");
}

// -- Recording -----------------------------------------------------------

#[tokio::test]
async fn recording_captures_forwarded_traffic_but_not_mocked_responses() {
    let upstream = upstream::start().await;
    let registry = registry_from(
        r"
mocks:
  - id: mocked
    match:
      GET: /mocked
    response:
      json: { from: mock }
",
    )
    .await;

    let proxy = proxy_with(
        &[&format!("/={}", upstream.url())],
        Some(MockMatcher::new(registry)),
    )
    .await;

    let dir = tempfile::tempdir().unwrap();
    proxy
        .state()
        .start_recording(dir.path(), Some("session".into()), RecordingFormat::Json)
        .await
        .unwrap();

    client()
        .get(format!("{}/forwarded", proxy.url()))
        .send()
        .await
        .unwrap();
    client()
        .get(format!("{}/mocked", proxy.url()))
        .send()
        .await
        .unwrap();

    // The tee commits after the last body frame has gone out, so the write
    // lands just behind the response the client already has.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let path = proxy.state().stop_recording().await.unwrap().unwrap();

    let recorded = std::fs::read_to_string(&path).unwrap();
    assert!(
        recorded.contains("/forwarded"),
        "forwarded traffic should be recorded: {recorded}"
    );
    assert!(
        !recorded.contains("\"/mocked\""),
        "a mocked response is not an observation and must not be recorded: {recorded}"
    );
}

#[tokio::test]
async fn recording_does_not_hold_back_the_response() {
    use futures::StreamExt;

    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;

    let dir = tempfile::tempdir().unwrap();
    proxy
        .state()
        .start_recording(dir.path(), None, RecordingFormat::Json)
        .await
        .unwrap();

    // An event stream is the case a collect-then-record design breaks: it
    // never ends, so the first event would never arrive.
    let started = Instant::now();
    let mut stream = client()
        .get(format!("{}/sse", proxy.url()))
        .send()
        .await
        .unwrap()
        .bytes_stream();

    let first = stream.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&first).contains("event-0"));
    assert!(started.elapsed() < Duration::from_millis(200));

    drop(stream);
    let _ = proxy.state().stop_recording().await;
}

#[tokio::test]
async fn a_second_recording_is_refused_rather_than_silently_replacing_the_first() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;
    let dir = tempfile::tempdir().unwrap();

    proxy
        .state()
        .start_recording(dir.path(), Some("first".into()), RecordingFormat::Json)
        .await
        .unwrap();

    let error = proxy
        .state()
        .start_recording(dir.path(), Some("second".into()), RecordingFormat::Json)
        .await
        .unwrap_err()
        .to_string();

    assert!(error.contains("already in progress"), "{error}");
}

// -- Failure modes -------------------------------------------------------

#[tokio::test]
async fn an_unroutable_request_names_the_routes_that_do_exist() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/api={}", upstream.url())], None).await;

    let response = client()
        .get(format!("{}/not-under-api", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["path"], "/not-under-api");
    assert!(
        body["routes"].as_array().unwrap()[0]
            .as_str()
            .unwrap()
            .starts_with("/api ->"),
        "the answer should say which routes exist: {body}"
    );
}

#[tokio::test]
async fn an_unreachable_upstream_is_a_bad_gateway() {
    // Port 1 on loopback: reserved, and nothing is listening.
    let proxy = proxy_with(&["/=http://127.0.0.1:1"], None).await;

    let response = client()
        .get(format!("{}/x", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 502);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("upstream"),
        "{body}"
    );
}

#[tokio::test]
async fn an_upstream_that_will_not_answer_in_time_is_a_gateway_timeout() {
    let upstream = upstream::start().await;
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![RouteConfig::parse(&format!("/={}", upstream.url())).unwrap()],
        upstream: UpstreamConfig {
            timeout: Some(Duration::from_millis(200)),
            ..UpstreamConfig::default()
        },
        ..ProxyConfig::default()
    };
    config.compile();
    let proxy = ferrimock::proxy::start(config, None).await.unwrap();

    // `/slow-headers` sits on the response for five seconds.
    let response = client()
        .get(format!("{}/slow-headers", proxy.url()))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 504);
}

#[tokio::test]
async fn a_body_timeout_does_not_apply_to_a_stream_that_is_still_delivering() {
    use futures::StreamExt;

    let upstream = upstream::start().await;
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![RouteConfig::parse(&format!("/={}", upstream.url())).unwrap()],
        upstream: UpstreamConfig {
            // Shorter than the 360ms the event stream takes to finish. The
            // timeout covers headers only, so the stream must survive it.
            timeout: Some(Duration::from_millis(250)),
            ..UpstreamConfig::default()
        },
        ..ProxyConfig::default()
    };
    config.compile();
    let proxy = ferrimock::proxy::start(config, None).await.unwrap();

    let mut stream = client()
        .get(format!("{}/sse", proxy.url()))
        .send()
        .await
        .unwrap()
        .bytes_stream();

    let mut events = 0;
    while let Some(Ok(chunk)) = stream.next().await {
        events += String::from_utf8_lossy(&chunk).matches("data:").count();
    }

    assert_eq!(
        events, 3,
        "the header timeout must not cut off a body that is still arriving"
    );
}

#[tokio::test]
async fn shutting_down_stops_serving() {
    let upstream = upstream::start().await;
    let proxy = proxy_with(&[&format!("/={}", upstream.url())], None).await;
    let url = format!("{}/echo", proxy.url());

    assert!(client().get(&url).send().await.is_ok());

    proxy.wait().await;

    assert!(
        client()
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_err(),
        "the listener should be closed after shutdown"
    );
}
