#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Which widening would let a recorded corpus serve the requests it missed.

use ferrimock::engine::matcher::MockMatcher;
use ferrimock::engine::registry::MockRegistry;
use ferrimock::engine::suggestions::{SuggestionKind, suggest};
use ferrimock::engine::types::{
    BodySource, HeaderMatcher, MockDefinition, QueryMatcher, RequestMatcher, ResponseGenerator,
    UrlPattern,
};
use http::{HeaderMap, Method, StatusCode};
use smallvec::{SmallVec, smallvec};

fn mock(
    id: &str,
    method: Method,
    url: UrlPattern,
    query_matchers: SmallVec<[QueryMatcher; 2]>,
    header_matchers: SmallVec<[HeaderMatcher; 2]>,
) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: Some(format!("mocks/{id}.yaml")),
        scope: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![method],
            url_patterns: smallvec![url],
            header_matchers,
            query_matchers,
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    }
}

fn exact(id: &str, path: &str) -> MockDefinition {
    mock(
        id,
        Method::GET,
        UrlPattern::Exact(path.to_string()),
        smallvec![],
        smallvec![],
    )
}

/// Drive real misses through the matcher so the unmatched log is populated the
/// way a replay run would populate it.
fn miss(matcher: &MockMatcher, path: &str, query: Option<&str>, times: usize) {
    for _ in 0..times {
        assert!(
            matcher
                .find_match(&Method::GET, path, query, &HeaderMap::new(), None)
                .is_none(),
            "{path} was expected to miss"
        );
    }
}

#[test]
fn a_recorded_id_becomes_a_parameter() {
    let registry = MockRegistry::new();
    registry.add_mock(exact("get-user", "/api/users/123"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/users/456", None, 3);

    let suggestions = suggest(&matcher, &registry.unmatched_requests().requests);
    assert_eq!(suggestions.len(), 1, "{suggestions:#?}");

    let s = &suggestions[0];
    assert_eq!(s.mock_id, "get-user");
    assert_eq!(s.kind, SuggestionKind::ParameterizeUrl);
    assert_eq!(s.current, "/api/users/123");
    assert_eq!(s.proposed, "/api/users/:id");
    assert_eq!(
        s.request_count, 3,
        "repeats count toward what the edit buys"
    );
    assert_eq!(
        s.source_file.as_deref(),
        Some("mocks/get-user.yaml"),
        "a suggestion must name the file to edit"
    );
}

#[test]
fn a_pinned_query_value_is_proposed_for_relaxing() {
    let registry = MockRegistry::new();
    registry.add_mock(mock(
        "folder-items",
        Method::GET,
        UrlPattern::Exact("/folder/0/items".to_string()),
        smallvec![QueryMatcher::exact("format", "full")],
        smallvec![],
    ));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/folder/0/items", Some("format=minimal"), 1);

    let suggestions = suggest(&matcher, &registry.unmatched_requests().requests);
    assert_eq!(suggestions.len(), 1, "{suggestions:#?}");
    assert_eq!(suggestions[0].kind, SuggestionKind::RelaxQuery);
    assert_eq!(suggestions[0].current, "format=full");
    assert_eq!(suggestions[0].proposed, "format=<any>");
}

#[test]
fn a_genuinely_different_endpoint_gets_no_suggestion() {
    // The corpus holds /folder/0/items and the app asks for /folder/0. That
    // needs a new mock; proposing a widening would send the reader to edit a
    // file that cannot be made to serve it.
    let registry = MockRegistry::new();
    registry.add_mock(exact("folder-items", "/folder/0/items"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/folder/0", None, 1);

    assert!(
        suggest(&matcher, &registry.unmatched_requests().requests).is_empty(),
        "a missing endpoint is not a widening"
    );
}

#[test]
fn a_path_with_nothing_variable_gets_no_suggestion() {
    let registry = MockRegistry::new();
    registry.add_mock(exact("health", "/api/health"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/status", None, 1);

    assert!(
        suggest(&matcher, &registry.unmatched_requests().requests).is_empty(),
        "two static paths are different endpoints, not one parameterised one"
    );
}

#[test]
fn a_request_rejected_on_two_criteria_is_not_proposed() {
    // Widening only the url would still leave the query rejecting it, so the
    // proposal would be a lie. Two edits are needed and neither is safe alone.
    let registry = MockRegistry::new();
    registry.add_mock(mock(
        "get-user",
        Method::GET,
        UrlPattern::Exact("/api/users/123".to_string()),
        smallvec![QueryMatcher::exact("format", "full")],
        smallvec![],
    ));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/users/456", Some("format=minimal"), 1);

    assert!(
        suggest(&matcher, &registry.unmatched_requests().requests).is_empty(),
        "a half-fix must not be reported as a fix"
    );
}

#[test]
fn a_method_mismatch_is_never_proposed() {
    // Serving POST from a mock recorded for GET changes what the mock means.
    let registry = MockRegistry::new();
    registry.add_mock(exact("get-user", "/api/users/123"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    assert!(
        matcher
            .find_match(
                &Method::POST,
                "/api/users/123",
                None,
                &HeaderMap::new(),
                None
            )
            .is_none()
    );

    assert!(suggest(&matcher, &registry.unmatched_requests().requests).is_empty());
}

#[test]
fn a_header_matcher_is_never_proposed_against() {
    // The unmatched log keeps no headers, so a header rejection cannot be
    // re-evaluated here and must not be guessed at.
    let registry = MockRegistry::new();
    registry.add_mock(mock(
        "secure",
        Method::GET,
        UrlPattern::Exact("/api/secure/123".to_string()),
        smallvec![],
        smallvec![HeaderMatcher::exact(
            http::header::AUTHORIZATION,
            "Bearer x"
        )],
    ));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/secure/456", None, 1);

    assert!(suggest(&matcher, &registry.unmatched_requests().requests).is_empty());
}

#[test]
fn a_disabled_mock_is_not_proposed_for_widening() {
    let registry = MockRegistry::new();
    let mut disabled = exact("get-user", "/api/users/123");
    disabled.enabled = false;
    registry.add_mock(disabled);
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/users/456", None, 1);

    assert!(
        suggest(&matcher, &registry.unmatched_requests().requests).is_empty(),
        "re-enabling is the fix, and coverage already reports it as disabled"
    );
}

#[test]
fn one_widening_absorbing_many_ids_is_reported_once() {
    let registry = MockRegistry::new();
    registry.add_mock(exact("get-user", "/api/users/1"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/users/2", None, 1);
    miss(&matcher, "/api/users/3", None, 1);
    miss(&matcher, "/api/users/4", None, 1);

    let suggestions = suggest(&matcher, &registry.unmatched_requests().requests);
    assert_eq!(suggestions.len(), 1, "one edit, not one per missed id");
    assert_eq!(suggestions[0].covers.len(), 3);
    assert_eq!(suggestions[0].request_count, 3);
}

#[test]
fn suggestions_lead_with_the_edit_that_buys_the_most() {
    let registry = MockRegistry::new();
    registry.add_mock(exact("busy", "/api/busy/1"));
    registry.add_mock(exact("quiet", "/api/quiet/1"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/busy/2", None, 10);
    miss(&matcher, "/api/quiet/2", None, 1);

    let suggestions = suggest(&matcher, &registry.unmatched_requests().requests);
    assert_eq!(suggestions.len(), 2);
    assert_eq!(suggestions[0].mock_id, "busy");
    assert_eq!(suggestions[0].request_count, 10);
    assert_eq!(suggestions[1].mock_id, "quiet");
}

#[test]
fn a_uuid_path_parameterises_too() {
    let registry = MockRegistry::new();
    registry.add_mock(exact(
        "get-file",
        "/files/550e8400-e29b-41d4-a716-446655440000",
    ));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(
        &matcher,
        "/files/6ba7b810-9dad-11d1-80b4-00c04fd430c8",
        None,
        1,
    );

    let suggestions = suggest(&matcher, &registry.unmatched_requests().requests);
    assert_eq!(suggestions.len(), 1, "{suggestions:#?}");
    assert_eq!(suggestions[0].proposed, "/files/:uuid");
}

#[test]
fn a_widened_mock_actually_serves_what_was_suggested() {
    // The proposal is only worth printing if applying it works, so apply it.
    let registry = MockRegistry::new();
    registry.add_mock(exact("get-user", "/api/users/123"));
    let matcher = MockMatcher::new(registry.clone());
    matcher.set_track_unmatched(true);

    miss(&matcher, "/api/users/456", None, 1);
    let suggestions = suggest(&matcher, &registry.unmatched_requests().requests);
    let proposed = suggestions[0].proposed.clone();

    // Apply it the way editing the mock file would: the proposed string goes
    // through the config parser, which detects the Express style.
    let pattern = ferrimock::config::parse_url_pattern(&proposed)
        .expect("a proposal must be a pattern the config layer accepts");
    registry.remove_mock("get-user");
    registry.add_mock(mock(
        "get-user",
        Method::GET,
        pattern,
        smallvec![],
        smallvec![],
    ));
    let widened = MockMatcher::new(registry);

    assert!(
        widened
            .find_match(
                &Method::GET,
                "/api/users/456",
                None,
                &HeaderMap::new(),
                None
            )
            .is_some(),
        "applying {proposed} must actually serve the request it was proposed for"
    );
    assert!(
        widened
            .find_match(
                &Method::GET,
                "/api/users/123",
                None,
                &HeaderMap::new(),
                None
            )
            .is_some(),
        "and must not stop serving what it already served"
    );
}
