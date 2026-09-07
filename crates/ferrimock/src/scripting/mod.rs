//! JS-scripted mock handlers on the ferrijs QuickJS runtime.
//!
//! Lets `.js`/`.mjs`/`.ts`/`.mts` files in a mocks directory define
//! MSW-style handlers (`http.get('/api/users/:id', handler)`) that run
//! without Node: the CLI server, the library, and any Rust embedder
//! execute them on an in-process realm.
//!
//! Matching never touches JS -- a scripted mock is a normal
//! [`crate::types::MockDefinition`] whose body is
//! [`crate::types::BodySource::Handler`]; only response generation for
//! an already-matched request crosses into the realm.

mod bindings;
mod bridge;
mod bridge_streaming;
mod engine;
mod host;
mod loader;
mod slots;

pub use engine::{ScriptEngine, ScriptEngineConfig};
pub use host::ScriptHost;
