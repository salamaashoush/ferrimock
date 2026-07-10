#![cfg(feature = "server")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Declarative SSE mock tests: build a registry from YAML and drain the
//! streaming responses produced by the canonical serve path.

use ferrimock::engine::{MockMatcher, MockRegistry};

async fn matcher_for(yaml: &str) -> MockMatcher {
    let collection: ferrimock::config::MockCollectionConfig =
        serde_yaml::from_str(yaml).expect("parse yaml");
    let registry = MockRegistry::new();
    for mock in collection.mocks {
        let def = mock.into_mock_definition().await.expect("lower mock");
        registry.add_mock(def);
    }
    MockMatcher::new(registry)
}

/// Read a streaming body until `predicate` accepts the accumulated text.
/// Upstream relays reconnect instead of ending the client stream, so
/// tests read what they expect and drop the body (closing the pump).
async fn read_stream_until(
    response: axum::response::Response,
    predicate: impl Fn(&str) -> bool,
) -> String {
    use futures::StreamExt;

    let mut stream = response.into_body().into_data_stream();
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while !predicate(&text) {
        let chunk = tokio::time::timeout_at(deadline, stream.next())
            .await
            .unwrap_or_else(|_| panic!("stream stalled before the expected frames: {text}"));
        match chunk {
            Some(Ok(bytes)) => text.push_str(&String::from_utf8_lossy(&bytes)),
            Some(Err(e)) => panic!("stream errored: {e}"),
            None => panic!("stream ended early: {text}"),
        }
    }
    text
}

async fn get_body(matcher: &MockMatcher, path: &str, query: Option<&str>) -> (u16, String) {
    let response = ferrimock::services::serve::respond(
        matcher,
        &http::Method::GET,
        path,
        query,
        &http::HeaderMap::new(),
        None,
    )
    .await;
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("drain body");
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn declarative_events_play_in_order_with_metadata() {
    let matcher = matcher_for(
        r#"
mocks:
  - id: ticker
    match: { url: "/api/ticker", methods: [GET] }
    sse:
      retry: 3000
      events:
        - "hello"
        - { event: price, id: "1", data: { px: 123 } }
        - { event: done, data: finished }
"#,
    )
    .await;

    let response = ferrimock::services::serve::respond(
        &matcher,
        &http::Method::GET,
        "/api/ticker",
        None,
        &http::HeaderMap::new(),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );
    assert_eq!(
        response
            .headers()
            .get("x-mock-id")
            .and_then(|v| v.to_str().ok()),
        Some("ticker")
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("drain");
    let text = String::from_utf8_lossy(&body);

    let retry_pos = text.find("retry:").expect("retry field");
    let hello_pos = text
        .find("data: hello")
        .or_else(|| text.find("data:hello"))
        .expect("hello");
    let price_pos = text
        .find("event: price")
        .or_else(|| text.find("event:price"))
        .expect("price event");
    let done_pos = text
        .find("event: done")
        .or_else(|| text.find("event:done"))
        .expect("done event");
    assert!(retry_pos < hello_pos && hello_pos < price_pos && price_pos < done_pos);
    assert!(text.contains("3000"));
    assert!(
        text.contains(r#"{"px":123}"#),
        "object data JSON-serialized: {text}"
    );
    assert!(text.contains("id: 1") || text.contains("id:1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn per_event_delay_and_repeat() {
    let matcher = matcher_for(
        r#"
mocks:
  - id: repeater
    match: { url: "/api/stream" }
    sse:
      repeat: 3
      events:
        - { data: tick, delay: 20ms }
"#,
    )
    .await;

    let started = std::time::Instant::now();
    let (status, text) = get_body(&matcher, "/api/stream", None).await;
    let elapsed = started.elapsed();

    assert_eq!(status, 200);
    assert_eq!(
        text.matches("tick").count(),
        3,
        "repeat plays the list 3x: {text}"
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(60),
        "3 events x 20ms delay, got {elapsed:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_render_with_captures_and_query() {
    let matcher = matcher_for(
        r#"
mocks:
  - id: templated
    match: { url: "/api/feed/:channel" }
    sse:
      events:
        - { event: update, data_template: '{"chan":"{{ captures.channel }}","q":"{{ query.filter }}"}' }
"#,
    )
    .await;

    let (status, text) = get_body(&matcher, "/api/feed/news", Some("filter=hot")).await;
    assert_eq!(status, 200);
    assert!(text.contains(r#""chan":"news""#), "{text}");
    assert!(text.contains(r#""q":"hot""#), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn close_after_false_holds_the_connection_open() {
    let matcher = matcher_for(
        r#"
mocks:
  - id: open-ended
    match: { url: "/api/open" }
    sse:
      close_after: false
      events:
        - "first"
"#,
    )
    .await;

    let response = ferrimock::services::serve::respond(
        &matcher,
        &http::Method::GET,
        "/api/open",
        None,
        &http::HeaderMap::new(),
        None,
    )
    .await;

    // Draining never finishes because the stream stays open.
    let drained = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await;
    assert!(drained.is_err(), "close_after: false must keep streaming");
}

#[tokio::test(flavor = "multi_thread")]
async fn streaming_config_conflicts_are_rejected() {
    let yaml = r#"
mocks:
  - id: bad
    match: { url: "/x" }
    sse:
      events: ["a"]
    ws:
      echo: true
"#;
    let collection: ferrimock::config::MockCollectionConfig =
        serde_yaml::from_str(yaml).expect("parse");
    let err = collection.mocks[0]
        .clone()
        .into_mock_definition()
        .await
        .expect_err("sse+ws must be rejected");
    assert!(err.to_string().contains("sse"), "{err}");

    let yaml = r#"
mocks:
  - id: bad-body
    match: { url: "/x" }
    response: { body: "nope" }
    sse:
      events: ["a"]
"#;
    let collection: ferrimock::config::MockCollectionConfig =
        serde_yaml::from_str(yaml).expect("parse");
    let err = collection.mocks[0]
        .clone()
        .into_mock_definition()
        .await
        .expect_err("sse+body must be rejected");
    assert!(err.to_string().contains("full mock"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn declarative_upstream_relays_the_real_stream() {
    // Real upstream: a ferrimock server playing declarative events.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("upstream.yaml"),
        r#"
mocks:
  - id: real-stream
    match: { url: "/real" }
    sse:
      events:
        - { id: "1", data: alpha }
        - { event: tick, data: { n: 2 } }
"#,
    )
    .expect("write");
    let upstream = ferrimock::services::serve::start(ferrimock::services::serve::ServeInput {
        port: 0,
        mocks_dir: Some(dir.path().to_string_lossy().into_owned()),
        ..ferrimock::services::serve::ServeInput::default()
    })
    .await
    .expect("upstream start");

    let matcher = matcher_for(&format!(
        r#"
mocks:
  - id: relay
    match: {{ url: "/relay" }}
    sse:
      upstream: {}/real
"#,
        upstream.url
    ))
    .await;

    let response = ferrimock::services::serve::respond(
        &matcher,
        &http::Method::GET,
        "/relay",
        None,
        &http::HeaderMap::new(),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("text/event-stream")
    );

    let text = read_stream_until(response, |t| {
        t.contains("id:1\ndata:alpha") && t.contains("event:tick\ndata:{\"n\":2}")
    })
    .await;
    assert!(text.contains("id:1\ndata:alpha"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn declarative_upstream_reconnects_with_last_event_id() {
    // Real upstream: finite playback honoring the announced retry delay;
    // every pass echoes the Last-Event-ID header the dialer sent.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("upstream.yaml"),
        r#"
mocks:
  - id: real-stream
    match: { url: "/real" }
    sse:
      retry: 50
      events:
        - { id: "7", data: alpha }
        - data_template: 'last={{ headers["last-event-id"] | default(value="none") }}'
"#,
    )
    .expect("write");
    let upstream = ferrimock::services::serve::start(ferrimock::services::serve::ServeInput {
        port: 0,
        mocks_dir: Some(dir.path().to_string_lossy().into_owned()),
        ..ferrimock::services::serve::ServeInput::default()
    })
    .await
    .expect("upstream start");

    let matcher = matcher_for(&format!(
        r#"
mocks:
  - id: relay
    match: {{ url: "/relay" }}
    sse:
      upstream: {}/real
"#,
        upstream.url
    ))
    .await;

    let response = ferrimock::services::serve::respond(
        &matcher,
        &http::Method::GET,
        "/relay",
        None,
        &http::HeaderMap::new(),
        None,
    )
    .await;
    assert_eq!(response.status(), 200);

    // The upstream closes after each playback; the pump must redial
    // after the announced 50ms retry, sending the last seen id.
    let text = read_stream_until(response, |t| t.contains("data:last=7")).await;
    assert!(
        text.matches("data:alpha").count() >= 2,
        "expected a second playback after reconnect: {text}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_wrong_content_type_answers_bad_gateway() {
    // Real upstream: a plain JSON mock — not an SSE stream.
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("upstream.yaml"),
        r#"
mocks:
  - id: plain
    match: { url: "/real" }
    response: { body: "{}" }
"#,
    )
    .expect("write");
    let upstream = ferrimock::services::serve::start(ferrimock::services::serve::ServeInput {
        port: 0,
        mocks_dir: Some(dir.path().to_string_lossy().into_owned()),
        ..ferrimock::services::serve::ServeInput::default()
    })
    .await
    .expect("upstream start");

    let matcher = matcher_for(&format!(
        r#"
mocks:
  - id: relay
    match: {{ url: "/relay" }}
    sse:
      upstream: {}/real
"#,
        upstream.url
    ))
    .await;

    let (status, body) = get_body(&matcher, "/relay", None).await;
    assert_eq!(status, 502, "{body}");
    assert!(body.contains("responded"), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_failure_answers_bad_gateway() {
    // Nothing listens on this port.
    let matcher = matcher_for(
        r#"
mocks:
  - id: dead-relay
    match: { url: "/dead" }
    sse:
      upstream: http://127.0.0.1:1/real
"#,
    )
    .await;

    let (status, body) = get_body(&matcher, "/dead", None).await;
    assert_eq!(status, 502, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_config_conflicts_are_rejected() {
    let yaml = r#"
mocks:
  - id: bad
    match: { url: "/x" }
    sse:
      upstream: http://real.example.com/stream
      events: ["a"]
"#;
    let collection: ferrimock::config::MockCollectionConfig =
        serde_yaml::from_str(yaml).expect("parse");
    let err = collection.mocks[0]
        .clone()
        .into_mock_definition()
        .await
        .expect_err("upstream+events must be rejected");
    assert!(err.to_string().contains("upstream"), "{err}");

    let yaml = r#"
mocks:
  - id: bad-scheme
    match: { url: "/x" }
    sse:
      upstream: ws://real.example.com/stream
"#;
    let collection: ferrimock::config::MockCollectionConfig =
        serde_yaml::from_str(yaml).expect("parse");
    let err = collection.mocks[0]
        .clone()
        .into_mock_definition()
        .await
        .expect_err("non-http upstream must be rejected");
    assert!(err.to_string().contains("http"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_mock_ends_an_open_stream() {
    let matcher = matcher_for(
        r#"
mocks:
  - id: endless
    match: { url: "/api/endless" }
    sse:
      close_after: false
      events:
        - "first"
"#,
    )
    .await;

    let response = ferrimock::services::serve::respond(
        &matcher,
        &http::Method::GET,
        "/api/endless",
        None,
        &http::HeaderMap::new(),
        None,
    )
    .await;

    let drain = tokio::spawn(axum::body::to_bytes(response.into_body(), usize::MAX));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!drain.is_finished(), "stream must still be open");

    matcher.streaming_connections().close_mock("endless");

    let body = tokio::time::timeout(std::time::Duration::from_secs(2), drain)
        .await
        .expect("stream must end after mock teardown")
        .expect("join")
        .expect("body");
    assert!(String::from_utf8_lossy(&body).contains("first"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ws_mock_hit_without_upgrade_returns_426() {
    let matcher = matcher_for(
        r#"
mocks:
  - id: only-ws
    match: { url: "/ws/only" }
    ws:
      echo: true
"#,
    )
    .await;

    // The ws mock carries an upgrade header matcher; a plain GET without
    // the header does not match it at all -> 404.
    let (status, _) = get_body(&matcher, "/ws/only", None).await;
    assert_eq!(status, 404);

    // With upgrade headers it matches, and respond() (non-upgrade path)
    // answers 426 Upgrade Required.
    let mut headers = http::HeaderMap::new();
    headers.insert("upgrade", http::HeaderValue::from_static("websocket"));
    headers.insert("connection", http::HeaderValue::from_static("Upgrade"));
    let response = ferrimock::services::serve::respond(
        &matcher,
        &http::Method::GET,
        "/ws/only",
        None,
        &headers,
        None,
    )
    .await;
    assert_eq!(response.status(), 426);
}
