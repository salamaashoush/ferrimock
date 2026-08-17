//! Back ends: binding a schema to the ordinary mocks that serve it.

pub mod live;
pub mod rest;

pub use live::{SPEC_PRIORITY, bind_graphql};
pub use rest::bind_rest;
