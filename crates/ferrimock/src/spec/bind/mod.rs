//! Protocol bindings: the only place that knows what HTTP or GraphQL is.

pub mod graphql;
pub mod plan;
pub mod rest;

pub use plan::RootPlan;
