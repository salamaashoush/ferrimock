//! Runtime inspector for debugging mock matching

use super::MockApiState;
use super::types::{EvaluatedMock, InspectRequest, InspectResponse, MatchDetails, MatchedMock};
use crate::engine::diagnostics::{Criterion, MatchAttempt};
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use std::time::Instant;

/// Render the outcomes for one criterion kind as a single line.
///
/// `Header`/`Query` appear once per matcher, so their lines are joined; the
/// rest appear at most once. A criterion the mock does not declare reads
/// "not checked" rather than claiming a match.
fn detail(attempt: &MatchAttempt, want: &dyn Fn(&Criterion) -> bool) -> String {
    let lines: Vec<String> = attempt
        .outcomes
        .iter()
        .filter(|o| want(&o.criterion))
        .map(ToString::to_string)
        .collect();

    if lines.is_empty() {
        "not checked".to_string()
    } else {
        lines.join("; ")
    }
}

fn match_details(attempt: &MatchAttempt) -> MatchDetails {
    MatchDetails {
        method: detail(attempt, &|c| matches!(c, Criterion::Method)),
        url: detail(attempt, &|c| matches!(c, Criterion::Url)),
        headers: detail(attempt, &|c| matches!(c, Criterion::Header(_))),
        query: detail(attempt, &|c| matches!(c, Criterion::Query(_))),
        body: detail(attempt, &|c| {
            matches!(c, Criterion::Body | Criterion::GraphQl)
        }),
        failed: attempt.failures().map(ToString::to_string).collect(),
    }
}

/// Share of declared criteria the request satisfied, as a percentage.
fn score(attempt: &MatchAttempt) -> u32 {
    let total = attempt.outcomes.len();
    if total == 0 {
        return 100;
    }
    let passed = u32::try_from(attempt.passed_count()).unwrap_or(u32::MAX);
    let total = u32::try_from(total).unwrap_or(1).max(1);
    passed.saturating_mul(100) / total
}

/// Inspect how a request would match against mocks
///
/// POST /__ferrimock/inspect
///
/// Read-only: this reports what a real request *would* do without consuming a
/// `once` mock, recording a call, or warming the match cache.
pub async fn inspect_request(
    State(app_state): State<MockApiState>,
    Json(request): Json<InspectRequest>,
) -> impl IntoResponse {
    let start = Instant::now();

    // Parse method
    let method = match request.method.parse::<axum::http::Method>() {
        Ok(m) => m,
        Err(e) => {
            let mut error_response = serde_json::Map::new();
            error_response.insert(
                "error".to_string(),
                serde_json::Value::String(format!("Invalid HTTP method: {e}")),
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::Value::Object(error_response)),
            )
                .into_response();
        }
    };

    // Convert headers to HeaderMap
    let mut headers = axum::http::HeaderMap::new();
    if let Some(req_headers) = &request.headers {
        for (key, value) in req_headers {
            if let (Ok(name), Ok(val)) = (
                axum::http::HeaderName::from_bytes(key.as_bytes()),
                axum::http::HeaderValue::from_str(value),
            ) {
                headers.insert(name, val);
            }
        }
    }

    // Get body bytes
    let body = request.body.as_ref().map(String::as_bytes);

    let report = app_state.mock.mock_matcher.explain(
        &method,
        &request.path,
        request.query.as_deref(),
        &headers,
        body,
    );

    let execution_time_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);

    let matched = report.matched().map(|attempt| MatchedMock {
        id: attempt.mock_id.as_str().into(),
        priority: attempt.priority,
        score: score(attempt),
        captures: attempt.captures.clone(),
    });

    // Near-miss order, so the most useful candidates read first.
    let mut ranked: Vec<&MatchAttempt> = report.near_misses(usize::MAX);
    if let Some(winner) = report.matched() {
        ranked.insert(0, winner);
    }

    let evaluated = ranked
        .into_iter()
        .map(|attempt| EvaluatedMock {
            id: attempt.mock_id.as_str().into(),
            priority: attempt.priority,
            matched: attempt.matched(),
            enabled: attempt.enabled,
            reason: if attempt.matched() {
                None
            } else {
                let mut reasons: Vec<String> = Vec::new();
                if !attempt.enabled {
                    reasons.push("mock is disabled".to_string());
                }
                reasons.extend(attempt.failures().map(ToString::to_string));
                Some(reasons.join("; "))
            },
            match_details: Some(match_details(attempt)),
        })
        .collect();

    Json(InspectResponse {
        matched,
        evaluated,
        execution_time_us,
        cache_hit: false,
    })
    .into_response()
}
