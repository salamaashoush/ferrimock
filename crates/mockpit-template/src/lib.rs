//! Template engine for dynamic mock responses
//!
//! This module provides a powerful template engine for generating dynamic mock responses
//! using Tera templates with custom functions for fake data generation, request context,
//! and persistent state management.
//!
//! ## Module Structure
//!
//! - `error` - Template error handling and diagnostics
//! - `engine` - Core template engine with LRU caching
//! - `renderer` - Public API for rendering and validating templates
//! - `functions` - Registration of custom Tera functions
//! - `filters` - Custom Tera filters
//! - `store` - Persistence store and Tera functions
//! - `graphql_helpers` - GraphQL-specific template helpers

// Re-export the unified RequestContext from types
pub use mockpit_types::RequestContext;

// Public modules
pub mod error;
mod renderer;

// Private modules
mod engine;
mod fake_data;
mod filters;
mod functions;
pub mod graphql_helpers;
pub mod store;

// Re-export public APIs
pub use engine::hash_template;
pub use error::TemplateError;
pub use renderer::{
  render_patch_template, render_template, render_template_with_hash, render_template_with_id, validate_template,
};
pub use store::{get_global_persistence_store, set_global_persistence_store};
