#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Match explanations and near-miss ranking.

use ferrimock::engine::diagnostics::Criterion;
use ferrimock::engine::matcher::MockMatcher;
use ferrimock::engine::registry::MockRegistry;
use ferrimock::engine::types::{
    BodyMatcher, BodySource, HeaderMatcher, MockDefinition, QueryMatcher, RequestMatcher,
    ResponseGenerator, UrlPattern,
};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use smallvec::smallvec;

fn mock(
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
        once: false,
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
        streaming: None,
    }
}

fn simple(id: &str, priority: u32, method: Method, path: &str) -> MockDefinition {
    mock(
        id,
        priority,
        smallvec![method],
        smallvec![UrlPattern::exact(path)],
        smallvec![],
        smallvec![],
        None,
    )
}

fn auth_headers(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_str(value).unwrap(),
    );
    headers
}

#[test]
fn explains_a_match() {
    let registry = MockRegistry::new();
    registry.add_mock(simple("get-user", 100, Method::GET, "/api/users/1"));
    let matcher = MockMatcher::new(registry);

    let report = matcher.explain(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None);

    let matched = report.matched().expect("should match");
    assert_eq!(matched.mock_id, "get-user");
    assert!(report.near_misses(5).is_empty());
    assert!(report.summary().contains("matched mock get-user"));
}

#[test]
fn names_the_criterion_that_rejected_the_request() {
    let registry = MockRegistry::new();
    registry.add_mock(mock(
        "get-user",
        100,
        smallvec![Method::GET],
        smallvec![UrlPattern::exact("/api/users/1")],
        smallvec![HeaderMatcher::exact(
            HeaderName::from_static("authorization"),
            "Bearer good",
        )],
        smallvec![],
        None,
    ));
    let matcher = MockMatcher::new(registry);

    let report = matcher.explain(
        &Method::GET,
        "/api/users/1",
        None,
        &auth_headers("Bearer bad"),
        None,
    );

    assert!(report.matched().is_none());
    let closest = report.near_misses(1)[0];
    assert_eq!(closest.mock_id, "get-user");

    let failures: Vec<_> = closest.failures().collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].criterion,
        Criterion::Header("authorization".to_string())
    );
    assert!(failures[0].expected.contains("Bearer good"));
    assert!(failures[0].actual.contains("Bearer bad"));

    let summary = report.summary();
    assert!(summary.contains("GET /api/users/1"));
    assert!(summary.contains("header authorization"));
}

#[test]
fn ranks_the_closest_mock_first() {
    let registry = MockRegistry::new();
    // Wrong path and wrong method: two failures.
    registry.add_mock(simple("far", 100, Method::POST, "/api/other"));
    // Right path, wrong method: one failure.
    registry.add_mock(simple("close", 100, Method::POST, "/api/users/1"));
    let matcher = MockMatcher::new(registry);

    let report = matcher.explain(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None);

    let ranked = report.near_misses(2);
    assert_eq!(ranked[0].mock_id, "close");
    assert_eq!(ranked[1].mock_id, "far");
    assert_eq!(ranked[0].failures().count(), 1);
    assert_eq!(ranked[1].failures().count(), 2);
}

#[test]
fn reports_a_consumed_once_mock_as_disabled() {
    let registry = MockRegistry::new();
    let mut once_mock = simple("get-user", 100, Method::GET, "/api/users/1");
    once_mock.once = true;
    registry.add_mock(once_mock);
    let matcher = MockMatcher::new(registry);

    assert!(
        matcher
            .find_match(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None)
            .is_some()
    );

    let report = matcher.explain(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None);
    assert!(report.matched().is_none());

    let closest = report.near_misses(1)[0];
    assert_eq!(closest.mock_id, "get-user");
    assert!(!closest.enabled);
    // Every criterion still passes — only the disabled state stands in the way.
    assert_eq!(closest.failures().count(), 0);
    assert!(closest.to_string().contains("disabled"));
}

#[test]
fn explaining_does_not_consume_a_once_mock() {
    let registry = MockRegistry::new();
    let mut once_mock = simple("get-user", 100, Method::GET, "/api/users/1");
    once_mock.once = true;
    registry.add_mock(once_mock);
    let matcher = MockMatcher::new(registry);

    let _ = matcher.explain(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None);

    assert!(
        matcher
            .find_match(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None)
            .is_some(),
        "explain() must not disable the mock"
    );
}

#[test]
fn reports_query_and_body_criteria() {
    let registry = MockRegistry::new();
    registry.add_mock(mock(
        "search",
        100,
        smallvec![Method::POST],
        smallvec![UrlPattern::exact("/api/search")],
        smallvec![],
        smallvec![QueryMatcher::exact("page", "2")],
        Some(BodyMatcher::contains("needle")),
    ));
    let matcher = MockMatcher::new(registry);

    let report = matcher.explain(
        &Method::POST,
        "/api/search",
        Some("page=1"),
        &HeaderMap::new(),
        Some(b"haystack"),
    );

    let attempt = &report.attempts[0];
    let failed: Vec<&Criterion> = attempt.failures().map(|o| &o.criterion).collect();
    assert!(failed.contains(&&Criterion::Query("page".to_string())));
    assert!(failed.contains(&&Criterion::Body));
    assert_eq!(attempt.passed_count(), 2); // method + url
    assert!(report.summary().contains("page=1"));
}

#[test]
fn empty_registry_says_so() {
    let matcher = MockMatcher::new(MockRegistry::new());
    let report = matcher.explain(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None);

    assert!(report.matched().is_none());
    assert!(report.attempts.is_empty());
    assert!(report.summary().contains("registry is empty"));
}
