//! Server utilities for ferrimock: hot reload, graceful shutdown, and state management
//!
//! Provides infrastructure for running ferrimock as an HTTP server:
//! - `MockState` - Mock system state (registry + matcher + recorder)
//! - `HotReloadManager` - Automatic mock reloading on file changes
//! - `serve_with_graceful_shutdown` - HTTP server with graceful shutdown

pub mod debouncer;
pub mod file_watcher;
pub mod hot_reload;
pub mod serve;
pub mod sse;
pub mod state;
pub mod ws;

pub use debouncer::{DebouncedEvent, EventDebouncer};
pub use file_watcher::{FileEvent, FileEventFilter, FileWatcher};
pub use hot_reload::{HotReloadConfig, HotReloadManager};
pub use serve::serve_with_graceful_shutdown;
pub use state::{ConsolidateOptions, MockState, StopRecordingResult};
