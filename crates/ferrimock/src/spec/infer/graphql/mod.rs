//! GraphQL front end: SDL and introspection in, entity graph out.

pub mod defects;
pub mod entities;
pub mod sdl;

pub use defects::{DefectKind, SdlDefect};
pub use entities::to_entity_graph;
pub use sdl::{parse_sdl, parse_sdl_lenient};
