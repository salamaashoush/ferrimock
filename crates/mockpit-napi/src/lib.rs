#![deny(clippy::all)]

//! Node.js NAPI bindings for the Mockpit HTTP mocking engine.
//!
//! Exposes both the MSW-style handler API and the declarative mock API to TypeScript/Node.js.
//!
//! Uses NAPI-RS's built-in shared tokio runtime.

mod config;
mod fake_ns;
mod handler_bridge;
mod http_ns;
mod graphql_ns;
mod response_ns;
mod server;
mod services;
mod types;
