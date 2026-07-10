//! Integration tests for new API features:
//! - Call tracking
//! - Reset endpoint
//! - Quick mock API

use ferrimock::engine::matcher::MockMatcher;
use ferrimock::engine::registry::MockRegistry;
use ferrimock::engine::types::{
    BodySource, MockDefinition, RequestMatcher, ResponseGenerator, UrlPattern,
};
use http::{HeaderMap, Method, StatusCode};
use smallvec::smallvec;

fn create_test_mock(id: &str, path: &str) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact(path)],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline("{\"status\": \"ok\"}"),
        ),
        vars: None,
        streaming: None,
    }
}

#[test]
fn test_call_tracking_integration() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("api-users", "/api/users");
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry.clone());

    // Enable call tracking
    registry.enable_call_tracking("api-users", Some(50));
    assert!(registry.is_call_tracking_enabled("api-users"));

    // Simulate multiple requests
    for i in 0..10 {
        let result = matcher.find_match(&Method::GET, "/api/users", None, &HeaderMap::new(), None);

        assert!(result.is_some(), "Request {i} should match");
    }

    // Verify calls were tracked
    let call_count = registry.get_call_count("api-users");
    assert_eq!(call_count, 10, "Should have tracked 10 calls");

    let calls = registry.get_calls("api-users").unwrap();
    assert_eq!(calls.len(), 10);

    // Verify all calls have correct path
    for call in &calls {
        assert_eq!(call.method, "GET");
        assert_eq!(call.path, "/api/users");
    }

    // Clear calls
    registry.clear_calls("api-users");
    assert_eq!(registry.get_call_count("api-users"), 0);

    // Tracking should still be enabled
    assert!(registry.is_call_tracking_enabled("api-users"));
}

#[test]
fn test_call_tracking_with_different_methods() {
    let registry = MockRegistry::new();

    let mock = MockDefinition {
        id: "multi-method".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET, Method::POST, Method::PUT],
            url_patterns: smallvec![UrlPattern::exact("/api/resource")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry.clone());

    registry.enable_call_tracking("multi-method", None);

    // Make requests with different methods
    let methods = vec![
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::GET,
        Method::POST,
    ];

    for method in &methods {
        let result = matcher.find_match(
            method,
            "/api/resource",
            None,
            &HeaderMap::new(),
            if *method == Method::POST || *method == Method::PUT {
                Some(b"{\"data\": \"test\"}")
            } else {
                None
            },
        );
        assert!(result.is_some());
    }

    let calls = registry.get_calls("multi-method").unwrap();
    assert_eq!(calls.len(), 5);

    // Verify method distribution
    let get_count = calls.iter().filter(|c| c.method == "GET").count();
    let post_count = calls.iter().filter(|c| c.method == "POST").count();
    let put_count = calls.iter().filter(|c| c.method == "PUT").count();

    assert_eq!(get_count, 2);
    assert_eq!(post_count, 2);
    assert_eq!(put_count, 1);
}

#[test]
fn test_scope_with_call_tracking() {
    let registry = MockRegistry::new();

    // Create a scope
    let scope_info = registry.create_scope("test-scope".into(), None).unwrap();
    assert_eq!(scope_info.id, "test-scope");

    // Create mock in scope
    let mut mock = create_test_mock("scoped-mock", "/api/scoped");
    mock.scope = Some("test-scope".into());
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry.clone());

    // Enable tracking
    registry.enable_call_tracking("scoped-mock", None);

    // Make some requests
    for _ in 0..5 {
        matcher.find_match(&Method::GET, "/api/scoped", None, &HeaderMap::new(), None);
    }

    assert_eq!(registry.get_call_count("scoped-mock"), 5);

    // Delete scope
    let deleted = registry.delete_scope("test-scope").unwrap();
    assert_eq!(deleted, 1); // 1 mock deleted

    // Mock should be gone
    assert!(registry.get_mock("scoped-mock").is_none());

    // Call tracking should be automatically cleaned when mock is deleted
    // (note: in current implementation, tracking data persists but mock doesn't exist)
}

#[test]
fn test_call_limit_enforcement() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("limited", "/api/limited");
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry.clone());

    // Enable tracking with limit of 5
    registry.enable_call_tracking("limited", Some(5));

    // Make 10 requests
    for _ in 0..10 {
        matcher.find_match(&Method::GET, "/api/limited", None, &HeaderMap::new(), None);
    }

    // Should only keep last 5
    let calls = registry.get_calls("limited").unwrap();
    assert!(calls.len() <= 5, "Should not exceed limit of 5 calls");
}

#[test]
fn test_multiple_mocks_independent_tracking() {
    let registry = MockRegistry::new();

    registry.add_mock(create_test_mock("mock-1", "/api/one"));
    registry.add_mock(create_test_mock("mock-2", "/api/two"));
    registry.add_mock(create_test_mock("mock-3", "/api/three"));

    let matcher = MockMatcher::new(registry.clone());

    // Enable tracking for mock-1 and mock-2 only
    registry.enable_call_tracking("mock-1", None);
    registry.enable_call_tracking("mock-2", None);

    // Make requests to all three
    matcher.find_match(&Method::GET, "/api/one", None, &HeaderMap::new(), None);
    matcher.find_match(&Method::GET, "/api/one", None, &HeaderMap::new(), None);
    matcher.find_match(&Method::GET, "/api/two", None, &HeaderMap::new(), None);
    matcher.find_match(&Method::GET, "/api/three", None, &HeaderMap::new(), None);

    // Check tracking
    assert_eq!(registry.get_call_count("mock-1"), 2);
    assert_eq!(registry.get_call_count("mock-2"), 1);
    assert_eq!(registry.get_call_count("mock-3"), 0); // Not tracked

    // Get tracked mock IDs
    let tracked = registry.get_tracked_mock_ids();
    assert_eq!(tracked.len(), 2);
    assert!(tracked.contains(&"mock-1".to_string()));
    assert!(tracked.contains(&"mock-2".to_string()));
}

#[test]
fn test_call_tracking_with_query_params() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("with-query", "/api/search");
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry.clone());
    registry.enable_call_tracking("with-query", None);

    // Make requests with different query params
    matcher.find_match(
        &Method::GET,
        "/api/search",
        Some("q=test"),
        &HeaderMap::new(),
        None,
    );
    matcher.find_match(
        &Method::GET,
        "/api/search",
        Some("q=foo&limit=10"),
        &HeaderMap::new(),
        None,
    );
    matcher.find_match(&Method::GET, "/api/search", None, &HeaderMap::new(), None);

    let calls = registry.get_calls("with-query").unwrap();
    assert_eq!(calls.len(), 3);

    // Check query params were captured
    assert_eq!(calls[0].query, Some("q=test".to_string()));
    assert_eq!(calls[1].query, Some("q=foo&limit=10".to_string()));
    assert_eq!(calls[2].query, None);
}

#[test]
fn test_call_tracking_with_headers() {
    let registry = MockRegistry::new();
    let mock = create_test_mock("with-headers", "/api/auth");
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry.clone());
    registry.enable_call_tracking("with-headers", None);

    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer token123".parse().unwrap());
    headers.insert("content-type", "application/json".parse().unwrap());

    matcher.find_match(&Method::GET, "/api/auth", None, &headers, None);

    let calls = registry.get_calls("with-headers").unwrap();
    assert_eq!(calls.len(), 1);

    // Verify headers were captured
    let call_headers = &calls[0].headers;
    assert_eq!(
        call_headers.get("authorization"),
        Some(&"Bearer token123".to_string())
    );
    assert_eq!(
        call_headers.get("content-type"),
        Some(&"application/json".to_string())
    );
}

#[test]
fn test_reset_with_call_tracking() {
    let registry = MockRegistry::new();

    // Create multiple mocks with tracking
    for i in 1..=3 {
        let mock = create_test_mock(&format!("mock-{i}"), &format!("/api/{i}"));
        registry.add_mock(mock);
        registry.enable_call_tracking(&format!("mock-{i}"), None);
    }

    let matcher = MockMatcher::new(registry.clone());

    // Generate some calls
    for i in 1..=3 {
        matcher.find_match(
            &Method::GET,
            &format!("/api/{i}"),
            None,
            &HeaderMap::new(),
            None,
        );
    }

    // Verify tracking
    assert_eq!(registry.get_tracked_mock_ids().len(), 3);

    // Clear all tracking
    registry.clear_all_call_tracking();

    // Verify all cleared
    assert_eq!(registry.get_tracked_mock_ids().len(), 0);
    for i in 1..=3 {
        assert!(!registry.is_call_tracking_enabled(&format!("mock-{i}")));
    }
}
