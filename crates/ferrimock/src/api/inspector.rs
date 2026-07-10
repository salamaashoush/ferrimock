//! Runtime inspector for debugging mock matching

use super::MockApiState;
use super::types::{EvaluatedMock, InspectRequest, InspectResponse, MatchDetails, MatchedMock};
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use std::time::Instant;

/// Inspect how a request would match against mocks
///
/// POST /__ferrimock/inspect
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

    // Use find_match from the matcher
    let mock_match = app_state.mock.mock_matcher.find_match(
        &method,
        &request.path,
        request.query.as_deref(),
        &headers,
        body,
    );

    let execution_time_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);

    let (matched, evaluated) = if let Some(matched_mock) = mock_match {
        let matched_info = MatchedMock {
            id: matched_mock.mock.id.clone(),
            priority: matched_mock.mock.priority,
            score: 100, // MockMatch doesn't expose score, using placeholder
            captures: matched_mock.captures,
        };

        let evaluated_info = vec![EvaluatedMock {
            id: matched_mock.mock.id.clone(),
            priority: matched_mock.mock.priority,
            matched: true,
            reason: None,
            match_details: Some(MatchDetails {
                method: format!("Matched: {}", request.method),
                url: format!("Matched: {}", request.path),
                headers: format!("Matched: {} headers", headers.len()),
                query: request
                    .query
                    .as_ref()
                    .map_or("N/A".to_string(), |q| format!("Matched: {q}")),
                body: body.map_or("N/A".to_string(), |_| "Matched: body".to_string()),
            }),
        }];

        (Some(matched_info), evaluated_info)
    } else {
        // No match found - list all mocks as not matched
        let all_mocks = app_state.mock.mock_registry.get_enabled_mocks();
        let evaluated_list = all_mocks
            .into_iter()
            .map(|mock| EvaluatedMock {
                id: mock.id.clone(),
                priority: mock.priority,
                matched: false,
                reason: Some("Did not match request criteria".to_string()),
                match_details: None,
            })
            .collect();

        (None, evaluated_list)
    };

    Json(InspectResponse {
        matched,
        evaluated,
        execution_time_us,
        cache_hit: false,
    })
    .into_response()
}
