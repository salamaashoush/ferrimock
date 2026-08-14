//! Helper utilities for template generation

use super::types::ResponseStructure;
use crate::type_detector::FieldType;

/// Detect which field is the pagination results array (results, items, data, etc.)
///
/// Only meaningful when the response actually paginates. The array this names is
/// sized by the `limit` variable, and `limit` exists only because the pagination
/// preamble binds it -- so naming an array here without pagination emits a
/// template that references an undefined variable and fails to render. Plenty of
/// unpaginated endpoints answer with an `entries` or `data` array; a search POST
/// returning `{"total_count": n, "entries": [...]}` is the ordinary case.
pub(super) fn detect_results_array_field(analysis: &ResponseStructure) -> Option<String> {
    analysis.pagination.as_ref()?;

    // Common field names for pagination results arrays
    let result_field_names = [
        "results", "items", "data", "entries", "records", "list", "objects",
    ];

    for (field, field_type) in &analysis.varying_fields {
        if result_field_names.contains(&field.as_str()) && matches!(field_type, FieldType::Array(_))
        {
            return Some(field.clone());
        }
    }
    None
}
