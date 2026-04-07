//! Field type to Tera expression conversion
//!
//! This module handles the conversion of FieldType enums to Tera template expressions,
//! including support for complex types like arrays and objects with GraphQL context.

use mockpit_type_detector::FieldType;

/// Convert a field type to a Tera template expression
///
/// This function generates Tera template syntax from a detected field type.
/// It handles all field types including complex arrays and objects.
///
/// # Arguments
///
/// * `field_name` - Name of the field being converted
/// * `field_type` - The detected field type
/// * `has_matching_path_ids` - Whether the request has path IDs that should be captured
///
/// # Returns
///
/// A string containing the Tera template expression for this field type
pub fn field_type_to_tera_expr(field_name: &str, field_type: &FieldType, has_matching_path_ids: bool) -> String {
  match field_type {
    // Complex types with special handling
    FieldType::Array(pattern) => crate::array_object::generate_tera_array(pattern),
    FieldType::Object(analysis) => {
      let empty_graphql_analysis = crate::types::GraphQLVariableInfo::empty();
      crate::array_object::generate_tera_object_with_extension(
        analysis,
        has_matching_path_ids,
        None,
        &empty_graphql_analysis,
      )
    },

    // Numeric types
    FieldType::SequentialNumber { .. } if has_matching_path_ids && field_name == "id" => {
      "{{ captures.id }}".to_string()
    },
    FieldType::SequentialNumber { start, .. } => {
      // Ensure end > start to avoid empty range panic
      let end = if *start >= 1000 { start + 100 } else { 1000 };
      format!("{{{{ get_random(start={start}, end={end}) }}}}")
    },
    FieldType::RandomNumber { min, max } => {
      let start = min.unwrap_or(1);
      let mut end = max.unwrap_or(1000);
      // Ensure end > start to avoid empty range panic
      if end <= start {
        end = start + 100;
      }
      format!("{{{{ get_random(start={start}, end={end}) }}}}")
    },
    FieldType::RandomFloat { min, max } => {
      let start = min.unwrap_or(0.0);
      let mut end = max.unwrap_or(1000.0);
      // Ensure end > start to avoid empty range panic
      if end <= start {
        end = start + 100.0;
      }
      format!("{{{{ get_random(start={start}, end={end}) }}}}")
    },
    FieldType::UnixTimestamp => "{{ fake_unix_timestamp() }}".to_string(),
    FieldType::MillisecondTimestamp => "{{ fake_unix_timestamp() * 1000 }}".to_string(),
    FieldType::MicrosecondTimestamp => "{{ fake_unix_timestamp() * 1000000 }}".to_string(),

    // ID and identifier types
    FieldType::Uuid => "\"{{ uuid() }}\"".to_string(),
    FieldType::NumericStringId => "\"{{ fake_numeric_id() }}\"".to_string(),
    FieldType::RandomString => "\"{{ fake_alphanumeric(length=10) }}\"".to_string(),
    FieldType::Token => "\"{{ fake_token() }}\"".to_string(),
    FieldType::ETag => "\"{{ fake_etag() }}\"".to_string(),
    FieldType::HexString => "\"{{ fake_md5() }}\"".to_string(),
    FieldType::Base64 => "\"{{ fake_base64() }}\"".to_string(),

    // Date/time types
    FieldType::Timestamp => "\"{{ now() }}\"".to_string(),
    FieldType::IsoDate => "\"{{ fake_iso_date() }}\"".to_string(),

    // Person/contact types
    FieldType::Email => "\"{{ fake_email() }}\"".to_string(),
    FieldType::Username => "\"{{ fake_username() }}\"".to_string(),
    FieldType::Name => "\"{{ fake_name() }}\"".to_string(),
    FieldType::PhoneNumber => "\"{{ fake_phone() }}\"".to_string(),

    // Text types
    FieldType::Sentence => "\"{{ fake_sentence(word_count=8) }}\"".to_string(),
    FieldType::Paragraph => "\"{{ fake_paragraph(sentence_count=3) }}\"".to_string(),

    // Network types
    FieldType::Url => "\"{{ fake_url() }}\"".to_string(),
    FieldType::IpAddress => "\"{{ fake_ipv4() }}\"".to_string(),
    FieldType::ApiEndpoint => "\"{{ fake_api_endpoint() }}\"".to_string(),

    // File types
    FieldType::FileName => "\"{{ fake_filename() }}\"".to_string(),
    FieldType::FileSize => "{{ fake_file_size(min=1000, max=10000000) }}".to_string(),
    FieldType::FilePath => "\"{{ fake_file_path() }}\"".to_string(),
    FieldType::MimeType => "\"{{ fake_mime_type() }}\"".to_string(),
    FieldType::ImageUrl => "\"{{ fake_png_data_uri() }}\"".to_string(),
    FieldType::DownloadUrl { sample_url } => generate_download_url_template(sample_url.as_deref()),
    FieldType::DataUri { mime_type } => generate_data_uri_template(mime_type.as_deref()),

    // Location types
    FieldType::Latitude => "{{ fake_latitude() }}".to_string(),
    FieldType::Longitude => "{{ fake_longitude() }}".to_string(),
    FieldType::CountryCode => "\"{{ fake_country_code() }}\"".to_string(),
    FieldType::PostalCode => "\"{{ fake_postal_code() }}\"".to_string(),
    FieldType::LocaleCode => "\"{{ fake_locale() }}\"".to_string(),
    FieldType::Timezone => "\"{{ fake_timezone() }}\"".to_string(),
    FieldType::CurrencyCode => "\"{{ fake_currency_code() }}\"".to_string(),

    // Version/semantic types
    FieldType::Semver => "\"{{ fake_semver() }}\"".to_string(),

    // Boolean type
    FieldType::Boolean => "{{ fake_boolean() }}".to_string(),

    // Constant value
    FieldType::Constant(value) => serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),

    // Categorical/enum type
    FieldType::Categorical { values } => {
      if values.is_empty() {
        "\"{{ uuid() }}\"".to_string()
      } else {
        let args = values
          .iter()
          .map(|v| {
            let escaped = v.replace('\"', "\\\"");
            format!("\"{escaped}\"")
          })
          .collect::<Vec<_>>()
          .join(", ");
        format!("\"{{{{ [{args}] | random_choice }}}}\"")
      }
    },

    // Pagination URL
    FieldType::PaginationUrl(pattern) => {
      use mockpit_type_detector::PaginationScheme;

      let static_qs = pattern
        .static_params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");

      let dynamic_part = match &pattern.pagination_scheme {
        PaginationScheme::PageBased {
          page_key,
          limit_key,
          sample_limit,
          ..
        } => {
          let mut parts = vec![format!("{}={{{{ get_random(start=1, end=100) }}}}", page_key)];
          if let (Some(lk), Some(sl)) = (limit_key, sample_limit) {
            parts.push(format!("{lk}={sl}"));
          }
          parts.join("&")
        },
        PaginationScheme::CursorBased { cursor_key, .. } => {
          format!("{cursor_key}={{{{ fake_token() }}}}")
        },
      };

      let sep = if static_qs.is_empty() { "" } else { "&" };
      format!(
        "\"{}?{}{}{}\"",
        pattern.base_url.trim_end_matches('?'),
        static_qs,
        sep,
        dynamic_part
      )
    },
  }
}

/// Generate Tera expression for a field with additional context (e.g., file extension)
///
/// This is used when we have additional context that can improve the template generation
pub fn field_type_to_tera_expr_with_context(
  field_name: &str,
  field_type: &FieldType,
  has_matching_path_ids: bool,
  extension: Option<&str>,
) -> String {
  match field_type {
    FieldType::DownloadUrl { sample_url } => {
      // Use extension context if available, otherwise use sample URL
      let file_type = extension.or_else(|| {
        sample_url.as_ref().and_then(|url| {
          let url_lower = url.to_lowercase();
          if url_lower.contains(".pdf") || url_lower.contains("pdf") {
            Some("pdf")
          } else if url_lower.contains(".png") {
            Some("png")
          } else if url_lower.contains(".jpg") || url_lower.contains(".jpeg") {
            Some("jpeg")
          } else {
            None
          }
        })
      });

      match file_type {
        Some("pdf") => "\"{{ fake_pdf_data_uri() }}\"".to_string(),
        Some("png") => "\"{{ fake_png_data_uri() }}\"".to_string(),
        Some("jpeg" | "jpg") => "\"{{ fake_jpeg_data_uri() }}\"".to_string(),
        _ => "\"{{ fake_download_url() }}\"".to_string(),
      }
    },
    _ => field_type_to_tera_expr(field_name, field_type, has_matching_path_ids),
  }
}

/// Detect file type from URL and generate appropriate template
pub fn generate_download_url_template(sample_url: Option<&str>) -> String {
  if let Some(url) = sample_url {
    let url_lower = url.to_lowercase();

    // Check for PDF
    if url_lower.contains(".pdf") || url_lower.contains("pdf") {
      return "\"{{ fake_pdf_data_uri() }}\"".to_string();
    }

    // Check for PNG
    if url_lower.contains(".png") {
      return "\"{{ fake_png_data_uri() }}\"".to_string();
    }

    // Check for JPEG/JPG
    if url_lower.contains(".jpg") || url_lower.contains(".jpeg") {
      return "\"{{ fake_jpeg_data_uri() }}\"".to_string();
    }
  }

  // Default to fake_download_url if no file type detected
  "\"{{ fake_download_url() }}\"".to_string()
}

/// Generate data URI template based on detected mime type
pub fn generate_data_uri_template(mime_type: Option<&str>) -> String {
  if let Some(mime) = mime_type {
    let mime_lower = mime.to_lowercase();

    // Check for PNG
    if mime_lower.contains("image/png") {
      return "\"{{ fake_png_data_uri() }}\"".to_string();
    }

    // Check for JPEG
    if mime_lower.contains("image/jpeg") || mime_lower.contains("image/jpg") {
      return "\"{{ fake_jpeg_data_uri() }}\"".to_string();
    }

    // Check for PDF
    if mime_lower.contains("application/pdf") {
      return "\"{{ fake_pdf_data_uri() }}\"".to_string();
    }
  }

  // Default to PNG data URI if no mime type detected or unknown type
  "\"{{ fake_png_data_uri() }}\"".to_string()
}

#[cfg(test)]
mod tests {
  use super::*;
  use mockpit_type_detector::FieldType;

  #[test]
  fn test_random_number_equal_min_max() {
    // Edge case: min == max should not create empty range
    let field_type = FieldType::RandomNumber {
      min: Some(5),
      max: Some(5),
    };

    let template = field_type_to_tera_expr("test_field", &field_type, false);

    // Should generate end > start
    assert!(
      template.contains("get_random(start=5, end=105)"),
      "Expected get_random(start=5, end=105) but got: {template}"
    );
  }

  #[test]
  fn test_random_number_max_less_than_min() {
    // Edge case: max < min should not create empty range
    let field_type = FieldType::RandomNumber {
      min: Some(100),
      max: Some(50),
    };

    let template = field_type_to_tera_expr("test_field", &field_type, false);

    // Should generate end > start (start=100, end=200)
    assert!(
      template.contains("get_random(start=100, end=200)"),
      "Expected get_random(start=100, end=200) but got: {template}"
    );
  }

  #[test]
  fn test_random_float_equal_min_max() {
    // Edge case: min == max should not create empty range
    let field_type = FieldType::RandomFloat {
      min: Some(2.5),
      max: Some(2.5),
    };

    let template = field_type_to_tera_expr("test_field", &field_type, false);

    // Should generate end > start
    assert!(
      template.contains("get_random(start=2.5, end=102.5)"),
      "Expected get_random(start=2.5, end=102.5) but got: {template}"
    );
  }

  #[test]
  fn test_sequential_number_large_start() {
    // Edge case: start >= 1000 should not create empty range
    let field_type = FieldType::SequentialNumber { start: 1000, step: 1 };

    let template = field_type_to_tera_expr("test_field", &field_type, false);

    // Should generate end > start (start=1000, end=1100)
    assert!(
      template.contains("get_random(start=1000, end=1100)"),
      "Expected get_random(start=1000, end=1100) but got: {template}"
    );
  }

  #[test]
  fn test_sequential_number_normal_range() {
    // Normal case: start < 1000
    let field_type = FieldType::SequentialNumber { start: 1, step: 1 };

    let template = field_type_to_tera_expr("test_field", &field_type, false);

    // Should use default end=1000
    assert!(
      template.contains("get_random(start=1, end=1000)"),
      "Expected get_random(start=1, end=1000) but got: {template}"
    );
  }

  #[test]
  fn test_random_number_normal_range() {
    // Normal case: valid range
    let field_type = FieldType::RandomNumber {
      min: Some(1),
      max: Some(100),
    };

    let template = field_type_to_tera_expr("test_field", &field_type, false);

    // Should use the provided range
    assert!(
      template.contains("get_random(start=1, end=100)"),
      "Expected get_random(start=1, end=100) but got: {template}"
    );
  }
}
