//! Array and object template generation
//!
//! This module handles generation of Tera templates for complex types like arrays and objects.

use mockpit_type_detector::{ArrayPattern, FieldType, ObjectAnalysis};

use crate::field_converter::{field_type_to_tera_expr, field_type_to_tera_expr_with_context};
use crate::file_detection::extract_file_extension;

/// Generate Tera template for an array
///
/// # Edge Cases
/// The `sample_size_range` comes from analyzing actual API responses. Common edge cases:
/// - Arrays that are sometimes empty `[]` and sometimes have 1 item: `(min=0, max=1)`
/// - Arrays that always have exactly 1 item: `(min=1, max=1)`
///
/// We must ensure the generated `get_random(start, end)` call always has `start < end`
/// to avoid panics from rand's empty range check.
pub(super) fn generate_tera_array(pattern: &ArrayPattern) -> String {
  let (min, max) = pattern.sample_size_range;

  if !pattern.is_homogeneous {
    return "[]".to_string();
  }

  let element_expr = field_type_to_tera_expr("element", &pattern.element_type, false);

  let is_complex = matches!(pattern.element_type, FieldType::Object(_) | FieldType::Array(_));

  let (range_start, range_end) = if max > 0 && max > min {
    let start = min.max(1);
    let end = max.min(20);
    // Ensure end > start to avoid empty range panic in get_random
    // This happens when min=0, max=1 → start=1, end=1 which would cause:
    // "cannot sample empty range" panic in rand::random_range(1..1)
    if end <= start { (start, start + 1) } else { (start, end) }
  } else {
    (3, 5)
  };

  if is_complex {
    format!(
      "[\n        {{% for i in range(end=get_random(start={range_start}, end={range_end})) %}}\n        {element_expr}{{% if not loop.last %}},{{% endif %}}\n        {{% endfor %}}\n      ]"
    )
  } else {
    format!(
      "[{{% for i in range(end=get_random(start={range_start}, end={range_end})) %}}{element_expr}{{ if not loop.last }}, {{ endif }}{{% endfor %}}]"
    )
  }
}

/// Generate Tera array template that uses `limit` for pagination results
pub(super) fn generate_tera_array_with_limit(pattern: &ArrayPattern) -> String {
  if !pattern.is_homogeneous {
    return "[]".to_string();
  }

  let element_expr = field_type_to_tera_expr("element", &pattern.element_type, false);
  let is_complex = matches!(pattern.element_type, FieldType::Object(_) | FieldType::Array(_));

  // Use limit variable for pagination results array
  if is_complex {
    format!(
      "[\n        {{% for i in range(end=limit) %}}\n        {element_expr}{{% if not loop.last %}},{{% endif %}}\n        {{% endfor %}}\n      ]"
    )
  } else {
    format!("[{{% for i in range(end=limit) %}}{element_expr}{{ if not loop.last }}, {{ endif }}{{% endfor %}}]")
  }
}

/// Generate Tera template for an object with file extension context
pub(super) fn generate_tera_object_with_extension(
  analysis: &ObjectAnalysis,
  has_matching_path_ids: bool,
  parent_extension: Option<&str>,
  graphql_analysis: &crate::types::GraphQLVariableInfo,
) -> String {
  if analysis.varying_fields.is_empty() && analysis.constant_fields.is_empty() {
    return "{}".to_string();
  }

  let mut fields = Vec::new();

  // Detect file extension from this object or parent context
  let file_extension = extract_file_extension(analysis, parent_extension);

  for (field, field_type) in &analysis.varying_fields {
    // Check if this field matches a GraphQL variable first
    let graphql_var_expr = crate::generator::try_graphql_variable_expression_for_nested(field, graphql_analysis);

    let expr = if let Some(gql_expr) = graphql_var_expr {
      // Use GraphQL variable extraction
      gql_expr
    } else {
      match field_type {
        FieldType::DownloadUrl { .. } if file_extension.is_some() => {
          // For download URLs, use file extension context
          field_type_to_tera_expr_with_context(field, field_type, has_matching_path_ids, file_extension.as_deref())
        },
        // Handle URLs that should be download URLs in Box file objects (e.g., authenticated_download_url)
        FieldType::Url
          if file_extension.is_some() && (field.contains("download_url") || field.contains("download")) =>
        {
          // Treat as download URL with file extension
          field_type_to_tera_expr_with_context(
            field,
            &FieldType::DownloadUrl { sample_url: None },
            has_matching_path_ids,
            file_extension.as_deref(),
          )
        },
        FieldType::Object(nested_analysis) if file_extension.is_some() => {
          // Pass extension context and GraphQL analysis to nested objects
          generate_tera_object_with_extension(
            nested_analysis,
            has_matching_path_ids,
            file_extension.as_deref(),
            graphql_analysis,
          )
        },
        FieldType::Object(nested_analysis) => {
          // Pass GraphQL analysis to nested objects even without extension
          generate_tera_object_with_extension(nested_analysis, has_matching_path_ids, None, graphql_analysis)
        },
        FieldType::Array(array_pattern) => generate_tera_array(array_pattern),
        _ => field_type_to_tera_expr(field, field_type, has_matching_path_ids),
      }
    };
    fields.push(format!("\"{field}\": {expr}"));
  }

  for (field, value) in &analysis.constant_fields {
    let value_str = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    fields.push(format!("\"{field}\": {value_str}"));
  }

  let fields_str = fields.join(", ");
  format!("{{{fields_str}}}")
}

#[cfg(test)]
mod tests {
  use super::*;
  use mockpit_type_detector::{ArrayPattern, FieldType};

  #[test]
  fn test_generate_tera_array_prevents_empty_range() {
    // Test case that previously caused panic: min=0, max=1
    // This would result in start=1, end=1 which causes get_random to panic
    let pattern = ArrayPattern {
      element_type: FieldType::RandomString,
      is_homogeneous: true,
      sample_size_range: (0, 1),
    };

    let template = generate_tera_array(&pattern);

    // Template should contain get_random with start < end
    assert!(
      template.contains("get_random(start=1, end=2)"),
      "Expected get_random(start=1, end=2) but got: {template}"
    );
  }

  #[test]
  fn test_generate_tera_array_single_element() {
    // Another edge case: array always has exactly 1 element
    let pattern = ArrayPattern {
      element_type: FieldType::RandomString,
      is_homogeneous: true,
      sample_size_range: (1, 1),
    };

    let template = generate_tera_array(&pattern);

    // Should fall through to default (3, 5) since max is not > min
    assert!(
      template.contains("get_random(start=3, end=5)"),
      "Expected get_random(start=3, end=5) but got: {template}"
    );
  }

  #[test]
  fn test_generate_tera_array_normal_range() {
    // Normal case: valid range
    let pattern = ArrayPattern {
      element_type: FieldType::RandomString,
      is_homogeneous: true,
      sample_size_range: (2, 10),
    };

    let template = generate_tera_array(&pattern);

    // Should use the provided range (clamped to min=1)
    assert!(
      template.contains("get_random(start=2, end=10)"),
      "Expected get_random(start=2, end=10) but got: {template}"
    );
  }

  #[test]
  fn test_generate_tera_array_zero_min() {
    // Edge case: min=0 should be clamped to 1
    let pattern = ArrayPattern {
      element_type: FieldType::RandomString,
      is_homogeneous: true,
      sample_size_range: (0, 5),
    };

    let template = generate_tera_array(&pattern);

    // min should be clamped to 1
    assert!(
      template.contains("get_random(start=1, end=5)"),
      "Expected get_random(start=1, end=5) but got: {template}"
    );
  }

  #[test]
  fn test_generate_tera_array_large_max() {
    // Edge case: max > 20 should be clamped to 20
    let pattern = ArrayPattern {
      element_type: FieldType::RandomString,
      is_homogeneous: true,
      sample_size_range: (5, 100),
    };

    let template = generate_tera_array(&pattern);

    // max should be clamped to 20
    assert!(
      template.contains("get_random(start=5, end=20)"),
      "Expected get_random(start=5, end=20) but got: {template}"
    );
  }

  #[test]
  fn test_generate_tera_array_complex_type() {
    // Test with complex type (object) - should use multi-line format
    let pattern = ArrayPattern {
      element_type: FieldType::Object(Box::new(mockpit_type_detector::ObjectAnalysis {
        varying_fields: vec![],
        constant_fields: vec![],
      })),
      is_homogeneous: true,
      sample_size_range: (1, 5),
    };

    let template = generate_tera_array(&pattern);

    // Should use multi-line format with newlines
    assert!(template.contains("[\n"), "Expected multi-line format for complex type");
    assert!(
      template.contains("get_random(start=1, end=5)"),
      "Expected get_random(start=1, end=5) but got: {template}"
    );
  }
}
