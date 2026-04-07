//! Core mock engine functionality tests
//!
//! Tests for fundamental mock engine features including:
//! - Configuration loading and validation (TOML/JSON)
//! - Request matching (URL patterns, methods, headers, query params, body)
//! - Response generation (status codes, headers, body)
//! - Mock lifecycle management (enable/disable, priority)

pub mod config_loading;
pub mod request_matching;
pub mod response_generation;
