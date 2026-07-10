#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Mock Engine Test Suite
//!
//! Organized test suite for the mock engine library.
//!
//! ## Test Organization
//!
//! - `core/` - Core mock engine functionality (matching, config, responses)
//! - `integration/` - Integration tests combining multiple features

pub mod core;
pub mod integration;
