#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Coverage over the loaded mocks, and the log of requests that matched none.

use ferrimock::engine::matcher::MockMatcher;
use ferrimock::engine::registry::MockRegistry;
use ferrimock::engine::types::{
    BodySource, MockDefinition, RequestMatcher, ResponseGenerator, UrlPattern,
};
use http::{HeaderMap, Method, StatusCode};
use smallvec::smallvec;

fn mock(id: &str, path: &str, source_file: Option<&str>) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: source_file.map(ToString::to_string),
        scope: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::Exact(path.into())],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    }
}

fn get(matcher: &MockMatcher, path: &str, query: Option<&str>) -> bool {
    matcher
        .find_match(&Method::GET, path, query, &HeaderMap::new(), None)
        .is_some()
}

#[test]
fn coverage_splits_served_from_unused_and_names_the_source_file() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("used", "/api/used", Some("mocks/a.yaml")));
    registry.add_mock(mock("never", "/api/never", Some("mocks/b.yaml")));
    let matcher = MockMatcher::new(registry.clone());

    assert!(get(&matcher, "/api/used", None));
    assert!(get(&matcher, "/api/used", None));

    let report = registry.coverage();
    assert_eq!(report.total_mocks(), 2);
    assert_eq!(report.total_matches, 2);

    assert_eq!(report.served.len(), 1);
    assert_eq!(report.served[0].mock_id, "used");
    assert_eq!(report.served[0].count, 2);

    assert_eq!(report.unused.len(), 1);
    assert_eq!(report.unused[0].mock_id, "never");
    assert_eq!(report.unused[0].count, 0);
    assert_eq!(
        report.unused[0].source_file.as_deref(),
        Some("mocks/b.yaml"),
        "an unused entry must name the file to fix"
    );

    assert!((report.percent_covered() - 50.0).abs() < f64::EPSILON);
}

#[test]
fn coverage_of_an_empty_registry_is_zero_not_a_vacuous_hundred() {
    let registry = MockRegistry::new();
    let report = registry.coverage();

    assert_eq!(report.total_mocks(), 0);
    assert!((report.percent_covered() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn coverage_sorts_served_by_count_and_excludes_removed_mocks() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("quiet", "/api/quiet", None));
    registry.add_mock(mock("busy", "/api/busy", None));
    registry.add_mock(mock("gone", "/api/gone", None));
    let matcher = MockMatcher::new(registry.clone());

    assert!(get(&matcher, "/api/quiet", None));
    for _ in 0..3 {
        assert!(get(&matcher, "/api/busy", None));
    }
    assert!(get(&matcher, "/api/gone", None));

    registry.remove_mock("gone");

    let report = registry.coverage();
    let ids: Vec<&str> = report.served.iter().map(|m| m.mock_id.as_str()).collect();
    assert_eq!(ids, ["busy", "quiet"], "busiest first");
    assert_eq!(
        report.total_matches, 4,
        "the removed mock's count must not inflate the total it is no longer part of"
    );
}

#[test]
fn a_consumed_once_mock_reads_as_served_but_disabled() {
    let registry = MockRegistry::new();
    let mut once = mock("once", "/api/once", None);
    once.once = true;
    registry.add_mock(once);
    let matcher = MockMatcher::new(registry.clone());

    assert!(get(&matcher, "/api/once", None));

    let report = registry.coverage();
    assert_eq!(report.served.len(), 1);
    assert_eq!(report.served[0].count, 1);
    assert!(
        !report.served[0].enabled,
        "a used-up once mock must not read as an unused one"
    );
}

#[test]
fn unmatched_requests_are_off_until_tracking_is_enabled() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("only", "/api/only", None));
    let matcher = MockMatcher::new(registry.clone());

    assert!(!get(&matcher, "/api/missing", None));
    assert_eq!(registry.unmatched_requests().total, 0);

    matcher.set_track_unmatched(true);
    assert!(!get(&matcher, "/api/missing", None));
    assert_eq!(registry.unmatched_requests().total, 1);
}

#[test]
fn repeats_of_a_request_line_fold_into_one_entry() {
    let registry = MockRegistry::new();
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    for _ in 0..5 {
        assert!(!get(&matcher, "/api/poll", None));
    }
    assert!(!get(&matcher, "/api/other", None));

    let report = registry.unmatched_requests();
    assert_eq!(report.total, 6);
    assert_eq!(report.requests.len(), 2);
    assert_eq!(report.requests[0].path, "/api/poll");
    assert_eq!(report.requests[0].count, 5, "most frequent first");
    assert_eq!(report.requests[1].count, 1);
    assert!(report.requests[0].last_seen >= report.requests[0].first_seen);
}

#[test]
fn the_query_string_separates_request_lines() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("items", "/folder/0/items", None));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    assert!(!get(&matcher, "/folder/0", Some("format=minimal")));
    assert!(!get(&matcher, "/folder/0", Some("format=full")));

    let report = registry.unmatched_requests();
    assert_eq!(
        report.requests.len(),
        2,
        "the query is the difference between a hit and a miss, so it must not be folded away"
    );
    let queries: Vec<&str> = report
        .requests
        .iter()
        .filter_map(|r| r.query.as_deref())
        .collect();
    assert!(queries.contains(&"format=minimal"));
    assert!(queries.contains(&"format=full"));
}

#[test]
fn a_disabled_registry_records_no_misses() {
    let registry = MockRegistry::new();
    registry.add_mock(mock("only", "/api/only", None));
    registry.disable();
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    assert!(!get(&matcher, "/api/anything", None));
    assert_eq!(
        registry.unmatched_requests().total,
        0,
        "mocking being switched off is not a mock corpus gap"
    );
}

#[test]
fn resetting_clears_the_log_and_its_totals() {
    let registry = MockRegistry::new();
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    assert!(!get(&matcher, "/api/missing", None));
    registry.reset_unmatched();

    let report = registry.unmatched_requests();
    assert_eq!(report.total, 0);
    assert_eq!(report.dropped, 0);
    assert!(report.requests.is_empty());
}
