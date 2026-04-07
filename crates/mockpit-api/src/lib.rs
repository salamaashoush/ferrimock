//! Mock management API crate
//!
//! Provides HTTP handlers and router for the mock management API endpoints.
//! All routes are prefixed with `/__mockpit/`.

mod bulk;
mod handlers;
mod inspector;
mod query;
mod reload;
mod status;
mod store;
pub mod types;

pub use state::MockApiState;

mod state;

use axum::{
  Router,
  routing::{delete, get, post, put},
};

/// Create the mock management API router.
///
/// Returns an unfused `Router<MockApiState>` so callers can `.with_state()` or
/// `.merge()` it into a parent router after converting state.
pub fn create_mock_router() -> Router<MockApiState> {
  Router::new()
    // Status
    .route("/__mockpit/status", get(status::get_status))
    // Reload mocks from config files
    .route("/__mockpit/reload", post(reload::reload_mocks))
    // Mock CRUD (uses config syntax directly)
    .route("/__mockpit/mocks", post(handlers::create_mock))
    .route("/__mockpit/mocks", get(handlers::get_mock))
    .route("/__mockpit/mocks/{id}", get(handlers::get_mock))
    .route("/__mockpit/mocks/{id}", put(handlers::update_mock))
    .route(
      "/__mockpit/mocks/{id}",
      axum::routing::patch(handlers::patch_mock),
    )
    .route("/__mockpit/mocks/{id}", delete(handlers::delete_mock))
    // Scope/filter-based delete (no ID in path)
    .route("/__mockpit/mocks", delete(handlers::delete_mock))
    // Bulk operations
    .route("/__mockpit/bulk", post(bulk::bulk_operations))
    // Runtime inspector
    .route("/__mockpit/inspect", post(inspector::inspect_request))
    // Recording endpoints
    .route("/__mockpit/recordings", get(status::get_recordings))
    .route("/__mockpit/recordings", delete(status::clear_recordings))
    .route(
      "/__mockpit/recordings/finalize",
      post(status::finalize_recordings),
    )
    // System-wide enable/disable and mode control
    .route("/__mockpit/enable", post(status::enable_system))
    .route("/__mockpit/disable", post(status::disable_system))
    .route("/__mockpit/mode", post(status::set_mode))
    // Recording start/stop at runtime
    .route("/__mockpit/recording/start", post(status::start_recording))
    .route("/__mockpit/recording/stop", post(status::stop_recording))
    // Deprecated endpoints
    .route(
      "/__mockpit/scenarios/{id}/reset",
      post(status::reset_scenario),
    )
    // Persistence store debugging
    .route("/__mockpit/store", get(store::get_all_store))
    .route("/__mockpit/store", delete(store::clear_store))
    .route("/__mockpit/store/{key}", get(store::get_store_key))
    .route("/__mockpit/store/{key}", post(store::set_store_key))
    .route("/__mockpit/store/{key}", delete(store::delete_store_key))
}
