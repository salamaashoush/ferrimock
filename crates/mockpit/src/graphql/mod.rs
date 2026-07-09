//! GraphQL schema introspection and mock generation
//!
//! This crate provides functionality to generate mock configurations from GraphQL schemas
//! by analyzing introspection queries and mapping GraphQL types to fake data templates.
//!
//! ## Features
//!
//! - GraphQL schema introspection query and response parsing
//! - SDL (Schema Definition Language) generation from parsed schemas
//! - Generate mocks from GraphQL operations (queries, mutations, subscriptions)
//! - Map GraphQL types to appropriate fake data generators
//! - Support for nested types, lists, and custom scalars
//! - Configurable mock generation options
//!
//! ## Example
//!
//! ```rust,no_run
//! use mockpit::graphql::{MockGenerator, GeneratorOptions, ParsedSchema};
//! use mockpit::Result;
//!
//! fn generate_mocks(schema: ParsedSchema) -> Result<()> {
//!     let options = GeneratorOptions {
//!         endpoint_url: "/graphql".to_string(),
//!         ..Default::default()
//!     };
//!
//!     let generator = MockGenerator::new(schema, options);
//!     let collection = generator.generate_all()?;
//!
//!     println!("Generated {} mocks", collection.mocks.len());
//!     Ok(())
//! }
//! ```

pub mod generator;
pub mod introspection;
pub mod type_mapper;

// Re-export introspection types
pub use introspection::{
    EnumValueDefinition, FieldDefinition, InputValueDefinition, IntrospectionResponse,
    OperationType, ParsedSchema, SchemaParser, TypeDefinition, TypeKind, TypeRef, UnwrappedType,
    generate_sdl, get_introspection_query,
};

// Re-export mock-specific types
pub use generator::{GeneratorOptions, MockGenerator};
pub use type_mapper::TypeToFakeMapper;
