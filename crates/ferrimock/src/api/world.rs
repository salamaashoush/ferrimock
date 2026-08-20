//! Entity world endpoints.
//!
//! The typed counterpart of the `/__ferrimock/store` routes, sitting beside
//! them on purpose: one is untyped scratch state, the other is the world the
//! mocked API pretends to have. An external driver — a Playwright fixture, a
//! shell script — gets the same access a template or a script has, without
//! embedding the engine.

use super::MockApiState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use crate::core::EntityQuery;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldSummary {
    pub entities: Vec<EntitySummary>,
    pub seed: u64,
    /// Writes laid over the seeded world. Non-zero between tests means state
    /// is leaking from one into the next.
    pub pending_writes: usize,
    pub schemas: Vec<String>,
    /// Entity names declared by more than one schema, and merged.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collisions: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitySummary {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityPageResponse {
    pub records: Vec<JsonValue>,
    pub total: usize,
    pub has_next: bool,
    pub has_previous: bool,
}

/// Query string for a list: `?limit=25&skip=50&sort=-name&name=needle`.
///
/// Anything that is not a reserved word is a filter on that field, so
/// `?status=active` reads the way a real API's would.
#[derive(Debug, Default, Deserialize)]
pub struct ListParams {
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub skip: Option<usize>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(flatten)]
    pub filters: std::collections::BTreeMap<String, String>,
}

impl ListParams {
    fn into_query(self) -> EntityQuery {
        EntityQuery {
            filter: self
                .filters
                .into_iter()
                // A bare query parameter is a string; JSON-shaped values are
                // read as themselves so `?age={"gt":30}` and `?active=true`
                // both work.
                .map(|(field, raw)| {
                    let value = serde_json::from_str::<JsonValue>(&raw)
                        .unwrap_or_else(|_| JsonValue::String(raw));
                    (field, value)
                })
                .collect(),
            sort: self
                .sort
                .map(|raw| {
                    raw.split(',')
                        .map(str::trim)
                        .filter(|part| !part.is_empty())
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            skip: self.skip.unwrap_or(0),
            limit: self.limit,
        }
    }
}

fn failure(status: StatusCode, message: impl std::fmt::Display) -> (StatusCode, Json<JsonValue>) {
    (
        status,
        Json(serde_json::json!({ "error": message.to_string() })),
    )
}

/// GET /__ferrimock/world
pub async fn get_world(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();

    Json(WorldSummary {
        entities: world
            .entities()
            .into_iter()
            .map(|name| EntitySummary {
                count: world.count(name.as_str()),
                name: name.to_string(),
            })
            .collect(),
        seed: world.seed(),
        pending_writes: world.pending_writes(),
        #[cfg(feature = "spec")]
        schemas: world
            .schemas()
            .into_iter()
            .map(|schema| schema.path.display().to_string())
            .collect(),
        #[cfg(not(feature = "spec"))]
        schemas: Vec::new(),
        collisions: world
            .collisions()
            .into_iter()
            .map(|collision| collision.to_string())
            .collect(),
    })
}

/// GET /__ferrimock/world/{entity}
pub async fn list_entity(
    State(app_state): State<MockApiState>,
    Path(entity): Path<String>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();
    match world.list(&entity, &params.into_query()) {
        Ok(page) => Json(EntityPageResponse {
            records: page.records,
            total: page.total,
            has_next: page.has_next,
            has_previous: page.has_previous,
        })
        .into_response(),
        Err(e) => {
            failure(StatusCode::NOT_FOUND, unknown_entity(world, &entity, &e)).into_response()
        }
    }
}

/// GET /__ferrimock/world/{entity}/{key}
pub async fn get_entity(
    State(app_state): State<MockApiState>,
    Path((entity, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();
    match world.get(&entity, &key) {
        Some(record) => Json(record).into_response(),
        None => failure(
            StatusCode::NOT_FOUND,
            format!("no `{entity}` with key `{key}`"),
        )
        .into_response(),
    }
}

/// POST /__ferrimock/world/{entity}
pub async fn create_entity(
    State(app_state): State<MockApiState>,
    Path(entity): Path<String>,
    Json(values): Json<JsonValue>,
) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();
    match world.create(&entity, values) {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(e) => failure(StatusCode::BAD_REQUEST, e).into_response(),
    }
}

/// PATCH /__ferrimock/world/{entity}/{key}
pub async fn update_entity(
    State(app_state): State<MockApiState>,
    Path((entity, key)): Path<(String, String)>,
    Json(values): Json<JsonValue>,
) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();
    let existed = world.get(&entity, &key).is_some();
    match world.update(&entity, &key, values) {
        Ok(record) => Json(record).into_response(),
        // A record that is there and still would not take the write failed on
        // the values, not on the address; answering 404 sends the caller
        // looking for a record they are holding.
        Err(e) if existed => failure(StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => failure(StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// DELETE /__ferrimock/world/{entity}/{key}
pub async fn delete_entity(
    State(app_state): State<MockApiState>,
    Path((entity, key)): Path<(String, String)>,
) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();
    let existed = world.get(&entity, &key).is_some();
    match world.delete(&entity, &key) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        // With cascade off, a delete that would orphan children is refused —
        // which is a conflict with the world's state, not a missing record.
        Err(e) if existed => failure(StatusCode::CONFLICT, e).into_response(),
        Err(e) => failure(StatusCode::NOT_FOUND, e).into_response(),
    }
}

/// POST /__ferrimock/world/reset
///
/// Drops every write and leaves exactly what the seed derives. The counterpart
/// of `DELETE /__ferrimock/store` for typed state — call it between tests.
pub async fn reset_world(State(app_state): State<MockApiState>) -> impl IntoResponse {
    let world = app_state.mock.mock_registry.world();
    let dropped = world.pending_writes();
    world.reset();
    Json(serde_json::json!({ "reset": true, "droppedWrites": dropped }))
}

/// A typo otherwise reads as "that entity has no instances".
fn unknown_entity(
    world: &crate::core::World,
    entity: &str,
    error: &crate::FerrimockError,
) -> String {
    let graph = world.graph();
    match crate::core::world::nearest_entity(graph.as_ref(), entity) {
        Some(nearest) => format!("{error} — did you mean `{}`?", nearest.name),
        None => error.to_string(),
    }
}
