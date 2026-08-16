//! Back ends: turning a compiled spec into something that serves.

pub mod live;

pub use live::{SPEC_PRIORITY, mount_graphql};
