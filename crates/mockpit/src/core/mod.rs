//! Core utilities for the mockpit mocking framework
//!
//! Provides shared utilities used across the mockpit crate ecosystem:
//! - `PersistenceStore` - Thread-safe in-memory key-value store for stateful mocking
//! - `levenshtein_distance` - String distance calculation for error suggestions

mod persistence;
mod utils;

pub use persistence::PersistenceStore;
pub use utils::levenshtein_distance;
