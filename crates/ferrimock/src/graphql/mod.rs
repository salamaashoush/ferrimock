//! Reading a GraphQL schema.
//!
//! Introspection over the wire, the response parsed into a
//! [`crate::graphql::introspection::ParsedSchema`],
//! and SDL written back out of one. What a schema *means* — which types have
//! identity, how they link — is [`crate::spec::infer::graphql`]; what it
//! *serves* is [`crate::spec::bind::graphql`]. This module only reads.
//!
//! ## Example
//!
//! ```rust,no_run
//! use ferrimock::graphql::{SchemaParser, generate_sdl, get_introspection_query};
//! use ferrimock::Result;
//!
//! fn print_sdl(response: ferrimock::graphql::IntrospectionResponse) -> Result<()> {
//!     let schema = SchemaParser::parse(response)?;
//!     println!("{}", generate_sdl(&schema));
//!     Ok(())
//! }
//! ```

pub mod introspection;

// Re-export introspection types
pub use introspection::{
    EnumValueDefinition, FieldDefinition, InputValueDefinition, IntrospectionResponse,
    OperationType, ParsedSchema, SchemaParser, TypeDefinition, TypeKind, TypeRef, UnwrappedType,
    generate_sdl, get_introspection_query,
};
