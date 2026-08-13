#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Match counting and `verify()` assertions.

use ferrimock::engine::matcher::MockMatcher;
use ferrimock::engine::registry::{Expected, MockRegistry};
use ferrimock::engine::types::{
    BodySource, MockDefinition, RequestMatcher, ResponseGenerator, UrlPattern,
};
use http::{HeaderMap, Method, StatusCode};
use smallvec::smallvec;

fn mock(id: &str, path: &str) -> MockDefinition {
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
            ..RequestMatcher::default()
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    }
}

fn hit(matcher: &MockMatcher, path: &str) {
    assert!(
        matcher
            .find_match(&Method::GET, path, None, &HeaderMap::new(), None)
            .is_some()
    );
}

#[test]
fn counts_matches_without_any_setup() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));
    let matcher = MockMatcher::new(registry.clone());

    assert_eq!(registry.match_count("get-user"), 0);
    hit(&matcher, "/api/users/1");
    hit(&matcher, "/api/users/1");

    assert_eq!(registry.match_count("get-user"), 2);
    assert_eq!(registry.total_match_count(), 2);
}

#[test]
fn count_keeps_rising_past_the_call_tracking_window() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));
    registry.enable_call_tracking("get-user", Some(5));
    let matcher = MockMatcher::new(registry.clone());

    for _ in 0..20 {
        hit(&matcher, "/api/users/1");
    }

    // Retained calls stop at the window; the match count does not.
    assert_eq!(registry.get_call_count("get-user"), 5);
    assert_eq!(registry.match_count("get-user"), 20);
}

#[test]
fn verify_accepts_and_rejects() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));
    let matcher = MockMatcher::new(registry.clone());

    hit(&matcher, "/api/users/1");
    hit(&matcher, "/api/users/1");
    hit(&matcher, "/api/users/1");

    assert_eq!(
        registry.verify("get-user", Expected::Exactly(3)).unwrap(),
        3
    );
    assert!(registry.verify("get-user", Expected::AtLeast(2)).is_ok());
    assert!(registry.verify("get-user", Expected::AtMost(3)).is_ok());
    assert!(registry.verify("get-user", Expected::Never).is_err());

    let err = registry
        .verify("get-user", Expected::Exactly(1))
        .unwrap_err();
    assert_eq!(err.actual, 3);
    assert!(err.known_mock);
    assert!(
        err.to_string()
            .contains("expected to serve exactly 1 request(s), served 3")
    );
}

#[test]
fn verify_flags_an_unknown_mock_id() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));

    let err = registry
        .verify("get-usr", Expected::Exactly(1))
        .unwrap_err();
    assert!(!err.known_mock);
    assert!(
        err.to_string()
            .contains("no mock is registered with that id")
    );

    // A registered mock that simply never ran does not get that hint.
    let err = registry
        .verify("get-user", Expected::Exactly(1))
        .unwrap_err();
    assert!(err.known_mock);
    assert!(!err.to_string().contains("no mock is registered"));
}

#[test]
fn never_holds_for_an_untouched_mock() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));
    registry.add_mock(mock("get-post", "/api/posts/1"));
    let matcher = MockMatcher::new(registry.clone());

    hit(&matcher, "/api/users/1");

    assert!(registry.verify("get-post", Expected::Never).is_ok());
    assert_eq!(registry.match_counts(), vec![("get-user".to_string(), 1)]);
}

#[test]
fn reset_clears_counts_for_the_next_test() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));
    let matcher = MockMatcher::new(registry.clone());

    hit(&matcher, "/api/users/1");
    registry.reset_match_counts();

    assert_eq!(registry.match_count("get-user"), 0);
    assert!(registry.verify("get-user", Expected::Never).is_ok());
}

#[test]
fn explaining_a_request_does_not_count_as_a_match() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("get-user", "/api/users/1"));
    let matcher = MockMatcher::new(registry.clone());

    let _ = matcher.explain(&Method::GET, "/api/users/1", None, &HeaderMap::new(), None);

    assert_eq!(registry.match_count("get-user"), 0);
}
