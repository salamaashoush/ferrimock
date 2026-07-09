#![deny(clippy::all)]
// `#[napi]` exports are invoked from JS, not Rust — rustc/clippy see them as
// unused. Suppress crate-wide rather than annotating every binding.
#![allow(dead_code)]

//! Node.js NAPI bindings for the Mockpit HTTP mocking engine.
//!
//! Exposes both the MSW-style handler API and the declarative mock API to TypeScript/Node.js.
//!
//! Uses NAPI-RS's built-in shared tokio runtime.

mod config;
mod fake_ns;
mod graphql_ns;
mod handler_bridge;
mod http_ns;
mod request_context;
mod response_ns;
mod server;
mod services;
mod sse_ns;
mod types;
mod ws_ns;
