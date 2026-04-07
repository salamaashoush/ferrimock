//! Mock management API handlers
//!
//! Provides endpoints for creating, reading, updating, and deleting mocks
//! using the config syntax directly.

use super::MockApiState;
use super::query::{apply_filters, parse_query};
use super::types::{MockOperationResponse, MockRequest, PatchMockRequest};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use lean_string::LeanString;
use serde::Deserialize;
use serde_json::json;

// ============================================================================
// Create/Update Mock
// ============================================================================

/// Create or update a mock using config syntax
///
/// POST /__mockpit/mocks
pub async fn create_mock(
    State(app_state): State<MockApiState>,
    Json(request): Json<MockRequest>,
) -> impl IntoResponse {
    use crate::engine::validation::MockValidator;

    let registry = &app_state.mock.mock_registry;

    // First validate the mock configuration
    let validator = MockValidator::new();
    let mock_collection = crate::config::MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![request.config.clone()],
    };

    let validation_result = Box::pin(validator.validate_config(&mock_collection, None)).await;

    if validation_result.has_errors() {
        let error_msg = validation_result.format_errors();
        let response = json!({
            "success": false,
            "error": error_msg
        });
        return (StatusCode::BAD_REQUEST, Json(response)).into_response();
    }

    // Convert config to mock definition
    match Box::pin(request.config.into_mock_definition()).await {
        Ok(mock) => {
            let id = mock.id.clone();
            let existed = registry.get_mock(&id).is_some();

            registry.add_mock(mock);

            Json(MockOperationResponse {
                success: true,
                mock_id: id,
                created: Some(!existed),
                message: None,
            })
            .into_response()
        }
        Err(e) => {
            let response = json!({
                "success": false,
                "error": e
            });
            (StatusCode::BAD_REQUEST, Json(response)).into_response()
        }
    }
}

/// Update a mock by ID
///
/// PUT /__mockpit/mocks/:id
pub async fn update_mock(
    Path(id): Path<String>,
    State(app_state): State<MockApiState>,
    Json(mut request): Json<MockRequest>,
) -> impl IntoResponse {
    use crate::engine::validation::MockValidator;

    let registry = &app_state.mock.mock_registry;
    // Ensure the ID matches
    request.config.id = LeanString::from(id.as_str());

    // Check if mock exists
    if registry.get_mock(&id).is_none() {
        let response = json!({
            "success": false,
            "error": format!("Mock with ID '{id}' not found")
        });
        return (StatusCode::NOT_FOUND, Json(response)).into_response();
    }

    // Validate the mock configuration
    let validator = MockValidator::new();
    let mock_collection = crate::config::MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![request.config.clone()],
    };

    let validation_result = Box::pin(validator.validate_config(&mock_collection, None)).await;

    if validation_result.has_errors() {
        let error_msg = validation_result.format_errors();
        let response = json!({
            "success": false,
            "error": error_msg
        });
        return (StatusCode::BAD_REQUEST, Json(response)).into_response();
    }

    // Convert config to mock definition
    match Box::pin(request.config.into_mock_definition()).await {
        Ok(mock) => {
            if let Err(e) = registry.update_mock(mock) {
                let response = json!({
                    "success": false,
                    "error": e
                });
                return (StatusCode::BAD_REQUEST, Json(response)).into_response();
            }

            Json(MockOperationResponse {
                success: true,
                mock_id: id.into(),
                created: Some(false),
                message: None,
            })
            .into_response()
        }
        Err(e) => {
            let response = json!({
                "success": false,
                "error": e
            });
            (StatusCode::BAD_REQUEST, Json(response)).into_response()
        }
    }
}

// ============================================================================
// Patch Mock (Partial Update)
// ============================================================================

/// Patch a mock (partial update)
///
/// PATCH /__mockpit/mocks/:id
pub async fn patch_mock(
    Path(id): Path<String>,
    State(app_state): State<MockApiState>,
    Json(request): Json<PatchMockRequest>,
) -> impl IntoResponse {
    tracing::debug!(mock_id = %id, changes = ?request.changes, "Patching mock");

    let registry = &app_state.mock.mock_registry;
    // Get existing mock
    let Some(arc_mock) = registry.get_mock(&id) else {
        tracing::warn!(mock_id = %id, "Mock not found for patch");
        let response = json!({
            "success": false,
            "error": format!("Mock with ID '{id}' not found")
        });
        return (StatusCode::NOT_FOUND, Json(response)).into_response();
    };

    let old_enabled = arc_mock.enabled;
    let mut mock = (*arc_mock).clone();

    // Apply patches
    let empty_map = serde_json::Map::new();
    let changes = request.changes.as_object().unwrap_or(&empty_map);

    for (key, value) in changes {
        match key.as_str() {
            "enabled" => {
                if let Some(enabled) = value.as_bool() {
                    tracing::debug!(mock_id = %id, old = old_enabled, new = enabled, "Setting mock enabled");
                    mock.enabled = enabled;
                }
            }
            "priority" => {
                if let Some(priority) = value.as_u64() {
                    mock.priority = u32::try_from(priority).unwrap_or(u32::MAX);
                }
            }
            "scope" => {
                if let Some(scope) = value.as_str() {
                    mock.scope = Some(scope.into());
                } else if value.is_null() {
                    mock.scope = None;
                }
            }
            // Add more patchable fields as needed
            _ => {
                tracing::debug!(mock_id = %id, key = %key, "Ignoring unknown patch field");
            }
        }
    }

    // Update mock
    if let Err(e) = registry.update_mock(mock) {
        tracing::error!(mock_id = %id, error = %e, "Failed to update mock in registry");
        let response = json!({
            "success": false,
            "error": e
        });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response();
    }

    // Verify the update
    if let Some(updated_mock) = registry.get_mock(&id) {
        tracing::debug!(mock_id = %id, enabled = updated_mock.enabled, "Mock patched successfully");
    }

    Json(MockOperationResponse {
        success: true,
        mock_id: id.into(),
        created: Some(false),
        message: Some("Mock patched successfully".to_string()),
    })
    .into_response()
}

// ============================================================================
// Delete Mock
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DeleteQueryParams {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    filter: Option<String>,
}

/// Delete mock(s)
///
/// DELETE /__mockpit/mocks/:id
/// DELETE /__mockpit/mocks?scope=test-*
/// DELETE /__mockpit/mocks?filter=priority<50
pub async fn delete_mock(
    id: Option<Path<String>>,
    Query(params): Query<DeleteQueryParams>,
    State(app_state): State<MockApiState>,
) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    // Single delete by ID
    if let Some(Path(id)) = id {
        if registry.remove_mock(&id).is_some() {
            return Json(json!({
                "success": true,
                "deleted": 1
            }))
            .into_response();
        }
        let response = json!({
            "success": false,
            "error": format!("Mock with ID '{id}' not found")
        });
        return (StatusCode::NOT_FOUND, Json(response)).into_response();
    }

    // Bulk delete by scope
    if let Some(scope) = params.scope {
        let deleted = registry.remove_mocks_by_scope(&scope);
        return Json(json!({
            "success": true,
            "deleted": deleted
        }))
        .into_response();
    }

    // Bulk delete by filter
    if let Some(filter_str) = params.filter {
        match parse_query(&filter_str) {
            Ok(filters) => {
                let all_mocks = registry.get_all_mocks();
                let to_delete = apply_filters(all_mocks, &filters);

                let mut deleted = 0;
                for mock in to_delete {
                    if registry.remove_mock(&mock.id).is_some() {
                        deleted += 1;
                    }
                }

                return Json(json!({
                    "success": true,
                    "deleted": deleted
                }))
                .into_response();
            }
            Err(e) => {
                let response = json!({
                    "success": false,
                    "error": format!("Invalid filter: {e}")
                });
                return (StatusCode::BAD_REQUEST, Json(response)).into_response();
            }
        }
    }

    let response = json!({
        "success": false,
        "error": "Must provide either mock ID, scope, or filter parameter"
    });
    (StatusCode::BAD_REQUEST, Json(response)).into_response()
}

// ============================================================================
// Get Mock(s)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct GetQueryParams {
    #[serde(default)]
    filter: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
}

/// Get mock(s)
///
/// GET /__mockpit/mocks/:id
/// GET /__mockpit/mocks
/// GET /__mockpit/mocks?filter=enabled=true&priority>100
/// GET /__mockpit/mocks?scope=integration-test
pub async fn get_mock(
    id: Option<Path<String>>,
    Query(params): Query<GetQueryParams>,
    State(app_state): State<MockApiState>,
) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    // Single get by ID
    if let Some(Path(id)) = id {
        if let Some(mock) = registry.get_mock(&id) {
            // Extract method from the mock's request matchers
            let method = if mock.request.methods.is_empty() {
                "ANY".to_string()
            } else {
                mock.request
                    .methods
                    .first()
                    .map_or("ANY".to_string(), |m| m.as_str().to_string())
            };

            // Extract URL from the mock's URL patterns
            let url = if mock.request.url_patterns.is_empty() {
                "/*".to_string()
            } else {
                match mock.request.url_patterns.first() {
                    Some(crate::engine::types::UrlPattern::Exact(path)) => path.clone(),
                    Some(crate::engine::types::UrlPattern::Prefix(prefix)) => format!("{prefix}*"),
                    Some(crate::engine::types::UrlPattern::Suffix(suffix)) => format!("*{suffix}"),
                    Some(crate::engine::types::UrlPattern::Regex(pattern)) => {
                        format!("~{pattern}")
                    }
                    Some(crate::engine::types::UrlPattern::Glob(_)) => "[glob pattern]".to_string(),
                    None => "/*".to_string(),
                }
            };

            return Json(json!({
                "id": mock.id,
                "method": method,
                "url": url,
                "priority": mock.priority,
                "enabled": mock.enabled,
                "scope": mock.scope
            }))
            .into_response();
        }
        let response = json!({
            "success": false,
            "error": format!("Mock with ID '{id}' not found")
        });
        return (StatusCode::NOT_FOUND, Json(response)).into_response();
    }

    // Get all mocks
    let mut mocks = registry.get_all_mocks();

    // Apply scope filter
    if let Some(scope) = params.scope {
        mocks.retain(|m| m.scope.as_deref() == Some(&scope));
    }

    // Apply query filter
    if let Some(filter_str) = params.filter {
        match parse_query(&filter_str) {
            Ok(filters) => {
                mocks = apply_filters(mocks, &filters);
            }
            Err(e) => {
                let response = json!({
                    "success": false,
                    "error": format!("Invalid filter: {e}")
                });
                return (StatusCode::BAD_REQUEST, Json(response)).into_response();
            }
        }
    }

    // Apply search pattern (q parameter) - case-insensitive substring search on ID and scope
    if let Some(search_pattern) = params.q {
        let pattern_lower = search_pattern.to_lowercase();
        mocks.retain(|m| {
            m.id.to_lowercase().contains(&pattern_lower)
                || m.scope
                    .as_ref()
                    .is_some_and(|s| s.to_lowercase().contains(&pattern_lower))
        });
    }

    // Apply sorting
    if let Some(sort_field) = params.sort {
        let descending = sort_field.starts_with('-');
        let field = sort_field.trim_start_matches('-');

        mocks.sort_by(|a, b| {
            let cmp = match field {
                "priority" => a.priority.cmp(&b.priority),
                "id" => a.id.cmp(&b.id),
                _ => std::cmp::Ordering::Equal,
            };

            if descending { cmp.reverse() } else { cmp }
        });
    }

    // Apply pagination
    let total = mocks.len();
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(total);

    // Extract just the IDs and metadata (MockDefinition isn't Serializable)
    let mock_info: Vec<_> = mocks
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|m| {
            // Extract method from the mock's request matchers
            let method = if m.request.methods.is_empty() {
                "ANY".to_string()
            } else {
                m.request
                    .methods
                    .first()
                    .map_or("ANY".to_string(), |meth| meth.as_str().to_string())
            };

            // Extract URL from the mock's URL patterns
            let url = if m.request.url_patterns.is_empty() {
                "/*".to_string()
            } else {
                match m.request.url_patterns.first() {
                    Some(crate::engine::types::UrlPattern::Exact(path)) => path.clone(),
                    Some(crate::engine::types::UrlPattern::Prefix(prefix)) => format!("{prefix}*"),
                    Some(crate::engine::types::UrlPattern::Suffix(suffix)) => format!("*{suffix}"),
                    Some(crate::engine::types::UrlPattern::Regex(pattern)) => {
                        format!("~{pattern}")
                    }
                    Some(crate::engine::types::UrlPattern::Glob(_)) => "[glob]".to_string(),
                    None => "/*".to_string(),
                }
            };

            json!({
                "id": m.id,
                "method": method,
                "url": url,
                "priority": m.priority,
                "enabled": m.enabled,
                "scope": m.scope
            })
        })
        .collect();

    Json(json!({
        "success": true,
        "total": total,
        "count": mock_info.len(),
        "offset": offset,
        "mocks": mock_info
    }))
    .into_response()
}
