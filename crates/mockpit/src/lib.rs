//! # Mockpit
//!
//! A high-performance HTTP mocking engine with templates, recording,
//! consolidation, and GraphQL support.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use mockpit::prelude::*;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Create a mock registry and load mocks from a directory
//! let registry = MockRegistry::new();
//! registry.load_from_directory("mocks/").await?;
//!
//! // Create a matcher to evaluate incoming requests
//! let matcher = MockMatcher::new(registry);
//! # Ok(())
//! # }
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
//! | `cli` | no | Mock management CLI commands |
//! | `schema` | no | JSON schema generation for config validation |
//! | `full` | no | Enable everything |

// ---------------------------------------------------------------------------
// Core (always available when engine is enabled)
// ---------------------------------------------------------------------------

/// Thread-safe persistence store for stateful mocking
#[cfg(feature = "engine")]
pub mod core {
    pub use mockpit_core::*;
}

/// HTTP mock types: request matching, URL patterns, response generation
#[cfg(feature = "engine")]
pub mod types {
    pub use mockpit_types::*;
}

/// Mock configuration parsing (YAML/JSON) and HAR file loading
#[cfg(feature = "engine")]
pub mod config {
    pub use mockpit_config::*;
}

/// HTTP request/response recording for mock generation
#[cfg(feature = "engine")]
pub mod recorder {
    pub use mockpit_recorder::*;
}

/// Template rendering engine with fake data integration
#[cfg(feature = "engine")]
pub mod template {
    pub use mockpit_template::*;
}

/// Smart mock consolidation with pattern detection (90%+ reduction)
#[cfg(feature = "engine")]
pub mod consolidator {
    pub use mockpit_consolidator::*;
}

/// Core mock engine: registry, matcher, validation, scopes
#[cfg(feature = "engine")]
pub mod engine {
    pub use mockpit_engine::*;
}

// ---------------------------------------------------------------------------
// Optional features
// ---------------------------------------------------------------------------

/// Fake data generators for realistic mock responses
#[cfg(feature = "fake-data")]
pub mod fake_data {
    pub use mockpit_fake_data::*;
}

/// Semantic type detection from field names and JSON values
#[cfg(feature = "type-detector")]
pub mod type_detector {
    pub use mockpit_type_detector::*;
}

/// Template code generation from detected types
#[cfg(feature = "codegen")]
pub mod codegen {
    pub use mockpit_codegen::*;
}

/// GraphQL schema introspection and mock generation
#[cfg(feature = "graphql")]
pub mod graphql {
    pub use mockpit_graphql::*;
}

/// HTTP server utilities: hot reload, graceful shutdown, state management
#[cfg(feature = "server")]
pub mod server {
    pub use mockpit_server::*;
}

/// Mock management HTTP API (axum router)
#[cfg(feature = "api")]
pub mod api {
    pub use mockpit_api::*;
}

/// Mock management CLI commands
#[cfg(feature = "cli")]
pub mod cli {
    pub use mockpit_cli::*;
}

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
    pub use mockpit_engine::{MockMatcher, MockRegistry};

    // Config types
    pub use mockpit_config::{MockCollectionConfig, MockConfig};

    // Request types
    pub use mockpit_types::RequestContext;

    // Template rendering
    pub use mockpit_template::render_template;

    // Recording
    pub use mockpit_recorder::MockRecorder;

    // Persistence
    pub use mockpit_core::PersistenceStore;
}
