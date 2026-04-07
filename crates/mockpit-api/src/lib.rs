//! Mock management API crate
//!
//! Provides HTTP handlers and router for the mock management API endpoints.
//! Routes are prefixed with a configurable prefix (defaults to `/__mockpit`).

mod bulk;
mod handlers;
mod inspector;
mod query;
mod reload;
mod status;
mod store;
pub mod types;

pub use state::{MockApiConfig, MockApiState};

mod state;

use axum::{
  Router,
  routing::{delete, get, post, put},
};

/// Default route prefix for the mock management API.
pub const DEFAULT_PREFIX: &str = "/__mockpit";

/// Create the mock management API router with the default prefix (`/__mockpit`).
///
/// Returns an unfused `Router<MockApiState>` so callers can `.with_state()` or
/// `.merge()` it into a parent router after converting state.
pub fn create_mock_router() -> Router<MockApiState> {
  create_mock_router_with_prefix(DEFAULT_PREFIX)
}

/// Create the mock management API router with a custom route prefix.
///
/// This allows embedders to use their own prefix (e.g., `/__box_dev_gate_mock`).
pub fn create_mock_router_with_prefix(prefix: &str) -> Router<MockApiState> {
  let p = prefix.to_string();
  Router::new()
    // Status
    .route(&format!("{p}/status"), get(status::get_status))
    // Reload mocks from config files
    .route(&format!("{p}/reload"), post(reload::reload_mocks))
    // Mock CRUD (uses config syntax directly)
    .route(&format!("{p}/mocks"), post(handlers::create_mock))
    .route(&format!("{p}/mocks"), get(handlers::get_mock))
    .route(&format!("{p}/mocks/{{id}}"), get(handlers::get_mock))
    .route(&format!("{p}/mocks/{{id}}"), put(handlers::update_mock))
    .route(
      &format!("{p}/mocks/{{id}}"),
      axum::routing::patch(handlers::patch_mock),
    )
    .route(&format!("{p}/mocks/{{id}}"), delete(handlers::delete_mock))
    // Scope/filter-based delete (no ID in path)
    .route(&format!("{p}/mocks"), delete(handlers::delete_mock))
    // Bulk operations
    .route(&format!("{p}/bulk"), post(bulk::bulk_operations))
    // Runtime inspector
    .route(&format!("{p}/inspect"), post(inspector::inspect_request))
    // Recording endpoints
    .route(&format!("{p}/recordings"), get(status::get_recordings))
    .route(&format!("{p}/recordings"), delete(status::clear_recordings))
    .route(
      &format!("{p}/recordings/finalize"),
      post(status::finalize_recordings),
    )
    // System-wide enable/disable and mode control
    .route(&format!("{p}/enable"), post(status::enable_system))
    .route(&format!("{p}/disable"), post(status::disable_system))
    .route(&format!("{p}/mode"), post(status::set_mode))
    // Recording start/stop at runtime
    .route(&format!("{p}/recording/start"), post(status::start_recording))
    .route(&format!("{p}/recording/stop"), post(status::stop_recording))
    // Deprecated endpoints
    .route(
      &format!("{p}/scenarios/{{id}}/reset"),
      post(status::reset_scenario),
    )
    // Persistence store debugging
    .route(&format!("{p}/store"), get(store::get_all_store))
    .route(&format!("{p}/store"), delete(store::clear_store))
    .route(&format!("{p}/store/{{key}}"), get(store::get_store_key))
    .route(&format!("{p}/store/{{key}}"), post(store::set_store_key))
    .route(&format!("{p}/store/{{key}}"), delete(store::delete_store_key))
}
