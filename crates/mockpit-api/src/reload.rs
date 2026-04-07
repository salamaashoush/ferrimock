//! Mock reload handler - loads mocks from config files

use crate::MockApiState;
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde::{Deserialize, Serialize};
use tracing::{debug, error};

/// Reload request
#[derive(Debug, Deserialize)]
pub struct ReloadRequest {
    pub dir: Option<String>,
}

/// Reload response
#[derive(Debug, Serialize)]
pub struct ReloadResponse {
    pub success: bool,
    pub message: String,
    pub count: usize,
}

/// POST /__mockpit/reload
///
/// Reloads mocks from collections directory AND recordings directory (same as startup behavior)
pub async fn reload_mocks(
    State(app_state): State<MockApiState>,
    Json(payload): Json<ReloadRequest>,
) -> impl IntoResponse {
    // Clear existing mocks
    app_state.mock.mock_registry.clear();

    let mut total_count = 0;
    let mut loaded_dirs = Vec::new();

    // If a specific dir is provided, only load from that
    if let Some(dir) = payload.dir {
        match app_state.mock.mock_registry.load_from_directory(&dir).await {
            Ok(count) => {
                debug!("Reloaded {} mocks from {}", count, dir);
                total_count += count;
                loaded_dirs.push(dir);
            }
            Err(e) => {
                error!("Failed to reload mocks: {}", e);
                let mut response = serde_json::Map::new();
                response.insert("success".to_string(), serde_json::Value::Bool(false));
                response.insert(
                    "error".to_string(),
                    serde_json::Value::String(format!("Failed to reload: {e}")),
                );
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::Value::Object(response)),
                )
                    .into_response();
            }
        }
    } else {
        // Load from collections directory
        if let Some(collections_dir) = &app_state.config.collections_dir {
            let collections_str = collections_dir.to_string_lossy().to_string();
            match app_state
                .mock
                .mock_registry
                .load_from_directory(&collections_str)
                .await
            {
                Ok(count) => {
                    debug!(
                        "Reloaded {} mocks from collections: {}",
                        count, collections_str
                    );
                    total_count += count;
                    loaded_dirs.push(collections_str);
                }
                Err(e) => {
                    error!("Failed to reload mocks from collections: {}", e);
                    // Continue to try recordings dir
                }
            }
        }

        // Also load from recordings directory if it exists
        if let Some(recordings_dir) = &app_state.config.recordings_dir {
            if recordings_dir.exists() {
                let recordings_str = recordings_dir.to_string_lossy().to_string();
                match app_state
                    .mock
                    .mock_registry
                    .load_from_directory(&recordings_str)
                    .await
                {
                    Ok(count) => {
                        debug!(
                            "Reloaded {} recordings as mocks from: {}",
                            count, recordings_str
                        );
                        total_count += count;
                        loaded_dirs.push(recordings_str);
                    }
                    Err(e) => {
                        error!("Failed to reload recordings: {}", e);
                        // Non-fatal, continue
                    }
                }
            }
        }
    }

    let response = ReloadResponse {
        success: true,
        message: format!(
            "Reloaded {} mock(s) from: {}",
            total_count,
            loaded_dirs.join(", ")
        ),
        count: total_count,
    };

    (StatusCode::OK, Json(response)).into_response()
}
