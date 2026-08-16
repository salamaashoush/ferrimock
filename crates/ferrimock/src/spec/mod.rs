//! Spec-driven virtual backends.
//!
//! A spec is a type system, not a list of observations, so it does not compile
//! to independent mocks. It compiles to an [`model::EntityGraph`] — types,
//! keys, relations — which a seeded [`store::EntityStore`] answers queries
//! against, and which protocol bindings serve.
//!
//! The split that keeps this honest: [`model`] and [`algebra`] know nothing
//! about HTTP or GraphQL, and the store speaks only the algebra. Paths, status
//! codes, selection sets and connections live in the bindings.

pub mod algebra;
pub mod bind;
pub mod emit;
pub mod infer;
pub mod model;
pub mod store;
