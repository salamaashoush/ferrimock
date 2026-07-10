//! File object detection for template code generation
//!
//! Provides a pluggable extension point for detecting file objects in API
//! responses and extracting their file extensions. This enables the template
//! generator to produce appropriate fake file content (PDFs, images, etc.).

use super::types::ResponseStructure;
use crate::type_detector::ObjectAnalysis;
use std::sync::{Arc, Mutex};

/// Detects file objects in API responses and extracts file extensions.
///
/// Embedders register custom detectors for their API's file object patterns.
/// For example, an API that returns `{"type": "file", "extension": "pdf", ...}`
/// would register a detector that checks for the `type: "file"` pattern.
///
/// Closures with the right signature automatically implement this trait.
pub trait FileObjectDetector: Send + Sync + 'static {
    /// Extract file extension from response-level analysis.
    /// Returns None if this is not a file object.
    fn detect_from_response(&self, analysis: &ResponseStructure) -> Option<String>;

    /// Extract file extension from object-level analysis.
    /// `parent_extension` is the extension detected at a parent level, if any.
    /// Returns None if this is not a file object.
    fn detect_from_object(
        &self,
        analysis: &ObjectAnalysis,
        parent_extension: Option<&str>,
    ) -> Option<String>;
}

static FILE_DETECTORS: Mutex<Vec<Arc<dyn FileObjectDetector>>> = Mutex::new(Vec::new());

/// Register a custom file object detector for template code generation.
///
/// Must be called before consolidation or template generation runs.
///
/// # Example
///
/// ```rust,ignore
/// use ferrimock::codegen::register_file_object_detector;
/// use ferrimock::codegen::SimpleFileDetector;
///
/// // Detect objects with type: "file" and an "extension" field
/// register_file_object_detector(SimpleFileDetector::new("type", "file", "extension"));
/// ```
pub fn register_file_object_detector(detector: impl FileObjectDetector) {
    if let Ok(mut detectors) = FILE_DETECTORS.lock() {
        detectors.push(Arc::new(detector));
    }
}

/// A simple file object detector that checks for a specific field value
/// and extracts the extension from another field.
///
/// For example: `SimpleFileDetector::new("type", "file", "extension")`
/// detects objects where `type == "file"` and reads `extension` for the file type.
pub struct SimpleFileDetector {
    type_field: String,
    type_value: String,
    extension_field: String,
}

impl SimpleFileDetector {
    pub fn new(
        type_field: impl Into<String>,
        type_value: impl Into<String>,
        extension_field: impl Into<String>,
    ) -> Self {
        Self {
            type_field: type_field.into(),
            type_value: type_value.into(),
            extension_field: extension_field.into(),
        }
    }

    fn check_constants_and_extract(
        &self,
        constant_fields: &[(String, serde_json::Value)],
    ) -> Option<String> {
        let is_match = constant_fields.iter().any(|(name, value)| {
            name == &self.type_field && value.as_str() == Some(&self.type_value)
        });

        if is_match {
            constant_fields
                .iter()
                .find(|(name, _)| name == &self.extension_field)
                .and_then(|(_, value)| value.as_str())
                .map(std::string::ToString::to_string)
        } else {
            None
        }
    }
}

impl FileObjectDetector for SimpleFileDetector {
    fn detect_from_response(&self, analysis: &ResponseStructure) -> Option<String> {
        self.check_constants_and_extract(&analysis.constant_fields)
    }

    fn detect_from_object(
        &self,
        analysis: &ObjectAnalysis,
        parent_extension: Option<&str>,
    ) -> Option<String> {
        self.check_constants_and_extract(&analysis.constant_fields)
            .or_else(|| parent_extension.map(std::string::ToString::to_string))
    }
}

/// Try all registered file object detectors for response-level analysis.
pub(super) fn extract_file_extension_from_response(analysis: &ResponseStructure) -> Option<String> {
    let Ok(detectors) = FILE_DETECTORS.lock() else {
        return None;
    };
    for detector in detectors.iter() {
        if let Some(ext) = detector.detect_from_response(analysis) {
            return Some(ext);
        }
    }
    None
}

/// Try all registered file object detectors for object-level analysis.
pub(super) fn extract_file_extension(
    analysis: &ObjectAnalysis,
    parent_extension: Option<&str>,
) -> Option<String> {
    let Ok(detectors) = FILE_DETECTORS.lock() else {
        return parent_extension.map(std::string::ToString::to_string);
    };
    for detector in detectors.iter() {
        if let Some(ext) = detector.detect_from_object(analysis, parent_extension) {
            return Some(ext);
        }
    }
    parent_extension.map(std::string::ToString::to_string)
}
