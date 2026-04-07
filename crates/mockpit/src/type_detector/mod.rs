//! Sophisticated type detection system for mock consolidation
//!
//! This module implements a multi-layered type detection algorithm based on research from:
//! - BigQuery Schema Auto-Detection
//! - Sherlock (MIT) - Deep Learning for Semantic Type Detection
//! - Sato (VLDB 2020) - Contextual Semantic Type Detection
//!
//! ## Detection Layers
//!
//! 1. **Semantic Context Analysis** - Field names provide strong hints
//! 2. **Statistical Feature Extraction** - Character distributions, entropy, length statistics
//! 3. **Priority-Ordered Pattern Matching** - Specific to general type detection
//! 4. **Multi-Sample Validation** - Confidence scoring across multiple values
//!
//! ## Usage
//!
//! ```rust
//! use mockpit::type_detector::TypeDetector;
//! use serde_json::json;
//!
//! let detector = TypeDetector::new();
//! let values = vec![json!("test@example.com"), json!("user@domain.org")];
//! let value_refs: Vec<&serde_json::Value> = values.iter().collect();
//! let (field_type, confidence) = detector.detect_type("email", &value_refs);
//! ```

pub mod analyzers;
pub mod checkers;
#[allow(clippy::expect_used)]
// Static regex initialization - panics are appropriate for invalid compile-time patterns
pub mod constants;
pub mod features;
pub mod semantic;
pub mod types;

// Re-export public types
pub use features::TypeFeatures;
pub use semantic::{detect_from_field_name_only, detect_from_semantic_context};
pub use types::{
    ArrayPattern, FieldType, ObjectAnalysis, PaginationDirection, PaginationScheme,
    PaginationUrlPattern,
};

use serde_json::Value as JsonValue;

use analyzers::{analyze_array_pattern, analyze_numbers, analyze_object_pattern};
use checkers::get_checkers;
use constants::DATA_URI_REGEX;
use features::{check_categorical, extract_features};
use semantic::calculate_semantic_boost;

/// Main type detection engine (zero-sized type)
pub struct TypeDetector;

impl TypeDetector {
    /// Create a new type detector (zero-sized struct)
    pub fn new() -> Self {
        Self
    }

    /// Detect type with confidence score using field name context
    ///
    /// # Arguments
    /// * `field_name` - The name of the field (provides semantic context)
    /// * `values` - Sample values from the field
    ///
    /// # Returns
    /// Tuple of (detected type, confidence score 0.0-1.0)
    pub fn detect_type(&self, field_name: &str, values: &[&JsonValue]) -> (FieldType, f64) {
        if values.is_empty() {
            return (FieldType::RandomString, 0.5);
        }

        // Layer 1: Semantic context from field name (strong hints return immediately)
        if let Some(result) = detect_from_semantic_context(field_name, values) {
            return result;
        }

        // Layer 2-4: Pattern-based detection with weighted scoring
        let (field_type, base_confidence) = self.detect_type_from_values(values);

        // Apply semantic boost based on field name
        let boost = calculate_semantic_boost(field_name, &field_type);
        let boosted_confidence = (base_confidence * (1.0 + boost)).clamp(0.0, 1.0);

        (field_type, boosted_confidence)
    }

    /// Detect type without field name context (pattern-based only)
    pub fn detect_type_from_values(&self, values: &[&JsonValue]) -> (FieldType, f64) {
        if values.is_empty() {
            return (FieldType::RandomString, 0.5);
        }

        // Check for JSON primitive types first
        if values.iter().all(|v| v.is_number()) {
            return analyze_numbers(values);
        }

        if values.iter().all(|v| v.is_boolean()) {
            return (FieldType::Boolean, 1.0);
        }

        if values.iter().all(|v| v.is_array()) {
            return analyze_array_pattern(values, |vals| self.detect_type_from_values(vals));
        }

        if values.iter().all(|v| v.is_object()) {
            return analyze_object_pattern(values, |name, vals| self.detect_type(name, vals));
        }

        // String type detection - extract string values
        let strings: Option<Vec<&str>> = values.iter().map(|v| v.as_str()).collect();

        if let Some(strs) = strings {
            // Check for categorical/enum before feature extraction (low cardinality)
            if let Some(categorical) = check_categorical(&strs) {
                return categorical;
            }

            // Extract statistical features
            let features = extract_features(&strs);

            // Run priority-ordered pattern detection
            detect_from_patterns(values, &features)
        } else {
            (FieldType::RandomString, 0.5)
        }
    }
}

/// Layer 3 & 4: Priority-ordered pattern matching with multi-sample validation
/// Now uses weighted scoring to collect all potential types
fn detect_from_patterns(
    values: &[&JsonValue],
    features: &features::TypeFeatures,
) -> (FieldType, f64) {
    let strings: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();

    if strings.is_empty() {
        return (FieldType::RandomString, 0.5);
    }

    // Collect all potential types with their scores
    let mut potential_types: Vec<(FieldType, f64)> = Vec::new();

    for checker in get_checkers() {
        if let Some(confidence) = (checker.checker_fn)(&strings, features)
            && confidence >= checker.threshold
        {
            potential_types.push((checker.field_type.clone(), confidence));
        }
    }

    // If no types passed threshold, return default
    if potential_types.is_empty() {
        return (FieldType::RandomString, 0.5);
    }

    // Return the type with highest confidence
    let (field_type, confidence) = potential_types
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((FieldType::RandomString, 0.5));

    // For DownloadUrl, add sample URL for file type detection
    if matches!(field_type, FieldType::DownloadUrl { .. }) {
        let sample_url = strings
            .iter()
            .find(|s| s.len() > 100)
            .map(|s| (*s).to_string());
        return (FieldType::DownloadUrl { sample_url }, confidence);
    }

    // For DataUri, extract mime type for smart generation
    if matches!(field_type, FieldType::DataUri { .. }) {
        let mime_type = strings.iter().find_map(|s| {
            DATA_URI_REGEX
                .captures(s)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
        });
        return (FieldType::DataUri { mime_type }, confidence);
    }

    (field_type, confidence)
}

impl Default for TypeDetector {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// URL Classifier extension point
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

/// Classifies URLs for type detection (e.g., identifying download URLs).
///
/// Embedders can register custom classifiers to recognize domain-specific
/// download URL patterns that the built-in heuristics don't cover.
///
/// Closures with signature `Fn(&str) -> bool` automatically implement this trait.
pub trait UrlClassifier: Send + Sync + 'static {
    /// Returns true if the given URL is a download URL.
    fn is_download_url(&self, url: &str) -> bool;
}

impl<F> UrlClassifier for F
where
    F: Fn(&str) -> bool + Send + Sync + 'static,
{
    fn is_download_url(&self, url: &str) -> bool {
        self(url)
    }
}

static URL_CLASSIFIERS: Mutex<Vec<Arc<dyn UrlClassifier>>> = Mutex::new(Vec::new());

/// Register a custom URL classifier for type detection.
///
/// The classifier will be consulted when detecting download URL fields.
/// Must be called before consolidation or type detection runs.
///
/// # Example
///
/// ```rust,ignore
/// mockpit::type_detector::register_url_classifier(|url| {
///     url.contains("dl.mycdn.com") || url.contains("files.myservice.com")
/// });
/// ```
pub fn register_url_classifier(classifier: impl UrlClassifier) {
    if let Ok(mut classifiers) = URL_CLASSIFIERS.lock() {
        classifiers.push(Arc::new(classifier));
    }
}

/// Check if any registered custom classifier recognizes this as a download URL.
/// Called by checkers and semantic modules.
pub(crate) fn is_custom_download_url(url: &str) -> bool {
    let Ok(classifiers) = URL_CLASSIFIERS.lock() else {
        return false;
    };
    classifiers.iter().any(|c| c.is_download_url(url))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::uninlined_format_args,
    clippy::float_cmp,
    clippy::match_wildcard_for_single_variants,
    clippy::manual_string_new
)]
mod tests;
