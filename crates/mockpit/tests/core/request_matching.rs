//! Request matching tests
//!
//! Tests for URL patterns, HTTP methods, headers, query params, and body matching

use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use mockpit::engine::{
    BodyMatcher, BodySource, HeaderMatcher, MockDefinition, MockMatcher, MockRegistry,
    QueryMatcher, RequestMatcher, ResponseGenerator, UrlPattern,
};
use smallvec::smallvec;

#[test]
fn test_end_to_end_mock_matching() {
    // Create a mock registry
    let registry = MockRegistry::new();

    // Add a simple mock
    let mock = MockDefinition {
        id: "get-user".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/2.0/users/me")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"type":"user","id":"123","name":"Test User"}"#),
        ),
        vars: None,
    };

    registry.add_mock(mock);

    // Create matcher
    let matcher = MockMatcher::new(registry);

    // Test matching
    let headers = HeaderMap::new();
    let result = matcher.find_match(&Method::GET, "/api/2.0/users/me", None, &headers, None);

    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "get-user");
    assert_eq!(matched.mock.response.status, StatusCode::OK);
}

#[test]
fn test_mock_with_regex_pattern() {
    let registry = MockRegistry::new();

    // Add a mock with regex pattern
    let mock = MockDefinition {
        id: "get-file-by-id".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::regex(r"^/api/2\.0/files/\d+$").unwrap()],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"type":"file"}"#)),
        vars: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Test matching
    let headers = HeaderMap::new();

    // Should match
    assert!(
        matcher
            .find_match(&Method::GET, "/api/2.0/files/123", None, &headers, None)
            .is_some()
    );
    assert!(
        matcher
            .find_match(&Method::GET, "/api/2.0/files/456789", None, &headers, None)
            .is_some()
    );

    // Should not match
    assert!(
        matcher
            .find_match(&Method::GET, "/api/2.0/files/abc", None, &headers, None)
            .is_none()
    );
    assert!(
        matcher
            .find_match(
                &Method::GET,
                "/api/2.0/files/123/content",
                None,
                &headers,
                None
            )
            .is_none()
    );
}

#[test]
fn test_mock_with_header_matching() {
    let registry = MockRegistry::new();

    // Add a mock that requires authorization header
    let mock = MockDefinition {
        id: "authenticated-request".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/2.0/protected")],
            header_matchers: smallvec![HeaderMatcher::present(HeaderName::from_static(
                "authorization"
            ))],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"authorized":true}"#),
        ),
        vars: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Test with header
    let mut headers_with_auth = HeaderMap::new();
    headers_with_auth.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer token123"),
    );
    assert!(
        matcher
            .find_match(
                &Method::GET,
                "/api/2.0/protected",
                None,
                &headers_with_auth,
                None
            )
            .is_some()
    );

    // Test without header
    let headers_no_auth = HeaderMap::new();
    assert!(
        matcher
            .find_match(
                &Method::GET,
                "/api/2.0/protected",
                None,
                &headers_no_auth,
                None
            )
            .is_none()
    );
}

#[test]
fn test_body_matcher_contains() {
    use mockpit::engine::BodyMatcher;

    let matcher = BodyMatcher::contains("test");
    assert!(matcher.matches(b"this is a test", None));
    assert!(matcher.matches(b"test", None));
    assert!(!matcher.matches(b"no match here", None));
}

#[test]
fn test_body_matcher_regex() {
    use mockpit::engine::BodyMatcher;

    let matcher = BodyMatcher::regex(r"\d{3}-\d{3}-\d{4}").unwrap();
    assert!(matcher.matches(b"Phone: 123-456-7890", None));
    assert!(!matcher.matches(b"Phone: 123-45-6789", None));
}

#[test]
fn test_body_matcher_json_path() {
    use mockpit::engine::BodyMatcher;

    let matcher = BodyMatcher::json_path("user.name", serde_json::json!("John"));

    let json_body = r#"{"user": {"name": "John", "age": 30}}"#;
    assert!(matcher.matches(json_body.as_bytes(), None));

    let json_body_no_match = r#"{"user": {"name": "Jane", "age": 30}}"#;
    assert!(!matcher.matches(json_body_no_match.as_bytes(), None));
}

#[test]
fn test_body_matcher_json_equals() {
    use mockpit::engine::BodyMatcher;

    let expected = serde_json::json!({"type": "user", "id": "123"});
    let matcher = BodyMatcher::json_equals(expected);

    assert!(matcher.matches(br#"{"type": "user", "id": "123"}"#, None));
    assert!(!matcher.matches(br#"{"type": "user", "id": "456"}"#, None));
}

// Note: These tests have been removed as the Condition type is obsolete.
// The new system uses QueryMatcher for query parameters and BodyMatcher for body matching.
// See test_mock_with_conditions() for the updated approach using QueryMatcher.

#[test]
fn test_mock_with_body_matcher() {
    use mockpit::engine::BodyMatcher;

    let registry = MockRegistry::new();

    // Add a mock that matches based on body content
    let mock = MockDefinition {
        id: "body-match".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/api/2.0/data")],
            header_matchers: smallvec![],
            body_matcher: Some(BodyMatcher::contains("special_field")),
            query_matchers: smallvec![],
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"matched": true}"#),
        ),
        vars: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Test with matching body
    assert!(
        matcher
            .find_match(
                &Method::POST,
                "/api/2.0/data",
                None,
                &HeaderMap::new(),
                Some(br#"{"special_field": "value"}"#)
            )
            .is_some()
    );

    // Test with non-matching body
    assert!(
        matcher
            .find_match(
                &Method::POST,
                "/api/2.0/data",
                None,
                &HeaderMap::new(),
                Some(br#"{"other_field": "value"}"#)
            )
            .is_none()
    );
}

#[test]
fn test_mock_with_conditions() {
    let registry = MockRegistry::new();

    // Add a mock with query parameter condition
    let mock = MockDefinition {
        id: "conditional-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/2.0/search")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![QueryMatcher::exact("filter", "active")],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"filtered": true}"#),
        ),
        vars: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Test with matching query parameter
    assert!(
        matcher
            .find_match(
                &Method::GET,
                "/api/2.0/search",
                Some("filter=active&limit=10"),
                &HeaderMap::new(),
                None
            )
            .is_some()
    );

    // Test without matching query parameter
    assert!(
        matcher
            .find_match(
                &Method::GET,
                "/api/2.0/search",
                Some("filter=inactive&limit=10"),
                &HeaderMap::new(),
                None
            )
            .is_none()
    );
}

#[test]
fn test_glob_pattern_matching() {
    let registry = MockRegistry::new();

    // Add a mock with glob pattern
    let mock = MockDefinition {
        id: "glob-match".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::glob("/api/*/files/*").unwrap()],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"matched": "glob"}"#),
        ),
        vars: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Test matching
    let headers = HeaderMap::new();

    // Should match
    assert!(
        matcher
            .find_match(&Method::GET, "/api/v1/files/123", None, &headers, None)
            .is_some()
    );
    assert!(
        matcher
            .find_match(
                &Method::GET,
                "/api/v2/files/document.pdf",
                None,
                &headers,
                None
            )
            .is_some()
    );

    // Should not match
    assert!(
        matcher
            .find_match(&Method::GET, "/api/files/123", None, &headers, None)
            .is_none()
    );
}

#[test]
fn test_complex_conditional_matching() {
    let registry = MockRegistry::new();

    // High priority admin mock with multiple matchers
    // Note: In the new system, we use body_matcher for one JSON path check and query_matchers for query params
    // For multiple JSONPath checks, we would need separate mocks with different priorities
    let admin_mock = MockDefinition {
        id: "admin-endpoint".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/api/2.0/admin/action")],
            header_matchers: smallvec![HeaderMatcher::present(HeaderName::from_static(
                "authorization"
            ))],
            body_matcher: Some(BodyMatcher::json_path(
                "user.role",
                serde_json::json!("admin"),
            )),
            query_matchers: smallvec![QueryMatcher::exact("confirmed", "true")],
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"admin_action": "executed"}"#),
        ),
        vars: None,
    };

    registry.add_mock(admin_mock);

    let matcher = MockMatcher::new(registry);

    // Prepare request that matches all criteria
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer admin-token"),
    );

    let body = br#"{"user": {"role": "admin"}, "action": {"type": "critical"}}"#;

    // Should match when all criteria are met
    let result = matcher.find_match(
        &Method::POST,
        "/api/2.0/admin/action",
        Some("confirmed=true"),
        &headers,
        Some(body),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().mock.id, "admin-endpoint");

    // Should not match without confirmation
    let result_no_confirm = matcher.find_match(
        &Method::POST,
        "/api/2.0/admin/action",
        Some("confirmed=false"),
        &headers,
        Some(body),
    );
    assert!(result_no_confirm.is_none());

    // Should not match with wrong role
    let wrong_role_body = br#"{"user": {"role": "user"}, "action": {"type": "critical"}}"#;
    let result_wrong_role = matcher.find_match(
        &Method::POST,
        "/api/2.0/admin/action",
        Some("confirmed=true"),
        &headers,
        Some(wrong_role_body),
    );
    assert!(result_wrong_role.is_none());
}

#[test]
fn test_complex_priority_matching() {
    let registry = MockRegistry::new();

    // Add mocks in random order to test sorting
    registry.add_mock(MockDefinition {
        id: "low-priority".into(),
        priority: 10,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority": "low"}"#),
        ),
        vars: None,
    });

    registry.add_mock(MockDefinition {
        id: "medium-priority".into(),
        priority: 50,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority": "medium"}"#),
        ),
        vars: None,
    });

    registry.add_mock(MockDefinition {
        id: "high-priority".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority": "high"}"#),
        ),
        vars: None,
    });

    let matcher = MockMatcher::new(registry);
    let headers = HeaderMap::new();

    // Should always match highest priority
    let result = matcher
        .find_match(&Method::GET, "/api/test", None, &headers, None)
        .unwrap();
    assert_eq!(result.mock.id, "high-priority");
}
