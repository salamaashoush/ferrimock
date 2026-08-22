//! Core utilities for the ferrimock mocking framework
//!
//! Provides shared utilities used across the ferrimock crate ecosystem:
//! - `PersistenceStore` - Thread-safe in-memory key-value store for stateful mocking
//! - `Machine` - Named states and the moves between them, for anything that has them
//! - `World` - Typed, relational entity state a spec populates and every lane shares
//! - `levenshtein_distance` - String distance calculation for error suggestions

pub mod identity;
pub mod machine;
mod persistence;
mod utils;
pub mod world;

pub use identity::{app_name, set_app_name};
pub use machine::Machine;
pub use persistence::PersistenceStore;
pub use utils::levenshtein_distance;
pub use world::{EntityPage, EntityQuery, World, WorldSettings, global_world, set_global_world};
