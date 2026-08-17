//! Spec-driven backends.
//!
//! A spec is a type system, not a list of observations, so it does not compile
//! to independent mocks. It compiles into the engine's
//! [`crate::core::World`] — types, keys, relations — which a seeded store
//! answers queries against and which protocol bindings serve.
//!
//! The split that keeps this honest: the world knows nothing about HTTP or
//! GraphQL and lives in `core`, because a spec populates it rather than owning
//! it. Everything here is about *reading a spec* ([`infer`]), *binding it to a
//! protocol* ([`bind`]) and *mounting it as ordinary mocks* ([`emit`]).

pub mod bind;
pub mod emit;
pub mod infer;
pub mod source;

pub use source::{SCHEMA_EXTENSIONS, is_schema_file, load_schema_file};
