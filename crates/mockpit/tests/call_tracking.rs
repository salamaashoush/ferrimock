#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Tests for mock call tracking functionality

use http::{Method, StatusCode};
use mockpit::engine::registry::{MockCall, MockRegistry};
use mockpit::engine::types::{BodySource, MockDefinition, RequestMatcher, ResponseGenerator};
use rustc_hash::FxHashMap;
use smallvec::smallvec;

fn create_test_mock(id: &str) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
    }
}

#[test]
fn test_enable_call_tracking() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("test-mock");
    registry.add_mock(mock);

    // Initially tracking should be disabled
    assert!(!registry.is_call_tracking_enabled("test-mock"));

    // Enable tracking
    registry.enable_call_tracking("test-mock", None);
    assert!(registry.is_call_tracking_enabled("test-mock"));

    // Disable tracking
    registry.disable_call_tracking("test-mock");
    assert!(!registry.is_call_tracking_enabled("test-mock"));
}

#[test]
fn test_record_calls() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("test-mock");
    registry.add_mock(mock);

    // Enable tracking
    registry.enable_call_tracking("test-mock", None);

    // Record some calls
    for i in 0..5 {
        let call = MockCall::new(
            "GET".to_string(),
            format!("/api/test/{i}"),
            None,
            FxHashMap::default(),
            None,
        );
        registry.record_call("test-mock", call);
    }

    // Check call count
    assert_eq!(registry.get_call_count("test-mock"), 5);

    // Get all calls
    let calls = registry.get_calls("test-mock").unwrap();
    assert_eq!(calls.len(), 5);

    // Verify call details
    for (i, call) in calls.iter().enumerate() {
        assert_eq!(call.method, "GET");
        assert_eq!(call.path, format!("/api/test/{i}"));
    }
}

#[test]
fn test_call_tracking_limit() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("test-mock");
    registry.add_mock(mock);

    // Enable tracking with max 10 calls
    registry.enable_call_tracking("test-mock", Some(10));

    // Record 15 calls (more than the limit)
    for i in 0..15 {
        let call = MockCall::new(
            "GET".to_string(),
            format!("/api/test/{i}"),
            None,
            FxHashMap::default(),
            None,
        );
        registry.record_call("test-mock", call);
    }

    // Should only keep the last 10 calls (oldest ones removed)
    let count = registry.get_call_count("test-mock");
    assert!(count <= 10, "Call count {count} exceeds limit of 10");

    let calls = registry.get_calls("test-mock").unwrap();
    assert!(
        calls.len() <= 10,
        "Stored {} calls, expected at most 10",
        calls.len()
    );
}

#[test]
fn test_clear_calls() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("test-mock");
    registry.add_mock(mock);

    // Enable tracking and record some calls
    registry.enable_call_tracking("test-mock", None);
    for i in 0..5 {
        let call = MockCall::new(
            "GET".to_string(),
            format!("/api/test/{i}"),
            None,
            FxHashMap::default(),
            None,
        );
        registry.record_call("test-mock", call);
    }

    assert_eq!(registry.get_call_count("test-mock"), 5);

    // Clear calls
    registry.clear_calls("test-mock");
    assert_eq!(registry.get_call_count("test-mock"), 0);

    // Tracking should still be enabled
    assert!(registry.is_call_tracking_enabled("test-mock"));
}

#[test]
fn test_record_calls_with_body() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("test-mock");
    registry.add_mock(mock);

    registry.enable_call_tracking("test-mock", None);

    // Record a call with body
    let body = b"{\"name\": \"test\"}";
    let call = MockCall::new(
        "POST".to_string(),
        "/api/users".to_string(),
        None,
        FxHashMap::default(),
        Some(body),
    );

    registry.record_call("test-mock", call);

    let calls = registry.get_calls("test-mock").unwrap();
    assert_eq!(calls.len(), 1);

    // Body hash should be set
    assert!(calls[0].body_hash.is_some());

    // Verify the hash is consistent
    let call2 = MockCall::new(
        "POST".to_string(),
        "/api/users".to_string(),
        None,
        FxHashMap::default(),
        Some(body),
    );
    assert_eq!(call2.body_hash, calls[0].body_hash);
}

#[test]
fn test_calls_only_recorded_when_enabled() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("test-mock");
    registry.add_mock(mock);

    // Try to record without enabling tracking
    let call = MockCall::new(
        "GET".to_string(),
        "/api/test".to_string(),
        None,
        FxHashMap::default(),
        None,
    );
    registry.record_call("test-mock", call);

    // No calls should be recorded
    assert_eq!(registry.get_call_count("test-mock"), 0);
    assert!(registry.get_calls("test-mock").is_none());
}

#[test]
fn test_get_tracked_mock_ids() {
    let registry = MockRegistry::new();

    registry.add_mock(create_test_mock("mock-1"));
    registry.add_mock(create_test_mock("mock-2"));
    registry.add_mock(create_test_mock("mock-3"));

    // Enable tracking for some mocks
    registry.enable_call_tracking("mock-1", None);
    registry.enable_call_tracking("mock-3", None);

    let tracked_ids = registry.get_tracked_mock_ids();
    assert_eq!(tracked_ids.len(), 2);
    assert!(tracked_ids.contains(&"mock-1".to_string()));
    assert!(tracked_ids.contains(&"mock-3".to_string()));
    assert!(!tracked_ids.contains(&"mock-2".to_string()));
}

#[test]
fn test_clear_all_call_tracking() {
    let registry = MockRegistry::new();

    registry.add_mock(create_test_mock("mock-1"));
    registry.add_mock(create_test_mock("mock-2"));

    // Enable tracking for both
    registry.enable_call_tracking("mock-1", None);
    registry.enable_call_tracking("mock-2", None);

    // Record some calls
    let call = MockCall::new(
        "GET".to_string(),
        "/test".to_string(),
        None,
        FxHashMap::default(),
        None,
    );
    registry.record_call("mock-1", call.clone());
    registry.record_call("mock-2", call);

    assert_eq!(registry.get_tracked_mock_ids().len(), 2);

    // Clear all tracking
    registry.clear_all_call_tracking();

    assert_eq!(registry.get_tracked_mock_ids().len(), 0);
    assert!(!registry.is_call_tracking_enabled("mock-1"));
    assert!(!registry.is_call_tracking_enabled("mock-2"));
}
