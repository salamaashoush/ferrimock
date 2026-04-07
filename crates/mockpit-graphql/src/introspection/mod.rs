//! GraphQL schema introspection
//!
//! This module provides functionality to:
//! - Standard GraphQL introspection query
//! - Parse introspection responses into structured types
//! - Define common GraphQL type structures
//! - Generate SDL from parsed schemas

pub mod parser;
pub mod query;
pub mod sdl;
pub mod types;

// Re-export main types
pub use parser::SchemaParser;
pub use query::get_introspection_query;
pub use sdl::generate_sdl;
pub use types::{
    EnumValueDefinition, FieldDefinition, InputValueDefinition, IntrospectionResponse,
    OperationType, ParsedSchema, TypeDefinition, TypeKind, TypeRef, UnwrappedType,
};
