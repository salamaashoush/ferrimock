//! GraphQL binding: request decoding, schema construction, resolution.

pub mod classify;
pub mod request;
pub mod schema;
pub mod value;

pub use request::parse_request;
pub use schema::{Coverage, GraphQLBackend};
