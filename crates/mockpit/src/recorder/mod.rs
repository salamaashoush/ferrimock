//! HTTP request/response recording for mock generation
//!
//! This crate provides functionality to record HTTP interactions and save them
//! in various formats including mock collections (JSON, YAML, TOML) and HAR files.
//!
//! ## Features
//!
//! - Record HTTP requests and responses
//! - Multiple output formats: JSON, YAML, TOML, HAR
//! - Streaming writes for low memory overhead
//! - GraphQL request detection
//! - Gzip decompression
//! - Flexible filtering options
//! - Auto-export on error with context
//!
//! ## Example
//!
//! ```rust,no_run
//! use crate::recorder::{MockRecorder, RecordingFormat};
//! use http::{Method, StatusCode, HeaderMap};
//! use bytes::Bytes;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create a recorder
//!     let recorder = MockRecorder::new("my-session", "./recordings");
//!
//!     // Initialize for streaming
//!     recorder.init_file().await?;
//!
//!     // Record an interaction
//!     recorder.record(
//!         &Method::GET,
//!         "/api/users",
//!         None,
//!         &HeaderMap::new(),
//!         None,
//!         StatusCode::OK,
//!         &HeaderMap::new(),
//!         &Bytes::from(r#"[{"id": 1, "name": "Alice"}]"#),
//!         Duration::from_millis(50),
//!     ).await?;
//!
//!     // Finalize the recording
//!     recorder.finalize_file().await?;
//!
//!     Ok(())
//! }
//! ```

pub mod filters;
mod formats;
mod har;
mod session;
mod types;

// Re-export the main recorder and public types
pub use filters::RecordingFilterOptions;
pub use formats::RecordingFormat;
pub use types::{RecordedInteraction, RecordedRequest, RecordedResponse, RecordingSession};

// Re-export the main recorder
mod mock_recorder;
pub use mock_recorder::MockRecorder;
