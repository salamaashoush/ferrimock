#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Additional matcher tests targeting edge cases and uncovered code paths

use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use mockpit::engine::matcher::{MockAction, MockMatcher};
use mockpit::engine::registry::MockRegistry;
use mockpit::engine::types::{
    BodyMatcher, BodySource, HeaderMatcher, MockDefinition, QueryMatcher, RequestMatcher,
    ResponseGenerator, ResponseMode, UrlPattern,
};
use smallvec::smallvec;

/// Helper to create a test mock
fn create_test_mock(
    id: &str,
    priority: u32,
    methods: smallvec::SmallVec<[Method; 2]>,
    url_patterns: smallvec::SmallVec<[UrlPattern; 1]>,
    header_matchers: smallvec::SmallVec<[HeaderMatcher; 2]>,
    query_matchers: smallvec::SmallVec<[QueryMatcher; 2]>,
    body_matcher: Option<BodyMatcher>,
) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority,
        enabled: true,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods,
            url_patterns,
            header_matchers,
            query_matchers,
            body_matcher,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
    }
}

/// Test cache clearing
#[test]
fn test_clear_cache() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // First call to populate cache
    let result1 = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result1.is_some(), "First call should find match");
    assert_eq!(
        result1.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );

    // Clear cache
    matcher.clear_cache();

    // Should still work after cache clear
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(
        result.is_some(),
        "Should still find match after cache clear"
    );
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should still match the correct mock"
    );
}

/// Test cache with specific size
#[test]
fn test_custom_cache_size() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    // Create matcher with small cache size
    let matcher = MockMatcher::with_cache_size(registry, 5);
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some(), "Should find match with custom cache size");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );
}

/// Test that query parameters prevent caching
#[test]
fn test_cache_skipped_with_query_params() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Request with query params should not use cache
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("foo=bar"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some(), "Should find match even with query params");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );
}

/// Test that body prevents caching
#[test]
fn test_cache_skipped_with_body() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Request with body should not use cache
    let body = b"test body";
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_some(), "Should find match even with body");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );
}

/// Test that headers prevent caching
#[test]
fn test_cache_skipped_with_headers() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Request with headers should not use cache
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-test"),
        HeaderValue::from_static("value"),
    );

    let result = matcher.find_match(&Method::GET, "/test", None, &headers, None);
    assert!(result.is_some(), "Should find match even with headers");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );
}

/// Test that mocks with query matchers are not cached
#[test]
fn test_cache_skipped_for_mocks_with_query_matchers() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![QueryMatcher::exact("foo", "bar")],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Mock with query matchers should not be cached even if request is simple
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    // Should not match because query matcher requires "foo=bar"
    assert!(result.is_none());
}

/// Test that mocks with header matchers are not cached
#[test]
fn test_cache_skipped_for_mocks_with_header_matchers() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![HeaderMatcher::present(HeaderName::from_static(
            "authorization"
        ))],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Mock with header matchers should not be cached
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    // Should not match because header matcher requires authorization header
    assert!(result.is_none());
}

/// Test that mocks with body matchers are not cached
#[test]
fn test_cache_skipped_for_mocks_with_body_matchers() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::contains("important")),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Mock with body matchers should not be cached
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    // Should not match because body matcher requires specific content
    assert!(result.is_none());
}

/// Test URL matching with query string - exact match including query
#[test]
fn test_url_match_with_query_string_exact() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/api/search?q=test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match when query string is included
    let result = matcher.find_match(
        &Method::GET,
        "/api/search",
        Some("q=test"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    // Should not match with different query
    let result = matcher.find_match(
        &Method::GET,
        "/api/search",
        Some("q=other"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_none());
}

/// Test URL matching with query string - path-only match
#[test]
fn test_url_match_path_only_backwards_compat() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/api/search")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match path-only even with query string present (backwards compatibility)
    let result = matcher.find_match(
        &Method::GET,
        "/api/search",
        Some("q=anything"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());
}

/// Test body matcher with no body provided
#[test]
fn test_body_matcher_no_body_provided() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::contains("required")),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should not match when body matcher is specified but no body provided
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_none());
}

/// Test body matcher with exact content
#[test]
fn test_body_matcher_contains() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::contains("hello")),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match when body contains the substring
    let body = b"hello world";
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_some());

    // Should not match when body doesn't contain the substring
    let body = b"goodbye world";
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_none());
}

/// Test body matcher with regex
#[test]
fn test_body_matcher_regex() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::regex(r"\d{3}-\d{4}").unwrap()),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match phone number pattern
    let body = b"Call me at 555-1234";
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_some());

    // Should not match without pattern
    let body = b"No phone number here";
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_none());
}

/// Test body matcher with JSON path
#[test]
fn test_body_matcher_json_path() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::json_path(
            "$.user.name",
            serde_json::json!("Alice"),
        )),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match when JSON path value matches
    let body = br#"{"user": {"name": "Alice", "age": 30}}"#;
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_some());

    // Should not match when JSON path value differs
    let body = br#"{"user": {"name": "Bob", "age": 25}}"#;
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_none());
}

/// Test body matcher with JSON equals
#[test]
fn test_body_matcher_json_equals() {
    let registry = MockRegistry::new();
    let expected = serde_json::json!({
      "action": "create",
      "data": {"name": "test"}
    });
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::json_equals(expected.clone())),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match exact JSON
    let body = serde_json::to_string(&expected).unwrap();
    let result = matcher.find_match(
        &Method::POST,
        "/test",
        None,
        &HeaderMap::new(),
        Some(body.as_bytes()),
    );
    assert!(result.is_some());

    // Should not match different JSON
    let different = serde_json::json!({
      "action": "update",
      "data": {"name": "test"}
    });
    let body = serde_json::to_string(&different).unwrap();
    let result = matcher.find_match(
        &Method::POST,
        "/test",
        None,
        &HeaderMap::new(),
        Some(body.as_bytes()),
    );
    assert!(result.is_none());
}

/// Test query matcher - exact value
#[test]
fn test_query_matcher_exact() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![QueryMatcher::exact("page", "1")],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match exact query value
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("page=1"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    // Should not match different value
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("page=2"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_none());

    // Should not match missing query param
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_none());
}

/// Test query matcher - regex
#[test]
fn test_query_matcher_regex() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![QueryMatcher::regex("id", r"^\d+$").unwrap()],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match numeric ID
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("id=123"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    // Should not match non-numeric ID
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("id=abc"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_none());
}

/// Test query matcher - present
#[test]
fn test_query_matcher_present() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![QueryMatcher::present("debug")],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match when query param is present
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("debug=true"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("debug="),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    // Should not match when query param is absent
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_none());
}

/// Test query matcher - absent
#[test]
fn test_query_matcher_absent() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![QueryMatcher::absent("cache")],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match when query param is absent
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some());

    // Should not match when query param is present
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("cache=true"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_none());
}

/// Test header matcher - regex
#[test]
fn test_header_matcher_regex() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![
            HeaderMatcher::regex(HeaderName::from_static("user-agent"), r"^Mozilla/.*").unwrap()
        ],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static("Mozilla/5.0 (X11; Linux x86_64)"),
    );

    // Should match Mozilla user agent
    let result = matcher.find_match(&Method::GET, "/test", None, &headers, None);
    assert!(result.is_some());

    // Should not match different user agent
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("user-agent"),
        HeaderValue::from_static("curl/7.68.0"),
    );
    let result = matcher.find_match(&Method::GET, "/test", None, &headers, None);
    assert!(result.is_none());
}

/// Test header matcher - absent
#[test]
fn test_header_matcher_absent() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![HeaderMatcher::absent(HeaderName::from_static("x-cache"))],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match when header is absent
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some());

    // Should not match when header is present
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-cache"),
        HeaderValue::from_static("hit"),
    );
    let result = matcher.find_match(&Method::GET, "/test", None, &headers, None);
    assert!(result.is_none());
}

/// Test URL pattern with captures
#[test]
fn test_url_captures_extraction() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::regex(r"^/users/(?P<user_id>\d+)/posts/(?P<post_id>\d+)$").unwrap()],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should extract named captures
    let result = matcher.find_match(
        &Method::GET,
        "/users/42/posts/123",
        None,
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    let mock_match = result.unwrap();
    assert_eq!(mock_match.captures.get("user_id"), Some(&"42".to_string()));
    assert_eq!(mock_match.captures.get("post_id"), Some(&"123".to_string()));
}

/// Test URL pattern with no captures
#[test]
fn test_url_no_captures() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some());

    let mock_match = result.unwrap();
    assert!(mock_match.captures.is_empty());
}

/// Test multiple URL patterns with first having captures
#[test]
fn test_multiple_url_patterns_first_with_captures() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![
            UrlPattern::regex(r"^/api/(?P<version>v\d+)/users$").unwrap(),
            UrlPattern::exact("/api/users"),
        ],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match first pattern and extract captures
    let result = matcher.find_match(&Method::GET, "/api/v1/users", None, &HeaderMap::new(), None);
    assert!(result.is_some());

    let mock_match = result.unwrap();
    assert_eq!(mock_match.captures.get("version"), Some(&"v1".to_string()));
}

/// Test suffix URL pattern
#[test]
fn test_url_pattern_suffix() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::suffix(".pdf")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match URLs ending with .pdf
    let result = matcher.find_match(
        &Method::GET,
        "/documents/file.pdf",
        None,
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    // Should not match other extensions
    let result = matcher.find_match(
        &Method::GET,
        "/documents/file.txt",
        None,
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_none());
}

/// Test glob URL pattern
#[test]
fn test_url_pattern_glob() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::glob("/api/**/users").unwrap()],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match nested paths
    let result = matcher.find_match(&Method::GET, "/api/v1/users", None, &HeaderMap::new(), None);
    assert!(result.is_some());

    let result = matcher.find_match(
        &Method::GET,
        "/api/v2/admin/users",
        None,
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some());

    // Should not match without proper prefix/suffix
    let result = matcher.find_match(&Method::GET, "/api/v1/posts", None, &HeaderMap::new(), None);
    assert!(result.is_none());
}

/// Test call tracking integration with matcher
#[test]
fn test_call_tracking_in_matcher() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    // Enable call tracking
    registry.enable_call_tracking("test", None);

    let matcher = MockMatcher::new(registry.clone());

    // Make a request
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some(), "Should find match for tracking");

    // Verify call was tracked
    assert_eq!(
        registry.get_call_count("test"),
        1,
        "Should track exactly one call"
    );

    let calls = registry.get_calls("test");
    assert!(calls.is_some(), "Should have calls recorded");
    assert_eq!(
        calls.unwrap().len(),
        1,
        "Should have exactly one recorded call"
    );
}

/// Test call tracking with body in matcher
#[test]
fn test_call_tracking_with_body() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::POST],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    registry.enable_call_tracking("test", None);

    let matcher = MockMatcher::new(registry.clone());

    let body = b"test request body";
    let result = matcher.find_match(&Method::POST, "/test", None, &HeaderMap::new(), Some(body));
    assert!(result.is_some(), "Should find match with body");

    // Verify call was tracked
    assert_eq!(
        registry.get_call_count("test"),
        1,
        "Should track exactly one call"
    );
    let calls = registry.get_calls("test");
    assert!(calls.is_some(), "Should have calls recorded");
    let calls = calls.unwrap();
    assert_eq!(calls.len(), 1, "Should have exactly one recorded call");
    assert!(calls[0].body_hash.is_some(), "Body hash should be recorded");
}

/// Test call tracking from cache hit
#[test]
fn test_call_tracking_from_cache() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    registry.enable_call_tracking("test", None);

    let matcher = MockMatcher::new(registry.clone());

    // First call - cache miss
    let result1 = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result1.is_some(), "First call should find match");

    // Second call - cache hit
    let result2 = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(
        result2.is_some(),
        "Second call should find match from cache"
    );

    // Both calls should be tracked
    assert_eq!(
        registry.get_call_count("test"),
        2,
        "Should track both cache miss and cache hit"
    );

    let calls = registry.get_calls("test");
    assert!(calls.is_some(), "Should have calls recorded");
    assert_eq!(
        calls.unwrap().len(),
        2,
        "Should have exactly two recorded calls"
    );
}

/// Test that cache is invalidated when mock is removed
#[test]
fn test_cache_invalidation_on_mock_removal() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry.clone());

    // Populate cache
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some(), "Should find match before removal");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );

    // Remove mock
    let removed = registry.remove_mock("test");
    assert!(removed.is_some(), "Mock should be successfully removed");

    // Cache should return None after mock is removed
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_none(), "Should not find match after mock removal");
}

/// Test that cache is invalidated when mock becomes non-cacheable
#[test]
fn test_cache_invalidation_on_mock_modification() {
    let registry = MockRegistry::new();
    let mut mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock.clone());

    let matcher = MockMatcher::new(registry.clone());

    // Populate cache
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(result.is_some(), "Should find match before modification");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );

    // Modify mock to have header matcher (makes it non-cacheable)
    mock.request
        .header_matchers
        .push(HeaderMatcher::present(HeaderName::from_static("x-test")));
    registry.add_mock(mock); // Re-add modified mock

    // Cache should be invalidated and new matcher should be used
    let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);
    assert!(
        result.is_none(),
        "Should not match because header is missing after modification"
    );

    // Verify it DOES match with the required header
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-test"),
        HeaderValue::from_static("value"),
    );
    let result = matcher.find_match(&Method::GET, "/test", None, &headers, None);
    assert!(
        result.is_some(),
        "Should match with required header present"
    );
}

/// Test query param URL decoding in matcher
#[test]
fn test_query_param_url_decoding() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![QueryMatcher::exact("name", "John Doe")],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // URL-encoded query param should be decoded and matched
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("name=John%20Doe"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some(), "Should match URL-encoded query param");
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );

    // Test that non-encoded version also works
    let result = matcher.find_match(
        &Method::GET,
        "/test",
        Some("name=John Doe"),
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some(), "Should match non-encoded query param");
}

/// Test empty URL patterns matches all
#[test]
fn test_empty_url_patterns_matches_all() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![],
        smallvec![],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    // Should match any URL
    let result = matcher.find_match(
        &Method::GET,
        "/any/path/here",
        None,
        &HeaderMap::new(),
        None,
    );
    assert!(
        result.is_some(),
        "Should match any URL when patterns are empty"
    );
    assert_eq!(
        result.unwrap().mock.id,
        "test",
        "Should match the correct mock"
    );

    // Test multiple different URLs
    let result = matcher.find_match(
        &Method::POST,
        "/completely/different",
        None,
        &HeaderMap::new(),
        None,
    );
    assert!(result.is_some(), "Should match different URL");

    let result = matcher.find_match(&Method::PUT, "/", None, &HeaderMap::new(), None);
    assert!(result.is_some(), "Should match root path");
}

/// Test async try_match_parts with patch mode
#[tokio::test]
async fn test_try_match_patch_mode() {
    let registry = MockRegistry::new();
    let mut mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );

    // Set response to patch mode
    mock.response.mode = ResponseMode::Patch { operations: vec![] };
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let action = matcher
        .try_match_parts(&Method::GET, "/test", None, &HeaderMap::new(), None)
        .await;
    assert!(action.is_some());

    match action.unwrap() {
        MockAction::PatchUpstream {
            response_patches,
            mock_id,
            ..
        } => {
            assert_eq!(mock_id, "test");
            assert_eq!(response_patches.len(), 0);
        }
        MockAction::FullMock(_) => panic!("Expected PatchUpstream action"),
    }
}

/// Test async try_match_parts with template that has structured response
#[tokio::test]
async fn test_try_match_template_structured_response() {
    let registry = MockRegistry::new();
    let mut mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );

    // Use template that returns structured response
    mock.response.set_body(BodySource::template(
        r#"{"status": 201, "headers": {"X-Custom": "value"}, "body": {"message": "created"}}"#,
    ));
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let action = matcher
        .try_match_parts(&Method::GET, "/test", None, &HeaderMap::new(), None)
        .await;
    assert!(action.is_some());

    match action.unwrap() {
        MockAction::FullMock(response) => {
            assert_eq!(response.status(), StatusCode::CREATED);
            assert_eq!(response.headers().get("X-Custom").unwrap(), "value");
            assert!(response.headers().get("X-Mock-Id").is_some());
        }
        MockAction::PatchUpstream { .. } => panic!("Expected FullMock action"),
    }
}

/// Test async try_match_parts with disabled registry
#[tokio::test]
async fn test_try_match_disabled_registry() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );
    registry.add_mock(mock);

    // Verify registry is enabled before disabling
    assert!(
        registry.is_enabled(),
        "Registry should be enabled initially"
    );

    registry.disable();
    assert!(
        !registry.is_enabled(),
        "Registry should be disabled after calling disable"
    );

    let matcher = MockMatcher::new(registry);

    let action = matcher
        .try_match_parts(&Method::GET, "/test", None, &HeaderMap::new(), None)
        .await;
    assert!(
        action.is_none(),
        "Should not match when registry is disabled"
    );
}

/// Test async try_match_parts with POST body reading
#[tokio::test]
async fn test_try_match_with_post_body() {
    let registry = MockRegistry::new();
    let mock = create_test_mock(
        "test",
        100,
        smallvec![Method::POST],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        Some(BodyMatcher::contains("data")),
    );
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let body = br#"{"data": "value"}"#;
    let action = matcher
        .try_match_parts(
            &Method::POST,
            "/test",
            None,
            &HeaderMap::new(),
            Some(body.as_slice()),
        )
        .await;
    assert!(action.is_some(), "Should match POST request with body");

    // Verify the action is a FullMock
    match action.unwrap() {
        MockAction::FullMock(response) => {
            assert_eq!(response.status(), StatusCode::OK, "Should return OK status");
            assert!(
                response.headers().get("X-Mock-Id").is_some(),
                "Should have mock ID header"
            );
        }
        MockAction::PatchUpstream { .. } => panic!("Expected FullMock action"),
    }
}

/// Test apply_patches static method
#[tokio::test]
async fn test_apply_patches() {
    use bytes::Bytes;
    use http::Response;

    // Create a mock upstream response
    let upstream = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Bytes::from(r#"{"original": "data"}"#))
        .unwrap();

    // Apply patches (empty for now)
    let patches = vec![];
    let result = MockMatcher::apply_patches(patches, "test-mock", upstream, None);

    assert!(result.is_ok());
    let patched = result.unwrap();
    assert_eq!(patched.status(), StatusCode::OK);
    assert!(patched.headers().get("X-Mock-Id").is_some());
    assert_eq!(patched.headers().get("X-Mock-Id").unwrap(), "test-mock");
}

/// Test template rendering error handling
#[tokio::test]
async fn test_try_match_template_error() {
    let registry = MockRegistry::new();
    let mut mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );

    // Use template with invalid syntax
    mock.response
        .set_body(BodySource::template("{{ invalid syntax"));
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let action = matcher
        .try_match_parts(&Method::GET, "/test", None, &HeaderMap::new(), None)
        .await;
    assert!(action.is_some());

    // Should return error response
    match action.unwrap() {
        MockAction::FullMock(response) => {
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(response.headers().get("X-Mock-Error").unwrap(), "true");
            assert_eq!(response.headers().get("X-Mock-Id").unwrap(), "test");
        }
        MockAction::PatchUpstream { .. } => panic!("Expected FullMock error response"),
    }
}

/// Test non-template static response
#[tokio::test]
async fn test_try_match_static_response() {
    let registry = MockRegistry::new();
    let mut mock = create_test_mock(
        "test",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/test")],
        smallvec![],
        smallvec![],
        None,
    );

    mock.response
        .set_body(BodySource::inline(r#"{"static": "response"}"#));
    mock.response
        .headers
        .insert("X-Custom".to_string(), "static-value".to_string());
    registry.add_mock(mock);

    let matcher = MockMatcher::new(registry);

    let action = matcher
        .try_match_parts(&Method::GET, "/test", None, &HeaderMap::new(), None)
        .await;
    assert!(action.is_some());

    match action.unwrap() {
        MockAction::FullMock(response) => {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers().get("X-Custom").unwrap(), "static-value");
            assert_eq!(response.headers().get("X-Mock-Id").unwrap(), "test");
        }
        MockAction::PatchUpstream { .. } => panic!("Expected FullMock action"),
    }
}

/// Test registry accessor
#[test]
fn test_registry_accessor() {
    let registry = MockRegistry::new();
    let matcher = MockMatcher::new(registry.clone());

    // Verify registry accessor returns the correct registry
    assert!(
        matcher.registry().is_enabled(),
        "Registry should be enabled by default"
    );

    // Disable registry and verify accessor reflects the change
    registry.disable();
    assert!(
        !matcher.registry().is_enabled(),
        "Registry should be disabled after disable call"
    );

    // Re-enable and verify
    registry.enable();
    assert!(
        matcher.registry().is_enabled(),
        "Registry should be enabled after enable call"
    );
}
