//! Response generation tests
//!
//! Tests for response priority ordering

use http::{HeaderMap, Method, StatusCode};
use mockpit_engine::{
    BodySource, MockDefinition, MockMatcher, MockRegistry, RequestMatcher, ResponseGenerator,
    UrlPattern,
};
use smallvec::smallvec;

#[test]
fn test_mock_priority_ordering() {
    let registry = MockRegistry::new();

    // Add two mocks that match the same URL, different priorities
    let low_priority = MockDefinition {
        id: "low".into(),
        priority: 10,
        enabled: true,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority":"low"}"#),
        ),
        vars: None,
    };

    let high_priority = MockDefinition {
        id: "high".into(),
        priority: 100,
        enabled: true,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority":"high"}"#),
        ),
        vars: None,
    };

    registry.add_mock(low_priority);
    registry.add_mock(high_priority);

    let matcher = MockMatcher::new(registry);
    let headers = HeaderMap::new();

    let result = matcher
        .find_match(&Method::GET, "/api/test", None, &headers, None)
        .unwrap();

    // Should match the high priority mock
    assert_eq!(result.mock.id, "high");
}
