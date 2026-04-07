//! File extension detection and context handling for Box file objects

use super::types::ResponseStructure;
use crate::type_detector::ObjectAnalysis;

/// Extract file extension from Box file objects (ResponseStructure)
pub(super) fn extract_file_extension_from_response(analysis: &ResponseStructure) -> Option<String> {
    // Check if this is a Box file object (has type: "file")
    let is_file_object = analysis
        .constant_fields
        .iter()
        .any(|(name, value)| name == "type" && value.as_str() == Some("file"));

    if is_file_object {
        // Look for extension in constant fields
        if let Some(ext) = analysis
            .constant_fields
            .iter()
            .find(|(name, _)| name == "extension")
            .and_then(|(_, value)| value.as_str())
            .map(std::string::ToString::to_string)
        {
            return Some(ext);
        }
    }

    None
}

/// Extract file extension from Box file objects (ObjectAnalysis)
/// Looks for extension field in objects that have type: "file"
pub(super) fn extract_file_extension(
    analysis: &ObjectAnalysis,
    parent_extension: Option<&str>,
) -> Option<String> {
    // Check if this is a Box file object (has type: "file")
    let is_file_object = analysis
        .constant_fields
        .iter()
        .any(|(name, value)| name == "type" && value.as_str() == Some("file"));

    if is_file_object {
        // Look for extension in constant fields
        if let Some(ext) = analysis
            .constant_fields
            .iter()
            .find(|(name, _)| name == "extension")
            .and_then(|(_, value)| value.as_str())
            .map(std::string::ToString::to_string)
        {
            return Some(ext);
        }
    }

    // Fall back to parent extension if available
    parent_extension.map(std::string::ToString::to_string)
}
