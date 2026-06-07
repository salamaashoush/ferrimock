//! # Mockpit
//!
//! A high-performance HTTP mocking engine with templates, recording,
//! consolidation, and GraphQL support.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use mockpit::prelude::*;
//!
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create a mock registry and load mocks from a directory
//!     let registry = MockRegistry::new();
//!     registry.load_from_directory("mocks/").await?;
//!
//!     // Create a matcher to evaluate incoming requests
//!     let matcher = MockMatcher::new(registry);
//!     Ok(())
//! }
//! ```
//!
//! ## Feature Flags
//!
//! | Feature | Default | Description |
//! |---------|---------|-------------|
//! | `engine` | yes | Core mock engine (types, config, matching, registry, templates) |
//! | `fake-data` | yes | Fake data generators (names, emails, UUIDs, etc.) |
//! | `type-detector` | no | Semantic type detection from field names and JSON values |
//! | `codegen` | no | Template code generation from detected types |
//! | `graphql` | no | GraphQL schema introspection and mock generation |
//! | `server` | no | HTTP server with hot reload and graceful shutdown |
//! | `api` | no | Mock management HTTP API (axum router) |
//! | `schema` | no | JSON schema generation for config validation |
//! | `full` | no | Enable everything |

// ---------------------------------------------------------------------------
// Core (always available when engine is enabled)
// ---------------------------------------------------------------------------

/// The library error type ([`MockpitError`]) and [`Result`] alias.
pub mod error;
pub use error::{MockpitError, Result};

/// Thread-safe persistence store for stateful mocking
#[cfg(feature = "engine")]
pub mod core;

/// HTTP mock types: request matching, URL patterns, response generation
#[cfg(feature = "engine")]
pub mod types;

/// Mock configuration parsing (YAML/JSON) and HAR file loading
#[cfg(feature = "engine")]
pub mod config;

/// HTTP request/response recording for mock generation
#[cfg(feature = "engine")]
pub mod recorder;

/// Template rendering engine with fake data integration
#[cfg(feature = "engine")]
pub mod template;

/// Smart mock consolidation with pattern detection (90%+ reduction)
#[cfg(feature = "engine")]
pub mod consolidator;

/// Core mock engine: registry, matcher, validation, scopes
#[cfg(feature = "engine")]
pub mod engine;

/// Ergonomic builder API for handler-based mock definitions (MSW-style)
#[cfg(feature = "engine")]
pub mod handler;

/// Service layer — pure execution logic with no CLI/UI coupling.
/// Used by NAPI bindings, TS CLI, and Rust consumers.
#[cfg(feature = "engine")]
pub mod services;

// ---------------------------------------------------------------------------
// Optional features
// ---------------------------------------------------------------------------

/// Fake data generators for realistic mock responses
#[cfg(feature = "fake-data")]
pub mod fake_data;

/// Semantic type detection from field names and JSON values
#[cfg(feature = "type-detector")]
pub mod type_detector;

/// Template code generation from detected types
#[cfg(feature = "codegen")]
pub mod codegen;

/// GraphQL schema introspection and mock generation
#[cfg(feature = "graphql")]
pub mod graphql;

/// HTTP server utilities: hot reload, graceful shutdown, state management
#[cfg(feature = "server")]
pub mod server;

/// Mock management HTTP API (axum router)
#[cfg(feature = "api")]
pub mod api;

// ---------------------------------------------------------------------------
// Prelude - the most commonly used types for quick imports
// ---------------------------------------------------------------------------

/// Common imports for working with mockpit
///
/// ```rust
/// use mockpit::prelude::*;
/// ```
#[cfg(feature = "engine")]
pub mod prelude {
    // Engine essentials
    pub use crate::engine::{MockMatcher, MockRegistry};

    // Config types
    pub use crate::config::{MockCollectionConfig, MockConfig};

    // Request types
    pub use crate::types::RequestContext;

    // Template rendering
    pub use crate::template::render_template;

    // Recording
    pub use crate::recorder::MockRecorder;

    // Persistence
    pub use crate::core::PersistenceStore;

    // Handler API
    pub use crate::handler::{DynamicResponseExt, IntoHandlerFn, MockResponse};
    pub use crate::types::{DynamicResponse, HandlerFn};
}
