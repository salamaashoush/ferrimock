//! Match-count endpoints for asserting which mocks served requests, and the
//! coverage and unmatched-request reports that summarise a whole run.

use super::MockApiState;
use crate::engine::Expected;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

/// One mock's match count.
#[derive(Debug, Serialize)]
pub struct MockMatchCount {
    /// Mock id.
    pub mock_id: String,
    /// Requests served since the last reset.
    pub count: u64,
}

/// Every mock that has served a request.
#[derive(Debug, Serialize)]
pub struct MatchCountsResponse {
    /// Per-mock counts, busiest first.
    pub counts: Vec<MockMatchCount>,
    /// Requests served across all mocks.
    pub total: u64,
}

/// The verdict of a verification.
#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    /// Whether the expectation held.
    pub verified: bool,
    /// Mock id asserted on.
    pub mock_id: String,
    /// Requests it actually served.
    pub actual: u64,
    /// Why the expectation failed, if it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Expectation for `GET /__ferrimock/calls/{id}?times=N`.
#[derive(Debug, Deserialize)]
pub struct VerifyQuery {
    /// Served exactly this many times.
    pub times: Option<u64>,
    /// Served at least this many times.
    pub at_least: Option<u64>,
    /// Served at most this many times.
    pub at_most: Option<u64>,
    /// Never served (any truthy value).
    pub never: Option<bool>,
}

impl VerifyQuery {
    fn expectation(&self) -> Option<Expected> {
        if self.never == Some(true) {
            return Some(Expected::Never);
        }
        self.times
            .map(Expected::Exactly)
            .or_else(|| self.at_least.map(Expected::AtLeast))
            .or_else(|| self.at_most.map(Expected::AtMost))
    }
}

/// Match counts for every mock that has served a request.
///
/// GET /`__ferrimock`/calls
pub async fn get_match_counts(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    let counts = registry
        .match_counts()
        .into_iter()
        .map(|(mock_id, count)| MockMatchCount { mock_id, count })
        .collect();

    Json(MatchCountsResponse {
        counts,
        total: registry.total_match_count(),
    })
}

/// One mock's count, or a verification when an expectation is supplied.
///
/// GET /`__ferrimock`/calls/{id}
/// GET /`__ferrimock`/calls/{id}?times=2
///
/// Answers 409 when the expectation fails, so a CI script can assert with a
/// bare status check.
pub async fn get_mock_match_count(
    State(app_state): State<MockApiState>,
    Path(mock_id): Path<String>,
    Query(query): Query<VerifyQuery>,
) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;

    let Some(expected) = query.expectation() else {
        return Json(MockMatchCount {
            mock_id: mock_id.clone(),
            count: registry.match_count(&mock_id),
        })
        .into_response();
    };

    match registry.verify(&mock_id, expected) {
        Ok(actual) => Json(VerifyResponse {
            verified: true,
            mock_id,
            actual,
            message: None,
        })
        .into_response(),
        Err(err) => (
            StatusCode::CONFLICT,
            Json(VerifyResponse {
                verified: false,
                mock_id,
                actual: err.actual,
                message: Some(err.to_string()),
            }),
        )
            .into_response(),
    }
}

/// Reset every match count.
///
/// DELETE /`__ferrimock`/calls
pub async fn reset_match_counts(State(app_state): State<MockApiState>) -> impl IntoResponse {
    app_state.mock.mock_registry.reset_match_counts();
    Json(MatchCountsResponse {
        counts: Vec::new(),
        total: 0,
    })
}

/// A coverage report plus the figures a reader would otherwise compute.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageResponse {
    /// Mocks loaded.
    pub total_mocks: usize,
    /// Share of them that served at least one request, 0-100.
    pub percent_covered: f64,
    /// The report itself.
    #[serde(flatten)]
    pub report: crate::engine::CoverageReport,
}

/// Which mocks ran and which never did.
///
/// GET /`__ferrimock`/coverage
pub async fn get_coverage(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let report = app_state.mock.mock_registry.coverage();
    Json(CoverageResponse {
        total_mocks: report.total_mocks(),
        percent_covered: report.percent_covered(),
        report,
    })
}

/// Every request that matched no mock.
///
/// GET /`__ferrimock`/unmatched
pub async fn get_unmatched(State(app_state): State<MockApiState>) -> impl IntoResponse {
    Json(app_state.mock.mock_registry.unmatched_requests())
}

/// Forget every unmatched request.
///
/// DELETE /`__ferrimock`/unmatched
pub async fn reset_unmatched(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    registry.reset_unmatched();
    Json(registry.unmatched_requests())
}

/// Edits that would let existing mocks serve the requests they missed.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionsResponse {
    /// Proposals, the one recovering the most requests first.
    pub suggestions: Vec<crate::engine::Suggestion>,
    /// Requests that no proposal covers, so they need a new mock rather than a
    /// widening.
    pub uncovered: Vec<String>,
}

/// What to widen so the corpus covers what it missed.
///
/// GET /`__ferrimock`/suggestions
pub async fn get_suggestions(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    let unmatched = registry.unmatched_requests();
    let suggestions = crate::engine::suggest(&app_state.mock.mock_matcher, &unmatched.requests);

    let covered: std::collections::HashSet<&str> = suggestions
        .iter()
        .flat_map(|s| s.covers.iter().map(String::as_str))
        .collect();
    let uncovered = unmatched
        .requests
        .iter()
        .map(|r| match r.query.as_deref() {
            Some(q) if !q.is_empty() => format!("{} {}?{}", r.method, r.path, q),
            _ => format!("{} {}", r.method, r.path),
        })
        .filter(|line| !covered.contains(line.as_str()))
        .collect();

    Json(SuggestionsResponse {
        suggestions,
        uncovered,
    })
}
