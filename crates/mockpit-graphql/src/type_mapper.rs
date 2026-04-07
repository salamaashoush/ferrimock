//! Maps GraphQL types to fake data template functions using the advanced type detector

use anyhow::{Context, Result};
use rustc_hash::FxHashMap;
use std::path::Path;

use mockpit_codegen::field_type_to_tera_expr;
use mockpit_type_detector::detect_from_semantic_context;

/// Maps GraphQL scalar types to Tera template fake data functions
pub struct TypeToFakeMapper {
    /// Custom mappings from type names to template expressions
    custom_mappings: FxHashMap<String, String>,
}

impl TypeToFakeMapper {
    /// Create a new type mapper with default mappings
    pub fn new() -> Self {
        Self {
            custom_mappings: FxHashMap::default(),
        }
    }

    /// Map a GraphQL scalar type to a fake data template expression, considering the field name
    pub fn scalar_to_fake_with_field(&self, scalar_name: &str, field_name: Option<&str>) -> String {
        // Check custom mappings first
        if let Some(custom) = self.custom_mappings.get(scalar_name) {
            return custom.clone();
        }

        // Use semantic detection from type detector (works without sample values)
        if let Some(name) = field_name {
            // The semantic detector can work with empty sample array for field-name-based detection
            if let Some((field_type, _confidence)) = detect_from_semantic_context(name, &[]) {
                return field_type_to_tera_expr(name, &field_type, false);
            }
        }

        // Type-based mappings (fallback to GraphQL scalar type)
        self.scalar_to_fake(scalar_name)
    }

    /// Map a GraphQL scalar type to a fake data template expression
    pub fn scalar_to_fake(&self, scalar_name: &str) -> String {
        // Check custom mappings first
        if let Some(custom) = self.custom_mappings.get(scalar_name) {
            return custom.clone();
        }

        // Built-in scalar mappings
        match scalar_name {
            // Standard GraphQL scalars
            "String" => "\"{{ fake_sentence(word_count=8) }}\"".to_string(),
            "Int" => "{{ get_random(start=1, end=1000) }}".to_string(),
            "Float" => "{{ get_random(start=0.0, end=1000.0) }}".to_string(),
            "Boolean" => "{{ fake_boolean() }}".to_string(),

            // Common custom scalars (heuristic-based)
            "Date" => "\"{{ fake_iso_date() }}\"".to_string(),
            "DateTime" | "Timestamp" => "\"{{ now() }}\"".to_string(),
            "Time" => "\"{{ fake_time() }}\"".to_string(),
            "Email" | "EmailAddress" => "\"{{ fake_email() }}\"".to_string(),
            "URL" | "Uri" | "Url" => "\"{{ fake_url() }}\"".to_string(),
            "ID" | "UUID" | "Uuid" => "\"{{ uuid() }}\"".to_string(),
            "JSON" | "Json" | "JSONObject" => "{}".to_string(),
            "PhoneNumber" | "Phone" => "\"{{ fake_phone() }}\"".to_string(),
            "PostalCode" | "ZipCode" => "\"{{ fake_postal_code() }}\"".to_string(),
            "CountryCode" => "\"{{ fake_country_code() }}\"".to_string(),
            "Currency" | "CurrencyCode" => "\"{{ fake_currency_code() }}\"".to_string(),
            "Decimal" | "Money" => "{{ get_random(start=1.0, end=999.99) }}".to_string(),
            "BigInt" | "Long" => "{{ get_random(start=1, end=1000000) }}".to_string(),

            // Fallback heuristics
            _ => {
                // ID suffix heuristic
                if scalar_name.ends_with("ID") || scalar_name.ends_with("Id") {
                    return "\"{{ uuid() }}\"".to_string();
                }

                // Name suffix heuristic
                if scalar_name.ends_with("Name") {
                    return "\"{{ fake_name() }}\"".to_string();
                }

                // Default to simple word
                "\"{{ fake_word() }}\"".to_string()
            }
        }
    }

    /// Map an enum type to a fake data template expression
    pub fn enum_to_fake(&self, enum_values: &[String]) -> String {
        if enum_values.is_empty() {
            return "null".to_string();
        }

        // Generate a random choice from enum values
        let values_list = enum_values
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(", ");

        format!("\"{{{{ [{values_list}] | random_choice }}}}\"")
    }

    /// Add a custom type mapping
    pub fn add_mapping(&mut self, type_name: String, template_expr: String) {
        self.custom_mappings.insert(type_name, template_expr);
    }

    /// Load custom mappings from a JSON configuration file.
    ///
    /// Expected format (JSON):
    /// ```json
    /// {
    ///   "scalars": {
    ///     "MyCustomScalar": "\"{{ my_custom_function() }}\"",
    ///     "UserId": "\"{{ uuid() }}\"",
    ///     "Money": "{{ get_random(start=1.0, end=999.99) }}"
    ///   }
    /// }
    /// ```
    pub fn load_mappings(&mut self, path: &Path) -> Result<()> {
        use std::fs;

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read scalar mappings file: {}", path.display()))?;

        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let mappings: FxHashMap<String, String> = match extension {
            "json" => {
                let config: serde_json::Value = serde_json::from_str(&content)
                    .with_context(|| format!("Failed to parse JSON file: {}", path.display()))?;

                // Extract the "scalars" object
                config
                    .get("scalars")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default()
            }
            _ => {
                anyhow::bail!("Unsupported file extension '{extension}'. Use .json");
            }
        };

        // Merge loaded mappings into custom_mappings
        for (type_name, template_expr) in mappings {
            self.custom_mappings.insert(type_name, template_expr);
        }

        Ok(())
    }

    /// Get all custom mappings
    pub fn get_custom_mappings(&self) -> &FxHashMap<String, String> {
        &self.custom_mappings
    }
}

impl Default for TypeToFakeMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_builtin_scalars() {
        let mapper = TypeToFakeMapper::new();

        assert_eq!(mapper.scalar_to_fake("ID"), "\"{{ uuid() }}\"");
        assert_eq!(
            mapper.scalar_to_fake("String"),
            "\"{{ fake_sentence(word_count=8) }}\""
        );
        assert_eq!(
            mapper.scalar_to_fake("Int"),
            "{{ get_random(start=1, end=1000) }}"
        );
        assert_eq!(
            mapper.scalar_to_fake("Float"),
            "{{ get_random(start=0.0, end=1000.0) }}"
        );
        assert_eq!(mapper.scalar_to_fake("Boolean"), "{{ fake_boolean() }}");
    }

    #[test]
    fn test_common_custom_scalars() {
        let mapper = TypeToFakeMapper::new();

        assert_eq!(mapper.scalar_to_fake("DateTime"), "\"{{ now() }}\"");
        assert_eq!(mapper.scalar_to_fake("Email"), "\"{{ fake_email() }}\"");
        assert_eq!(mapper.scalar_to_fake("URL"), "\"{{ fake_url() }}\"");
        assert_eq!(mapper.scalar_to_fake("UUID"), "\"{{ uuid() }}\"");
        assert_eq!(
            mapper.scalar_to_fake("PhoneNumber"),
            "\"{{ fake_phone() }}\""
        );
    }

    #[test]
    fn test_heuristic_id_suffix() {
        let mapper = TypeToFakeMapper::new();

        assert_eq!(mapper.scalar_to_fake("UserID"), "\"{{ uuid() }}\"");
        assert_eq!(mapper.scalar_to_fake("PostId"), "\"{{ uuid() }}\"");
    }

    #[test]
    fn test_heuristic_name_suffix() {
        let mapper = TypeToFakeMapper::new();

        assert_eq!(mapper.scalar_to_fake("FirstName"), "\"{{ fake_name() }}\"");
        assert_eq!(mapper.scalar_to_fake("LastName"), "\"{{ fake_name() }}\"");
    }

    #[test]
    fn test_custom_mappings() {
        let mut mapper = TypeToFakeMapper::new();

        mapper.add_mapping("CustomType".to_string(), "\"custom_value\"".to_string());
        assert_eq!(mapper.scalar_to_fake("CustomType"), "\"custom_value\"");
    }

    #[test]
    fn test_enum_to_fake() {
        let mapper = TypeToFakeMapper::new();

        let enum_values = vec!["ADMIN".to_string(), "USER".to_string(), "GUEST".to_string()];
        let result = mapper.enum_to_fake(&enum_values);

        assert!(result.contains("random_choice"));
        assert!(result.contains("ADMIN"));
        assert!(result.contains("USER"));
        assert!(result.contains("GUEST"));
    }

    #[test]
    fn test_enum_to_fake_empty() {
        let mapper = TypeToFakeMapper::new();

        let result = mapper.enum_to_fake(&[]);
        assert_eq!(result, "null");
    }

    #[test]
    fn test_fallback() {
        let mapper = TypeToFakeMapper::new();

        // Unknown type should fall back to fake_word
        assert_eq!(
            mapper.scalar_to_fake("UnknownType"),
            "\"{{ fake_word() }}\""
        );
    }

    #[test]
    fn test_load_mappings_toml_unsupported() {
        use tempfile::NamedTempFile;

        let toml_content = r#"
[scalars]
MyCustomScalar = "\"{{ my_custom_function() }}\""
"#;

        let mut temp_file = NamedTempFile::with_suffix(".toml")
            .expect("Failed to create temporary file with suffix");
        temp_file
            .write_all(toml_content.as_bytes())
            .expect("Failed to write TOML content to temporary file");
        temp_file.flush().expect("Failed to flush temporary file");

        let mut mapper = TypeToFakeMapper::new();
        let result = mapper.load_mappings(temp_file.path());
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported file extension")
        );
    }

    #[test]
    fn test_load_mappings_json() {
        use tempfile::NamedTempFile;

        let json_content = r#"
{
  "scalars": {
    "MyCustomScalar": "\"{{ my_custom_function() }}\"",
    "UserId": "\"{{ uuid() }}\"",
    "Money": "{{ get_random(start=1.0, end=999.99) }}"
  }
}
"#;

        let mut temp_file = NamedTempFile::with_suffix(".json")
            .expect("Failed to create temporary file with suffix");
        temp_file
            .write_all(json_content.as_bytes())
            .expect("Failed to write JSON content to temporary file");
        temp_file.flush().expect("Failed to flush temporary file");

        let mut mapper = TypeToFakeMapper::new();
        mapper
            .load_mappings(temp_file.path())
            .expect("Failed to load GraphQL type mappings from JSON file");

        assert_eq!(
            mapper.scalar_to_fake("MyCustomScalar"),
            "\"{{ my_custom_function() }}\""
        );
        assert_eq!(mapper.scalar_to_fake("UserId"), "\"{{ uuid() }}\"");
        assert_eq!(
            mapper.scalar_to_fake("Money"),
            "{{ get_random(start=1.0, end=999.99) }}"
        );
    }

    #[test]
    fn test_load_mappings_custom_override_builtin() {
        use tempfile::NamedTempFile;

        let json_content = r#"
{
  "scalars": {
    "UUID": "\"{{ fake_alphanumeric(length=32) }}\""
  }
}
"#;

        let mut temp_file = NamedTempFile::with_suffix(".json")
            .expect("Failed to create temporary file with suffix");
        temp_file
            .write_all(json_content.as_bytes())
            .expect("Failed to write JSON content to temporary file");
        temp_file.flush().expect("Failed to flush temporary file");

        let mut mapper = TypeToFakeMapper::new();

        // Before loading: uses built-in
        assert_eq!(mapper.scalar_to_fake("UUID"), "\"{{ uuid() }}\"");

        // After loading: uses custom
        mapper
            .load_mappings(temp_file.path())
            .expect("Failed to load custom GraphQL type mappings");
        assert_eq!(
            mapper.scalar_to_fake("UUID"),
            "\"{{ fake_alphanumeric(length=32) }}\""
        );
    }

    #[test]
    fn test_load_mappings_unsupported_extension() {
        use tempfile::NamedTempFile;

        let mut temp_file = NamedTempFile::with_suffix(".txt")
            .expect("Failed to create temporary file with suffix");
        temp_file
            .write_all(b"some content")
            .expect("Failed to write to temporary file");
        temp_file.flush().expect("Failed to flush temporary file");

        let mut mapper = TypeToFakeMapper::new();
        let result = mapper.load_mappings(temp_file.path());

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unsupported file extension")
        );
    }

    #[test]
    fn test_load_mappings_nonexistent_file() {
        let mut mapper = TypeToFakeMapper::new();
        let result = mapper.load_mappings(Path::new("/nonexistent/file.json"));

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read scalar mappings file")
        );
    }
}
