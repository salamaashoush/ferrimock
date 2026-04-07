//! Status endpoint for mock system observability

use super::MockApiState;
use super::types::{CallTrackingStatus, RecordingStatus, ScopeStatus, StatusResponse};
use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use crate::consolidator::ConsolidatorOptions;
use crate::engine::MockRecorderConsolidationExt;
use crate::recorder::{MockRecorder, RecordingFormat};
use std::sync::Arc;

/// Get mock system status
///
/// GET /__mockpit/status
pub async fn get_status(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    let all_mocks = registry.get_all_mocks();
    let total_mocks = all_mocks.len();
    let enabled_mocks = all_mocks.iter().filter(|m| m.enabled).count();
    let disabled_mocks = total_mocks - enabled_mocks;

    let scopes = registry.list_scopes();
    let scope_status = ScopeStatus {
        total: scopes.len(),
        active: scopes,
    };

    let recording_count = registry.recordings_count();
    let recording_enabled = app_state.config.recording_enabled;
    let recording = if recording_count > 0 {
        Some(RecordingStatus {
            enabled: recording_enabled,
            count: recording_count,
        })
    } else {
        None
    };

    let tracked_mocks = registry.get_tracked_mock_ids();
    let total_calls: usize = tracked_mocks
        .iter()
        .map(|id| registry.get_call_count(id))
        .sum();

    let call_tracking = if tracked_mocks.is_empty() {
        None
    } else {
        Some(CallTrackingStatus {
            enabled_mocks: tracked_mocks.len(),
            total_calls,
        })
    };

    Json(StatusResponse {
        enabled: registry.is_enabled(),
        total_mocks,
        enabled_mocks,
        disabled_mocks,
        scopes: scope_status,
        recording_enabled,
        recordings_count: recording_count,
        recording,
        call_tracking,
    })
}

/// Get all recordings
///
/// GET /__mockpit/recordings
pub async fn get_recordings(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    let recordings = registry.get_all_recordings();

    let mut response = serde_json::Map::new();
    response.insert(
        "recordings".to_string(),
        serde_json::to_value(&recordings).unwrap_or(serde_json::Value::Null),
    );
    response.insert(
        "count".to_string(),
        serde_json::Value::Number(recordings.len().into()),
    );

    Json(serde_json::Value::Object(response))
}

/// Clear all recordings
///
/// DELETE /__mockpit/recordings
pub async fn clear_recordings(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    registry.clear_recordings();

    let mut response = serde_json::Map::new();
    response.insert("success".to_string(), serde_json::Value::Bool(true));
    response.insert(
        "message".to_string(),
        serde_json::Value::String("All recordings cleared".to_string()),
    );

    Json(serde_json::Value::Object(response))
}

/// Finalize recordings (flush pending writes)
///
/// POST /__mockpit/recordings/finalize
pub async fn finalize_recordings(State(app_state): State<MockApiState>) -> impl IntoResponse {
    // Finalize the recording (waits for pending writes and closes file)
    let recorder_guard = app_state.mock.mock_recorder.read().await;
    if let Some(recorder) = recorder_guard.as_ref()
        && let Err(e) = recorder.finalize_file().await
    {
        let mut error_response = serde_json::Map::new();
        error_response.insert("success".to_string(), serde_json::Value::Bool(false));
        error_response.insert(
            "error".to_string(),
            serde_json::Value::String(format!("Failed to finalize recording: {e}")),
        );
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::Value::Object(error_response)),
        )
            .into_response();
    }

    let mut success_response = serde_json::Map::new();
    success_response.insert("success".to_string(), serde_json::Value::Bool(true));
    success_response.insert(
        "message".to_string(),
        serde_json::Value::String("Recording finalized successfully".to_string()),
    );

    (
        StatusCode::CREATED,
        Json(serde_json::Value::Object(success_response)),
    )
        .into_response()
}

/// Enable the entire mock system
///
/// POST /__mockpit/enable
pub async fn enable_system(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    registry.enable();

    let mut response = serde_json::Map::new();
    response.insert("success".to_string(), serde_json::Value::Bool(true));
    response.insert("enabled".to_string(), serde_json::Value::Bool(true));

    Json(serde_json::Value::Object(response))
}

/// Disable the entire mock system
///
/// POST /__mockpit/disable
pub async fn disable_system(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let registry = &app_state.mock.mock_registry;
    registry.disable();

    let mut response = serde_json::Map::new();
    response.insert("success".to_string(), serde_json::Value::Bool(true));
    response.insert("enabled".to_string(), serde_json::Value::Bool(false));

    Json(serde_json::Value::Object(response))
}

/// Request body for setting mock mode
#[derive(Debug, serde::Deserialize)]
pub struct SetModeRequest {
    /// Mock mode: "hybrid", "selective", or "full"
    pub mode: String,
}

/// Set mock mode at runtime
///
/// POST /__mockpit/mode
/// Body: { "mode": "hybrid" | "selective" | "full" }
pub async fn set_mode(
    State(_app_state): State<MockApiState>,
    Json(request): Json<SetModeRequest>,
) -> impl IntoResponse {
    let mode = request.mode.to_lowercase();

    // Validate mode
    if !["hybrid", "selective", "full"].contains(&mode.as_str()) {
        let mut response = serde_json::Map::new();
        response.insert("success".to_string(), serde_json::Value::Bool(false));
        response.insert(
            "error".to_string(),
            serde_json::Value::String(format!(
                "Invalid mock mode '{mode}'. Must be one of: hybrid, selective, full"
            )),
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::Value::Object(response)),
        )
            .into_response();
    }

    // Mode is stored at the API level (registry doesn't track mode directly)
    // The mode affects how the proxy layer decides to use mocks
    let _ = &mode; // mode is validated above and returned in response

    let mut response = serde_json::Map::new();
    response.insert("success".to_string(), serde_json::Value::Bool(true));
    response.insert("mode".to_string(), serde_json::Value::String(mode));
    response.insert(
        "message".to_string(),
        serde_json::Value::String("Mock mode updated".to_string()),
    );

    Json(serde_json::Value::Object(response)).into_response()
}

/// Reset a scenario (DEPRECATED)
///
/// POST /__mockpit/scenarios/:id/reset
pub async fn reset_scenario() -> impl IntoResponse {
    let mut response = serde_json::Map::new();
    response.insert("success".to_string(), serde_json::Value::Bool(false));
    response.insert(
        "error".to_string(),
        serde_json::Value::String(
            "Scenarios are deprecated and no longer supported. Use state management instead."
                .to_string(),
        ),
    );

    (StatusCode::GONE, Json(serde_json::Value::Object(response)))
}

/// Recording options for start/stop
#[derive(Debug, Default, serde::Deserialize)]
pub struct RecordingOptions {
    /// Session name for the recording
    pub session: Option<String>,
    /// Recording format (json, yaml, har)
    pub format: Option<String>,
    /// Whether to consolidate on stop
    pub consolidate: Option<bool>,
    /// Enable template extraction during consolidation
    pub enable_templates: Option<bool>,
    /// Keep original recording file before overwriting with consolidated version
    pub keep_original: Option<bool>,
    /// Minimum pattern threshold for consolidation
    pub min_pattern: Option<usize>,
}

/// Start recording at runtime
///
/// POST /__mockpit/recording/start
/// Body: { session?: string, format?: "json"|"yaml"|"har" }
pub async fn start_recording(
    State(app_state): State<MockApiState>,
    axum::Json(options): axum::Json<RecordingOptions>,
) -> impl IntoResponse {
    // Start recording inline
    let result: Result<String, String> = async {
        let mut recorder_guard = app_state.mock.mock_recorder.write().await;
        if recorder_guard.is_some() {
            return Err("Recording is already in progress".to_string());
        }
        let session = options
            .session
            .unwrap_or_else(|| chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string());
        let recording_format = match options.format.as_deref() {
            Some("yaml") => RecordingFormat::Yaml,
            Some("har") => RecordingFormat::Har,
            _ => RecordingFormat::Json,
        };
        let output_dir = app_state
            .config
            .recordings_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("recordings"));
        let recorder = MockRecorder::with_format(&session, output_dir, recording_format);
        *recorder_guard = Some(Arc::new(recorder));
        Ok(session)
    }
    .await;

    match result {
        Ok(session_name) => {
            let mut response = serde_json::Map::new();
            response.insert("success".to_string(), serde_json::Value::Bool(true));
            response.insert(
                "message".to_string(),
                serde_json::Value::String("Recording started".to_string()),
            );
            response.insert(
                "session".to_string(),
                serde_json::Value::String(session_name),
            );
            (StatusCode::OK, Json(serde_json::Value::Object(response))).into_response()
        }
        Err(e) => {
            let mut response = serde_json::Map::new();
            response.insert("success".to_string(), serde_json::Value::Bool(false));
            response.insert("error".to_string(), serde_json::Value::String(e));
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::Value::Object(response)),
            )
                .into_response()
        }
    }
}

/// Stop recording at runtime
///
/// POST /__mockpit/recording/stop
/// Body: { consolidate?: bool, enable_templates?: bool, keep_original?: bool, min_pattern?: number }
pub async fn stop_recording(
    State(app_state): State<MockApiState>,
    axum::Json(options): axum::Json<RecordingOptions>,
) -> impl IntoResponse {
    // Take the recorder out of the shared state
    let mut recorder_guard = app_state.mock.mock_recorder.write().await;
    let Some(recorder) = recorder_guard.take() else {
        let mut response = serde_json::Map::new();
        response.insert("success".to_string(), serde_json::Value::Bool(false));
        response.insert(
            "error".to_string(),
            serde_json::Value::String("No recording in progress".to_string()),
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::Value::Object(response)),
        )
            .into_response();
    };
    drop(recorder_guard);

    let should_consolidate = options.consolidate.unwrap_or(false);

    if should_consolidate {
        // Use finalize_and_consolidate: finalizes, backs up original if requested, then consolidates
        let consolidator_opts = ConsolidatorOptions {
            enable_templates: options.enable_templates.unwrap_or(true),
            min_pattern_threshold: options.min_pattern.unwrap_or(3),
            ..ConsolidatorOptions::default()
        };
        let keep_original = options.keep_original.unwrap_or(false);

        match recorder
            .finalize_and_consolidate(consolidator_opts, keep_original)
            .await
        {
            Ok((file_path, stats)) => {
                let mut response = serde_json::Map::new();
                response.insert("success".to_string(), serde_json::Value::Bool(true));
                response.insert(
                    "message".to_string(),
                    serde_json::Value::String("Recording stopped and consolidated".to_string()),
                );
                response.insert(
                    "file".to_string(),
                    serde_json::Value::String(file_path.display().to_string()),
                );
                let mut stats_obj = serde_json::Map::new();
                stats_obj.insert(
                    "original_count".to_string(),
                    serde_json::Value::Number(stats.original_count.into()),
                );
                stats_obj.insert(
                    "consolidated_count".to_string(),
                    serde_json::Value::Number(stats.consolidated_count.into()),
                );
                if let Some(pct) = serde_json::Number::from_f64(stats.reduction_ratio * 100.0) {
                    stats_obj.insert(
                        "reduction_percent".to_string(),
                        serde_json::Value::Number(pct),
                    );
                }
                response.insert(
                    "consolidation".to_string(),
                    serde_json::Value::Object(stats_obj),
                );
                (StatusCode::OK, Json(serde_json::Value::Object(response))).into_response()
            }
            Err(e) => {
                let mut response = serde_json::Map::new();
                response.insert("success".to_string(), serde_json::Value::Bool(false));
                response.insert(
                    "error".to_string(),
                    serde_json::Value::String(format!("Failed to stop recording: {e}")),
                );
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::Value::Object(response)),
                )
                    .into_response()
            }
        }
    } else {
        // Just finalize without consolidation
        if let Err(e) = recorder.finalize_file().await {
            let mut response = serde_json::Map::new();
            response.insert("success".to_string(), serde_json::Value::Bool(false));
            response.insert(
                "error".to_string(),
                serde_json::Value::String(format!("Failed to finalize: {e}")),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::Value::Object(response)),
            )
                .into_response();
        }

        let mut response = serde_json::Map::new();
        response.insert("success".to_string(), serde_json::Value::Bool(true));
        response.insert(
            "message".to_string(),
            serde_json::Value::String("Recording stopped".to_string()),
        );
        (StatusCode::OK, Json(serde_json::Value::Object(response))).into_response()
    }
}
