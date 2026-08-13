#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Declarative `network_error: true`.

use ferrimock::config::MockCollectionConfig;
use ferrimock::engine::types::ResponseGeneratorExt;
use ferrimock::types::NETWORK_ERROR_HEADER;

async fn dynamic_headers(yaml: &str) -> rustc_hash::FxHashMap<String, String> {
    let config = MockCollectionConfig::from_yaml(yaml).unwrap();
    let mock = config
        .mocks
        .into_iter()
        .next()
        .unwrap()
        .into_mock_definition()
        .await
        .unwrap();

    mock.response
        .generate_dynamic(
            "GET",
            "/boom",
            None,
            &http::HeaderMap::new(),
            None,
            rustc_hash::FxHashMap::default(),
            None,
        )
        .await
        .unwrap()
        .headers
        .unwrap_or_default()
}

#[tokio::test]
async fn network_error_emits_the_abort_marker() {
    let headers = dynamic_headers(
        r#"
mocks:
  - id: boom
    match:
      methods: ["GET"]
      url: "/boom"
    network_error: true
"#,
    )
    .await;

    assert_eq!(
        headers.get(NETWORK_ERROR_HEADER).map(String::as_str),
        Some("1")
    );
}

#[tokio::test]
async fn network_error_composes_with_delay() {
    let config = MockCollectionConfig::from_yaml(
        r#"
mocks:
  - id: slow-boom
    match:
      methods: ["GET"]
      url: "/slow-boom"
    delay: "200ms"
    network_error: true
"#,
    )
    .unwrap();

    let mock = config
        .mocks
        .into_iter()
        .next()
        .unwrap()
        .into_mock_definition()
        .await
        .unwrap();

    assert_eq!(
        mock.response.delay,
        Some(std::time::Duration::from_millis(200))
    );
}

#[tokio::test]
async fn network_error_rejects_a_response_body() {
    let config = MockCollectionConfig::from_yaml(
        r#"
mocks:
  - id: contradictory
    match:
      methods: ["GET"]
      url: "/boom"
    network_error: true
    response:
      json: { ok: true }
"#,
    )
    .unwrap();

    let err = config
        .mocks
        .into_iter()
        .next()
        .unwrap()
        .into_mock_definition()
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("network_error"),
        "error should name the offending field: {err}"
    );
}
