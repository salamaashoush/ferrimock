//! Persistence store debugging endpoints

use crate::MockApiState;
use crate::types::{StoreDeleteResponse, StoreGetAllResponse, StoreMetadata, StoreSetRequest};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use rustc_hash::FxHashMap;

/// Get all store data
///
/// GET /__mockpit/store
pub async fn get_all_store(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let persistence_store = app_state.mock.mock_registry.get_persistence_store();

    // Get all keys
    let keys = persistence_store.keys();
    let total_keys = keys.len();

    // Build a map of all key-value pairs
    let mut all_data = FxHashMap::default();
    let mut keys_with_ttl = Vec::new();

    for key in &keys {
        if let Some(value) = persistence_store.get(key) {
            all_data.insert(key.clone(), value);

            // Check if key has TTL
            if let Some(ttl_secs) = persistence_store.ttl_seconds(key) {
                // Calculate expiration time
                let expires_at = chrono::Utc::now()
                    + chrono::Duration::seconds(i64::try_from(ttl_secs).unwrap_or(i64::MAX));
                keys_with_ttl.push(crate::types::KeyTtlInfo {
                    key: key.clone(),
                    ttl_seconds: ttl_secs,
                    expires_at: expires_at.to_rfc3339(),
                });
            }
        }
    }

    // Rough memory estimation (this is approximate)
    let memory_bytes = serde_json::to_string(&all_data).map_or(0, |s| s.len());

    Json(StoreGetAllResponse {
        store: all_data,
        metadata: StoreMetadata {
            total_keys,
            memory_bytes,
        },
        keys_with_ttl,
    })
}

/// Get single key from store
///
/// GET /__mockpit/store/:key
pub async fn get_store_key(
    Path(key): Path<String>,
    State(app_state): State<MockApiState>,
) -> impl IntoResponse {
    let persistence_store = app_state.mock.mock_registry.get_persistence_store();

    if let Some(value) = persistence_store.get(&key) {
        let ttl_seconds = persistence_store.ttl_seconds(&key);

        let mut response = serde_json::Map::new();
        response.insert("key".to_string(), serde_json::Value::String(key));
        response.insert("value".to_string(), value);
        if let Some(ttl) = ttl_seconds {
            response.insert(
                "ttl_seconds".to_string(),
                serde_json::Value::Number(ttl.into()),
            );
        } else {
            response.insert("ttl_seconds".to_string(), serde_json::Value::Null);
        }
        response.insert("exists".to_string(), serde_json::Value::Bool(true));

        Json(serde_json::Value::Object(response)).into_response()
    } else {
        let mut response = serde_json::Map::new();
        response.insert("key".to_string(), serde_json::Value::String(key.clone()));
        response.insert("exists".to_string(), serde_json::Value::Bool(false));
        response.insert(
            "error".to_string(),
            serde_json::Value::String(format!("Key '{key}' not found in store")),
        );
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::Value::Object(response)),
        )
            .into_response()
    }
}

/// Set store value
///
/// POST /__mockpit/store/:key
pub async fn set_store_key(
    Path(key): Path<String>,
    State(app_state): State<MockApiState>,
    Json(request): Json<StoreSetRequest>,
) -> impl IntoResponse {
    let persistence_store = app_state.mock.mock_registry.get_persistence_store();

    // Convert TTL from seconds to Duration if provided
    let ttl = request.ttl_seconds.map(std::time::Duration::from_secs);

    persistence_store.set_with_ttl(key.clone(), request.value, ttl);

    let mut response = serde_json::Map::new();
    response.insert("success".to_string(), serde_json::Value::Bool(true));
    response.insert("key".to_string(), serde_json::Value::String(key));
    response.insert(
        "message".to_string(),
        serde_json::Value::String("Value set successfully".to_string()),
    );

    Json(serde_json::Value::Object(response)).into_response()
}

/// Delete store key
///
/// DELETE /__mockpit/store/:key
pub async fn delete_store_key(
    Path(key): Path<String>,
    State(app_state): State<MockApiState>,
) -> impl IntoResponse {
    let persistence_store = app_state.mock.mock_registry.get_persistence_store();

    let deleted = persistence_store.delete(&key);

    if deleted {
        Json(StoreDeleteResponse {
            success: true,
            deleted: Some(true),
            keys_deleted: Some(1),
        })
        .into_response()
    } else {
        let mut response = serde_json::Map::new();
        response.insert("success".to_string(), serde_json::Value::Bool(false));
        response.insert("deleted".to_string(), serde_json::Value::Bool(false));
        response.insert(
            "error".to_string(),
            serde_json::Value::String(format!("Key '{key}' not found in store")),
        );
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::Value::Object(response)),
        )
            .into_response()
    }
}

/// Clear all store data
///
/// DELETE /__mockpit/store
pub async fn clear_store(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let persistence_store = app_state.mock.mock_registry.get_persistence_store();

    // Get count before clearing
    let count = persistence_store.len();

    persistence_store.clear();

    Json(StoreDeleteResponse {
        success: true,
        deleted: Some(count > 0),
        keys_deleted: Some(count),
    })
}
