//! Mockpit CLI library: mock management and fake data command implementations.

pub mod commands;

// Re-export the command entry points and types for convenience
pub use commands::fake;
pub use commands::{FakeCommand, MockCommand, execute};
