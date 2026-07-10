//! Core utilities for the ferrimock mocking framework
//!
//! Provides shared utilities used across the ferrimock crate ecosystem:
//! - `PersistenceStore` - Thread-safe in-memory key-value store for stateful mocking
//! - `levenshtein_distance` - String distance calculation for error suggestions

pub mod identity;
mod persistence;
mod utils;

pub use identity::{app_name, set_app_name};
pub use persistence::PersistenceStore;
pub use utils::levenshtein_distance;
