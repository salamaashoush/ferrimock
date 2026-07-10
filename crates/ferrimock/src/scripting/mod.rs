//! JS-scripted mock handlers on an embedded QuickJS engine.
//!
//! Lets `.js`/`.mjs` files in a mocks directory define MSW-style
//! handlers (`http.get('/api/users/:id', handler)`) that run without
//! Node: the CLI server, the library, and any Rust embedder execute
//! them on an in-process QuickJS VM.
//!
//! Matching never touches JS — a scripted mock is a normal
//! [`crate::types::MockDefinition`] whose body is
//! [`crate::types::BodySource::Handler`]; only response generation for
//! an already-matched request crosses into the VM.

mod bindings;
mod bridge;
mod bridge_streaming;
mod bundle;
mod bytecode_cache;
mod engine;
mod host;
mod loader;
mod slots;
pub mod vm;

pub use engine::{ScriptEngine, ScriptEngineConfig};
pub use host::ScriptHost;
