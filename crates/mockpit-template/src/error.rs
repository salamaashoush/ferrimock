//! Template error handling and diagnostics

use mockpit_core::levenshtein_distance;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Compiled regexes for extracting line/column/function from error messages
static LINE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"line (\d+)").expect("valid regex"));
static COLUMN_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"column (\d+)").expect("valid regex"));
static FUNCTION_NAME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"function ['`](\w+)['`]").expect("valid regex"));

/// Structured error type for template rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateError {
    /// Error type: "parse" | "render" | "function"
    pub error_type: String,
    /// Error message
    pub message: String,
    /// Template excerpt showing context around error
    pub template_excerpt: Option<String>,
    /// Line number where error occurred (if available)
    pub line: Option<usize>,
    /// Column number where error occurred (if available)
    pub column: Option<usize>,
    /// Mock ID that triggered this error
    pub mock_id: Option<String>,
    /// Suggestions for fixing the error
    pub suggestions: Vec<String>,
}

impl TemplateError {
    /// Create a new template error
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error_type: error_type.into(),
            message: message.into(),
            template_excerpt: None,
            line: None,
            column: None,
            mock_id: None,
            suggestions: Vec::new(),
        }
    }

    /// Add template excerpt
    pub fn with_excerpt(mut self, template: &str, line: Option<usize>) -> Self {
        if let Some(line_num) = line {
            // Show 2 lines before and after the error
            let lines: Vec<&str> = template.lines().collect();
            let start = line_num.saturating_sub(2);
            let end = (line_num + 3).min(lines.len());

            let mut excerpt = String::new();
            for (i, line) in lines[start..end].iter().enumerate() {
                let actual_line = start + i + 1;
                if actual_line == line_num {
                    excerpt.push_str(&format!("→ {}: {}\n", actual_line, line));
                } else {
                    excerpt.push_str(&format!("  {}: {}\n", actual_line, line));
                }
            }
            self.template_excerpt = Some(excerpt);
            self.line = Some(line_num);
        } else {
            // Just show first few lines if no line number
            let lines: Vec<&str> = template.lines().take(5).collect();
            self.template_excerpt = Some(lines.join("\n"));
        }
        self
    }

    /// Add column information
    pub fn with_column(mut self, column: usize) -> Self {
        self.column = Some(column);
        self
    }

    /// Add mock ID
    pub fn with_mock_id(mut self, mock_id: impl Into<String>) -> Self {
        self.mock_id = Some(mock_id.into());
        self
    }

    /// Add suggestions
    pub fn with_suggestions(mut self, suggestions: Vec<String>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// Parse a Tera error and extract useful information
    pub fn from_tera_error(error: tera::Error, template: &str) -> Self {
        let message = error.to_string();

        // Try to extract line/column from error message
        // Tera errors often include line numbers
        let (line, column) = extract_line_column(&message);

        // Determine error type
        let error_type = if message.contains("parse") || message.contains("syntax") {
            "parse"
        } else if message.contains("function") || message.contains("filter") {
            "function"
        } else {
            "render"
        };

        // Generate suggestions based on error
        let suggestions = generate_template_suggestions(&message);

        Self::new(error_type, message)
            .with_excerpt(template, line)
            .with_column(column.unwrap_or(0))
            .with_suggestions(suggestions)
    }
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Template {} Error", self.error_type)?;
        if let Some(mock_id) = &self.mock_id {
            writeln!(f, "Mock ID: {}", mock_id)?;
        }
        writeln!(f, "Message: {}", self.message)?;

        if let Some(excerpt) = &self.template_excerpt {
            writeln!(f, "\nTemplate excerpt:")?;
            writeln!(f, "{}", excerpt)?;
        }

        if !self.suggestions.is_empty() {
            writeln!(f, "\nSuggestions:")?;
            for suggestion in &self.suggestions {
                writeln!(f, "  • {}", suggestion)?;
            }
        }

        Ok(())
    }
}

impl std::error::Error for TemplateError {}

/// Extract line and column numbers from error messages
fn extract_line_column(message: &str) -> (Option<usize>, Option<usize>) {
    // Try to match patterns like "line 5" or "at line 5, column 10"
    let line: Option<usize> = LINE_REGEX
        .captures(message)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok());

    let column: Option<usize> = COLUMN_REGEX
        .captures(message)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok());

    (line, column)
}

/// Generate suggestions based on error message
fn generate_template_suggestions(message: &str) -> Vec<String> {
    let mut suggestions = Vec::new();

    // Specific error: empty range in get_random
    if message.contains("cannot sample empty range") || message.contains("sample empty range") {
        suggestions.push(
            "The get_random() function requires start < end (e.g., get_random(start=1, end=5))"
                .to_string(),
        );
        suggestions.push(
            "Check if you're using get_random(start=X, end=X) which creates an empty range"
                .to_string(),
        );
        suggestions
            .push("If generated by consolidator, this might be a bug - please report".to_string());
    }

    // Specific error: division by zero
    if message.contains("division by zero") || message.contains("divide by zero") {
        suggestions
            .push("Check for division operations where the denominator might be 0".to_string());
        suggestions.push(
      "Use a default value or conditional: {% if divisor != 0 %}{{ value / divisor }}{% else %}0{% endif %}"
        .to_string(),
    );
    }

    // Specific error: null access
    if message.contains("null") && (message.contains("access") || message.contains("index")) {
        suggestions.push("Check for null/undefined values before accessing properties".to_string());
        suggestions.push(
            "Use the default filter: {{ variable | default(value=\"fallback\") }}".to_string(),
        );
        suggestions.push(
            "Or use conditional: {% if variable %}{{ variable.field }}{% endif %}".to_string(),
        );
    }

    if message.contains("unknown function") || message.contains("Unknown function") {
        suggestions.push("Check available functions: uuid(), fake_name(), fake_email(), now(), range(), get_random() (Tera built-ins), etc.".to_string());

        // Try to suggest similar function names
        if let Some(func_name) = extract_function_name(message) {
            let similar = find_similar_function(&func_name);
            if !similar.is_empty() {
                suggestions.push(format!("Did you mean: {}?", similar.join(", ")));
            }
        }
    }

    if message.contains("unknown variable") || message.contains("Variable") {
        suggestions.push(
            "Check available variables: method, path, captures, query, headers, body, body_json"
                .to_string(),
        );
        suggestions.push("Use {% set my_var = value %} to define custom variables".to_string());
    }

    if message.contains("parse") || message.contains("syntax") {
        suggestions.push("Check for missing closing braces {{ }} or {% %}".to_string());
        suggestions.push("Ensure proper Tera syntax: https://keats.github.io/tera/".to_string());
        suggestions.push(
            "Common syntax: {{ variable }}, {% if condition %}, {% for item in array %}"
                .to_string(),
        );
    }

    if message.contains("filter") {
        suggestions
            .push("Common filters: json_encode, lower, upper, trim, length, default".to_string());
        suggestions.push(
            "Custom filters: base64_encode, base64_decode, urldecode, random_choice".to_string(),
        );
        suggestions.push("Filter syntax: {{ value | filter_name(param=value) }}".to_string());
    }

    // Type mismatch errors
    if message.contains("expected")
        && (message.contains("number") || message.contains("string") || message.contains("array"))
    {
        suggestions.push("Check the data types being passed to functions or filters".to_string());
        suggestions
            .push("Use type conversion: {{ value | int }} or {{ value | string }}".to_string());
    }

    // If no specific suggestions, provide general help
    if suggestions.is_empty() {
        suggestions.push("Check the template syntax and variable names".to_string());
        suggestions.push("View Tera documentation: https://keats.github.io/tera/".to_string());
    }

    suggestions
}

/// Extract function name from error message
fn extract_function_name(message: &str) -> Option<String> {
    FUNCTION_NAME_REGEX
        .captures(message)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// Find similar function names using simple string distance
fn find_similar_function(name: &str) -> Vec<String> {
    const AVAILABLE_FUNCTIONS: &[&str] = &[
        // Custom functions - Identity & Personal
        "uuid",
        "fake_name",
        "fake_first_name",
        "fake_last_name",
        "fake_username",
        "fake_password",
        "fake_title",
        "fake_suffix",
        // Contact
        "fake_email",
        "fake_free_email",
        "fake_phone",
        "fake_cell_phone",
        // Location & Address
        "fake_street",
        "fake_street_address",
        "fake_city",
        "fake_state",
        "fake_state_abbr",
        "fake_zip",
        "fake_country",
        "fake_country_code",
        "fake_latitude",
        "fake_longitude",
        "fake_postal_code",
        "fake_building_number",
        "fake_secondary_address",
        // Company & Job
        "fake_company",
        "fake_company_suffix",
        "fake_job_title",
        "fake_industry",
        "fake_job_field",
        "fake_job_position",
        "fake_job_seniority",
        // Internet
        "fake_url",
        "fake_domain",
        "fake_ipv4",
        "fake_ipv6",
        "fake_mac_address",
        "fake_user_agent",
        "fake_color",
        // Text
        "fake_words",
        "fake_sentence",
        "fake_paragraph",
        "fake_word",
        "fake_slug",
        "fake_alphanumeric",
        // Web & Files
        "fake_boolean",
        "fake_filename",
        "fake_file_size",
        "fake_download_url",
        "fake_token",
        "fake_etag",
        "fake_mime_type",
        "fake_file_extension",
        // Dates
        "fake_date",
        "fake_time",
        "fake_iso_date",
        "fake_unix_timestamp",
        "fake_relative_time",
        // Finance
        "fake_credit_card",
        "fake_currency_code",
        "fake_currency_name",
        "fake_currency_symbol",
        "fake_price",
        "fake_amount",
        // Identifiers
        "fake_uuid",
        "fake_isbn",
        "fake_isbn13",
        "fake_numeric_id",
        "fake_short_hash",
        "fake_sha256",
        "fake_md5",
        "fake_base64",
        "fake_jwt",
        // Numbers
        "fake_number",
        "fake_float",
        "fake_digit",
        // Semantic
        "fake_status_message",
        "fake_api_version",
        "fake_version",
        "fake_hex_color",
        "fake_rgb_color",
        "fake_locale",
        "fake_timezone",
        "fake_semver",
        "fake_semver_prerelease",
        // URL Generators (Specialized)
        "fake_pagination_url",
        "fake_pagination_url_offset",
        "fake_search_url",
        "fake_file_download_url",
        "fake_api_url",
        "fake_webhook_url",
        // API & Path
        "fake_api_endpoint",
        "fake_resource_path",
        "fake_user_agent_modern",
        // File Generation (binary)
        "fake_pdf",
        "fake_png",
        "fake_jpeg",
        // Data URI Generation
        "fake_pdf_data_uri",
        "fake_png_data_uri",
        "fake_jpeg_data_uri",
        // Image Generation
        "fake_image_with_text",
        "fake_image_gradient",
        "fake_image_checkerboard",
        "fake_image_noise",
        "fake_image_stripes",
        "fake_placeholder",
        "fake_avatar",
        // Persistence Store
        "store_get",
        "store_set",
        "store_set_nx",
        "store_get_or_set",
        "store_incr",
        "store_decr",
        "store_has",
        "store_del",
        "store_clear",
        "store_keys",
        "store_ttl",
        // GraphQL Helpers
        "graphql_error",
        "graphql_field_error",
        "graphql_type",
        "graphql_schema",
        // Date Arithmetic
        "now_plus",
        "now_minus",
        "fake_iso_date_offset",
        // Array Generation
        "fake_array",
        // Tera built-ins
        "now",
        "range",
        "get_random",
        "throw",
        "get_env",
    ];

    AVAILABLE_FUNCTIONS
        .iter()
        .filter(|&func| levenshtein_distance(name, func) <= 2)
        .take(3)
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_suggestions_empty_range() {
        let message = "thread panicked: cannot sample empty range";
        let suggestions = generate_template_suggestions(message);

        assert!(
            !suggestions.is_empty(),
            "Should have suggestions for empty range error"
        );
        assert!(
            suggestions.iter().any(|s| s.contains("get_random")),
            "Should mention get_random: {:?}",
            suggestions
        );
        assert!(
            suggestions.iter().any(|s| s.contains("start < end")),
            "Should explain start < end requirement: {:?}",
            suggestions
        );
    }

    #[test]
    fn test_error_suggestions_unknown_function() {
        let message = "unknown function 'fake_nme'";
        let suggestions = generate_template_suggestions(message);

        assert!(!suggestions.is_empty());
        // Should suggest fake_name as it's similar to fake_nme
        assert!(
            suggestions.iter().any(|s| s.contains("fake_name")),
            "Should suggest similar function names: {:?}",
            suggestions
        );
    }

    #[test]
    fn test_error_suggestions_unknown_variable() {
        let message = "Variable 'foo' not found";
        let suggestions = generate_template_suggestions(message);

        assert!(!suggestions.is_empty());
        assert!(
            suggestions
                .iter()
                .any(|s| s.contains("method") || s.contains("path")),
            "Should list available variables: {:?}",
            suggestions
        );
    }

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein_distance("fake_name", "fake_nme"), 1);
        assert_eq!(levenshtein_distance("fake_name", "fake_email"), 4); // name->email: remove 'n', 'a', 'm', add 'e', 'm', 'a', 'i', 'l' = min 4 ops
        assert_eq!(levenshtein_distance("uuid", "uui"), 1);
    }

    #[test]
    fn test_template_error_display() {
        let error = TemplateError::new("parse", "Missing closing brace")
            .with_mock_id("test-mock")
            .with_suggestions(vec!["Check for missing {{ }}".to_string()]);

        let display = format!("{}", error);
        assert!(display.contains("test-mock"));
        assert!(display.contains("Missing closing brace"));
        assert!(display.contains("Check for missing"));
    }
}
