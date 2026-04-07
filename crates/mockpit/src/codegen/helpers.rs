//! Helper utilities for template generation

use super::types::ResponseStructure;
use crate::type_detector::FieldType;

/// Detect which field is the pagination results array (results, items, data, etc.)
pub(super) fn detect_results_array_field(analysis: &ResponseStructure) -> Option<String> {
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
