//! Template codegen types
//!
//! These types are independent of the consolidator and define the interface
//! for template generation. The consolidator maps its analysis types to these.

use crate::type_detector::FieldType;
use serde_json::Value as JsonValue;

/// Pagination pattern for template generation
#[derive(Debug, Clone)]
pub struct PaginationInfo {
    /// Total count field name (e.g., "total_count", "total")
    pub total_field: Option<String>,
    /// Offset field name (e.g., "offset", "skip")
    pub offset_field: Option<String>,
    /// Limit field name (e.g., "limit", "per_page")
    pub limit_field: Option<String>,
    /// Next marker/cursor field (e.g., "next_marker")
    pub next_field: Option<String>,
    /// Previous marker/cursor field (e.g., "prev_marker")
    pub prev_field: Option<String>,
    /// Has more field (e.g., "has_more")
    pub has_more_field: Option<String>,
    /// Sample total value for default
    pub sample_total: Option<i64>,
    /// Pagination type
    pub pagination_type: PaginationType,
    /// Static query parameters
    pub static_query_params: String,
}

/// Type of pagination
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationType {
    Offset,
    Cursor,
    Page,
}

/// Response structure analysis for template generation
#[derive(Debug)]
pub struct ResponseStructure {
    /// Fields that vary across responses
    pub varying_fields: Vec<(String, FieldType)>,
    /// Fields that are constant
    pub constant_fields: Vec<(String, JsonValue)>,
    /// Whether response IDs match path IDs
    pub has_matching_path_ids: bool,
    /// Whether response is JSON
    pub is_json: bool,
    /// Top-level type (object, array, etc.)
    pub top_level_type: String,
    /// Pagination information if detected
    pub pagination: Option<PaginationInfo>,
}

/// GraphQL variable analysis for template generation
#[derive(Debug, Clone)]
#[allow(clippy::struct_field_names)]
// Field names intentionally use `_variables` suffix to clearly distinguish between
// varying_variables and constant_variables, maintaining semantic clarity in GraphQL context
pub struct GraphQLVariableInfo {
    /// Variables that vary across mocks (e.g., `["id", "input.role"]`)
    pub varying_variables: Vec<String>,
    /// Variables that are constant with their values
    pub constant_variables: Vec<(String, JsonValue)>,
    /// Whether any variables exist
    pub has_variables: bool,
    /// Whether there are varying variables
    pub has_varying_variables: bool,
}

impl GraphQLVariableInfo {
    /// Create an empty analysis (for non-GraphQL)
    pub fn empty() -> Self {
        Self {
            varying_variables: vec![],
            constant_variables: vec![],
            has_variables: false,
            has_varying_variables: false,
        }
    }
}
