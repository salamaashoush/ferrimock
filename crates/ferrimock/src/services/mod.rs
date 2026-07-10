//! Service layer — pure execution logic with no CLI/UI coupling.
//!
//! Each service takes typed input and returns typed output.
//! No `println!`, no terminal colors, no clap types.
//!
//! Consumers:
//! - NAPI bindings (for the TS CLI and Node.js API)
//! - Rust consumers building their own CLIs or integrations
//!
//! # Examples
//!
//! ```rust,ignore
//! use ferrimock::services::{serve, validate, fake_data};
//!
//! // Start a mock server
//! let handle = serve::start(serve::ServeInput {
//!     port: 3006,
//!     host: "127.0.0.1".into(),
//!     mocks_dir: Some("mocks/".into()),
//!     ..Default::default()
//! }).await?;
//!
//! // Validate mock files
//! let result = validate::validate(validate::ValidateInput {
//!     path: "mocks/collections".into(),
//! }).await?;
//!
//! // Generate fake data
//! let values = fake_data::generate(fake_data::FakeDataInput {
//!     generator: "email".into(),
//!     count: 5,
//!     ..Default::default()
//! })?;
//! ```

pub mod config;
pub mod consolidate;
pub mod convert;
pub mod create;
pub mod export;
pub mod fake_data;
pub mod fake_image;
pub mod fake_pdf;
pub mod format;
pub mod list;
/// Standalone mock HTTP server (requires the `server` feature for axum/tower-http).
#[cfg(feature = "server")]
pub mod serve;
pub mod show;
pub mod template;
pub mod test_match;
pub mod validate;
