//! Template code generation utilities
//!
//! This crate provides utilities for converting field types (from `bdg-type-detector`)
//! into Tera template expressions for mock data generation.
//!
//! It includes:
//! - Basic field type to Tera expression conversion
//! - Complex template generation with pagination support
//! - Array and object template generation
//! - GraphQL variable integration
//!
//! This is used by both the consolidator and GraphQL mock generator.

pub mod array_object;
pub mod field_converter;
pub mod file_detection;
pub mod generator;
pub mod helpers;
pub mod pagination;
pub mod types;

pub use field_converter::{
    field_type_to_tera_expr, field_type_to_tera_expr_with_context, generate_data_uri_template,
    generate_download_url_template,
};
pub use file_detection::{FileObjectDetector, SimpleFileDetector, register_file_object_detector};
pub use generator::TemplateGenerator;
pub use types::{GraphQLVariableInfo, PaginationInfo, PaginationType, ResponseStructure};
