//! Bulk operations handler for mock management

use super::MockApiState;
use super::query::{apply_filters, parse_query};
use super::types::{BulkOpResult, BulkOperation, BulkOperationRequest, BulkOperationResponse};
use crate::engine::MockRegistry;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use lean_string::LeanString;
use std::sync::Arc;

/// Execute bulk operations on mocks
///
/// POST /__ferrimock/bulk
pub async fn bulk_operations(
    State(app_state): State<MockApiState>,
    Json(request): Json<BulkOperationRequest>,
) -> impl IntoResponse {
    let registry = Arc::clone(&app_state.mock.mock_registry);
    let mut results = Vec::new();
    let atomic = request.atomic;

    // If atomic, we need to create a snapshot for rollback
    let snapshot = if atomic {
        Some(registry.get_all_mocks())
    } else {
        None
    };

    for operation in request.operations {
        let result = Box::pin(execute_operation(Arc::clone(&registry), operation)).await;

        // If atomic and operation failed, rollback and return error
        if atomic && !result.success {
            if let Some(snapshot_mocks) = snapshot {
                // Rollback: clear registry and restore snapshot
                registry.clear();
                for mock in snapshot_mocks {
                    registry.add_mock((*mock).clone());
                }
            }

            let mut response = serde_json::Map::new();
            response.insert("success".to_string(), serde_json::Value::Bool(false));
            response.insert(
                "error".to_string(),
                serde_json::Value::String(
                    "Atomic operation failed, all changes rolled back".to_string(),
                ),
            );
            response.insert(
                "failed_operation".to_string(),
                serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::Value::Object(response)),
            )
                .into_response();
        }

        results.push(result);
    }

    Json(BulkOperationResponse {
        success: true,
        results,
    })
    .into_response()
}

async fn execute_operation(registry: Arc<MockRegistry>, operation: BulkOperation) -> BulkOpResult {
    match operation {
        BulkOperation::Create { mock } => match Box::pin(mock.into_mock_definition()).await {
            Ok(mock_def) => {
                let id = mock_def.id.clone();
                registry.add_mock(mock_def);
                BulkOpResult {
                    op: "create".to_string(),
                    id: Some(id),
                    success: true,
                    affected: None,
                    error: None,
                }
            }
            Err(e) => BulkOpResult {
                op: "create".to_string(),
                id: None,
                success: false,
                affected: None,
                error: Some(e.to_string()),
            },
        },

        BulkOperation::Update { id, mock } => {
            let lean_id: LeanString = id.into();
            match Box::pin(mock.into_mock_definition()).await {
                Ok(mut mock_def) => {
                    mock_def.id.clone_from(&lean_id);
                    match registry.update_mock(mock_def) {
                        Ok(()) => BulkOpResult {
                            op: "update".to_string(),
                            id: Some(lean_id),
                            success: true,
                            affected: None,
                            error: None,
                        },
                        Err(e) => BulkOpResult {
                            op: "update".to_string(),
                            id: Some(lean_id),
                            success: false,
                            affected: None,
                            error: Some(e.to_string()),
                        },
                    }
                }
                Err(e) => BulkOpResult {
                    op: "update".to_string(),
                    id: Some(lean_id),
                    success: false,
                    affected: None,
                    error: Some(e.to_string()),
                },
            }
        }

        BulkOperation::Patch { id, changes } => {
            let lean_id: LeanString = id.into();
            match registry.get_mock(&lean_id) {
                Some(arc_mock) => {
                    let mut mock = (*arc_mock).clone();
                    let empty_map = serde_json::Map::new();
                    let changes_obj = changes.as_object().unwrap_or(&empty_map);

                    for (key, value) in changes_obj {
                        match key.as_str() {
                            "enabled" => {
                                if let Some(enabled) = value.as_bool() {
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
                            _ => {}
                        }
                    }

                    match registry.update_mock(mock) {
                        Ok(()) => BulkOpResult {
                            op: "patch".to_string(),
                            id: Some(lean_id),
                            success: true,
                            affected: None,
                            error: None,
                        },
                        Err(e) => BulkOpResult {
                            op: "patch".to_string(),
                            id: Some(lean_id),
                            success: false,
                            affected: None,
                            error: Some(e.to_string()),
                        },
                    }
                }
                None => BulkOpResult {
                    op: "patch".to_string(),
                    id: Some(lean_id.clone()),
                    success: false,
                    affected: None,
                    error: Some(format!("Mock '{lean_id}' not found")),
                },
            }
        }

        BulkOperation::Delete { id, filter } => {
            if let Some(id) = id {
                let lean_id: LeanString = id.into();
                if registry.remove_mock(&lean_id).is_some() {
                    BulkOpResult {
                        op: "delete".to_string(),
                        id: Some(lean_id),
                        success: true,
                        affected: Some(1),
                        error: None,
                    }
                } else {
                    BulkOpResult {
                        op: "delete".to_string(),
                        id: Some(lean_id.clone()),
                        success: false,
                        affected: Some(0),
                        error: Some(format!("Mock '{lean_id}' not found")),
                    }
                }
            } else if let Some(filter_str) = filter {
                // Delete by filter
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

                        BulkOpResult {
                            op: "delete".to_string(),
                            id: None,
                            success: true,
                            affected: Some(deleted),
                            error: None,
                        }
                    }
                    Err(e) => BulkOpResult {
                        op: "delete".to_string(),
                        id: None,
                        success: false,
                        affected: Some(0),
                        error: Some(format!("Invalid filter: {e}")),
                    },
                }
            } else {
                BulkOpResult {
                    op: "delete".to_string(),
                    id: None,
                    success: false,
                    affected: Some(0),
                    error: Some("Must provide either 'id' or 'filter'".to_string()),
                }
            }
        }

        BulkOperation::Enable { id, ids, filter } => {
            let mut affected = 0;

            if let Some(id) = id {
                // Enable single mock
                if registry.enable_mock(&id).is_ok() {
                    affected = 1;
                }
            } else if let Some(ids) = ids {
                // Enable multiple mocks by IDs
                for id in ids {
                    if registry.enable_mock(&id).is_ok() {
                        affected += 1;
                    }
                }
            } else if let Some(filter_str) = filter {
                // Enable by filter
                if let Ok(filters) = parse_query(&filter_str) {
                    let all_mocks = registry.get_all_mocks();
                    let to_enable = apply_filters(all_mocks, &filters);

                    for mock in to_enable {
                        if registry.enable_mock(&mock.id).is_ok() {
                            affected += 1;
                        }
                    }
                }
            }

            BulkOpResult {
                op: "enable".to_string(),
                id: None,
                success: true,
                affected: Some(affected),
                error: None,
            }
        }

        BulkOperation::Disable { id, ids, filter } => {
            let mut affected = 0;

            if let Some(id) = id {
                // Disable single mock
                if registry.disable_mock(&id).is_ok() {
                    affected = 1;
                }
            } else if let Some(ids) = ids {
                // Disable multiple mocks by IDs
                for id in ids {
                    if registry.disable_mock(&id).is_ok() {
                        affected += 1;
                    }
                }
            } else if let Some(filter_str) = filter {
                // Disable by filter
                if let Ok(filters) = parse_query(&filter_str) {
                    let all_mocks = registry.get_all_mocks();
                    let to_disable = apply_filters(all_mocks, &filters);

                    for mock in to_disable {
                        if registry.disable_mock(&mock.id).is_ok() {
                            affected += 1;
                        }
                    }
                }
            }

            BulkOpResult {
                op: "disable".to_string(),
                id: None,
                success: true,
                affected: Some(affected),
                error: None,
            }
        }
    }
}
