//! Response configuration

use super::patches::{HeaderPatchesConfig, JsonPatchConfig, RegexReplaceConfig};
use http::StatusCode;
use mockpit_types::{BodySource, PatchOperation, ResponseGenerator};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Patches applied to upstream response (passthrough mode only)
///
/// Used under `response.patches` in mock config files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ResponsePatchesConfig {
    /// RFC 6902 JSON Patch operations
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<JsonPatchConfig>,

    /// JSONPath-style patches (simpler syntax)
    #[serde(default, skip_serializing_if = "FxHashMap::is_empty")]
    #[allow(clippy::disallowed_types)]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::HashMap<String, serde_json::Value>")
    )]
    pub jsonpath: FxHashMap<String, serde_json::Value>,

    /// Regex replacements
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regex: Vec<RegexReplaceConfig>,

    /// Header operations
    #[serde(default, skip_serializing_if = "HeaderPatchesConfig::is_empty")]
    pub headers: HeaderPatchesConfig,
}

/// Response configuration (flat syntax)
///
/// Supports multiple forms:
/// 1. Individual fields: `response.status`, `response.body`, `response.headers.*`
/// 2. Bare string: `response = 'static body'` - always treated as static inline body
/// 3. JSON shorthand: `response.json.*` - structured JSON response (static)
/// 4. Status as key: `response.200 = "body"` - ultra-flat syntax (static)
/// 5. Template: `response.template = '{{ expr }}'` - inline Tera template
/// 6. File: `response.file = "path"` - static body from file
/// 7. Template file: `response.template_file = "path"` - Tera template from file
///
/// Only one of `body`, `template`, `file`, `template_file`, `json` may be set per mock.
#[derive(Debug, Clone)]
pub enum ResponseConfig {
    /// Bare string value: response = "static body"
    /// Always treated as static inline body (no template detection).
    Template(String),

    /// Status shortcuts: status code as key
    /// Example: response.200 = "success", response.404 = "not found"
    /// Body values are always static (no template detection).
    StatusShortcuts(FxHashMap<u16, String>),

    /// Structured configuration with individual fields
    Structured {
        /// HTTP status code
        status: Option<u16>,

        /// Response headers
        headers: FxHashMap<String, String>,

        /// Static inline body (never templated)
        body: Option<String>,

        /// Inline Tera template (always processed by template engine)
        template: Option<String>,

        /// Body loaded from file (static, no template processing)
        file: Option<String>,

        /// Template loaded from file (processed by Tera)
        template_file: Option<String>,

        /// Structured JSON object (static, no template processing)
        json: Box<serde_json::Value>,
    },
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for ResponseConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ResponseConfig".into()
    }

    fn json_schema(_schema_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        serde_json::json!({
      "description": "Response configuration. Supports: bare string, status shortcuts, or structured object with explicit body type fields.",
      "oneOf": [
        {
          "type": "string",
          "description": "Static inline body string"
        },
        {
          "type": "object",
          "description": "Status code as key with body as value (e.g., {\"200\": \"body\"})",
          "patternProperties": {
            "^[1-5][0-9]{2}$": { "type": "string" }
          },
          "additionalProperties": false
        },
        {
          "type": "object",
          "description": "Structured response with explicit body type fields. Only one of body, template, file, template_file, json may be set.",
          "properties": {
            "status": { "type": "integer", "minimum": 100, "maximum": 599 },
            "headers": { "type": "object", "additionalProperties": { "type": "string" } },
            "body": {
              "description": "Static inline body (never templated). Can be a string, object, array, number, or boolean. Non-string values are automatically serialized to JSON.",
              "oneOf": [
                { "type": "string" },
                { "type": "object" },
                { "type": "array" },
                { "type": "number" },
                { "type": "boolean" }
              ]
            },
            "template": { "type": "string", "description": "Inline Tera template (always processed by template engine)" },
            "file": { "type": "string", "description": "Path to file for static body content" },
            "template_file": { "type": "string", "description": "Path to file containing a Tera template (alias: templateFile)" },
            "templateFile": { "type": "string", "description": "Alias for template_file" },
            "json": { "description": "Structured JSON body (static, no template processing)" }
          }
        }
      ]
    })
    .as_object()
    .unwrap()
    .clone()
    .into()
    }
}

// Custom serialization for ResponseConfig
impl serde::Serialize for ResponseConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        match self {
            ResponseConfig::Template(s) => {
                // For templates, serialize as a map with body field
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("body", s)?;
                map.end()
            }
            ResponseConfig::StatusShortcuts(shortcuts) => {
                // Serialize status shortcuts directly
                let mut map = serializer.serialize_map(Some(shortcuts.len()))?;
                for (status, body) in shortcuts {
                    map.serialize_entry(&status.to_string(), body)?;
                }
                map.end()
            }
            ResponseConfig::Structured {
                status,
                headers,
                body,
                template,
                file,
                template_file,
                json,
            } => {
                let mut map = serializer.serialize_map(None)?;

                if let Some(s) = status {
                    map.serialize_entry("status", s)?;
                }
                if !headers.is_empty() {
                    map.serialize_entry("headers", headers)?;
                }
                if let Some(b) = body {
                    map.serialize_entry("body", b)?;
                }
                if let Some(t) = template {
                    map.serialize_entry("template", t)?;
                }
                if let Some(f) = file {
                    map.serialize_entry("file", f)?;
                }
                if let Some(tf) = template_file {
                    map.serialize_entry("template_file", tf)?;
                }
                if !json.is_null() {
                    map.serialize_entry("json", json)?;
                }

                map.end()
            }
        }
    }
}

/// Deserialize `body` as either a string or an object/array.
/// When the value is an object or array, it is serialized to a compact JSON string.
/// This allows users to write `body: { "data": { ... } }` in JSON/YAML files
/// instead of requiring a JSON-encoded string.
fn deserialize_body_flexible<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(other) => {
            // Object, array, number, bool -> serialize to compact JSON (faster than pretty-print)
            let json_str = serde_json::to_string(&other).map_err(|e| {
                serde::de::Error::custom(format!("Failed to serialize body value: {e}"))
            })?;
            Ok(Some(json_str))
        }
    }
}

// Custom deserialization for ResponseConfig
impl<'de> serde::Deserialize<'de> for ResponseConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Template(String),
            Structured(Box<StructuredHelper>),
        }

        #[derive(Deserialize)]
        struct StructuredHelper {
            #[serde(default)]
            status: Option<u16>,
            #[serde(default)]
            headers: FxHashMap<String, String>,
            #[serde(default, deserialize_with = "deserialize_body_flexible")]
            body: Option<String>,
            #[serde(default)]
            template: Option<String>,
            #[serde(default)]
            file: Option<String>,
            #[serde(default, alias = "templateFile")]
            template_file: Option<String>,
            #[serde(default)]
            json: serde_json::Value,

            /// Capture unknown fields for status shortcuts
            #[serde(flatten)]
            extra: FxHashMap<String, serde_json::Value>,
        }

        let helper = Helper::deserialize(deserializer)?;
        Ok(match helper {
            Helper::Template(s) => ResponseConfig::Template(s),
            Helper::Structured(structured) => {
                let StructuredHelper {
                    status,
                    headers,
                    body,
                    template,
                    file,
                    template_file,
                    json,
                    extra,
                } = *structured;
                // Check if there are status code shortcuts (e.g., response.200 = "body")
                let mut status_shortcuts = FxHashMap::default();
                let has_structured_fields = status.is_some()
                    || !headers.is_empty()
                    || body.is_some()
                    || template.is_some()
                    || file.is_some()
                    || template_file.is_some()
                    || !json.is_null();

                for (key, value) in extra {
                    // Check if key is a valid status code (100-599)
                    if let Ok(status_code) = key.parse::<u16>() {
                        if (100..=599).contains(&status_code) {
                            // This is a status shortcut
                            if let Some(body_str) = value.as_str() {
                                status_shortcuts.insert(status_code, body_str.to_string());
                            } else {
                                return Err(D::Error::custom(format!(
                                    "Status shortcut value must be a string, got: {:?}",
                                    value
                                )));
                            }
                        }
                    }
                }

                // If we have status shortcuts and no structured fields, use StatusShortcuts variant
                if !status_shortcuts.is_empty() && !has_structured_fields {
                    ResponseConfig::StatusShortcuts(status_shortcuts)
                } else if !status_shortcuts.is_empty() {
                    // Can't mix status shortcuts with structured fields
                    return Err(D::Error::custom(
                        "Cannot mix status shortcuts (e.g., response.200) with structured fields (e.g., response.status)",
                    ));
                } else {
                    // Use structured variant
                    ResponseConfig::Structured {
                        status,
                        headers,
                        body,
                        template,
                        file,
                        template_file,
                        json: Box::new(json),
                    }
                }
            }
        })
    }
}

impl Default for ResponseConfig {
    fn default() -> Self {
        ResponseConfig::Structured {
            status: None,
            headers: FxHashMap::default(),
            body: None,
            template: None,
            file: None,
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        }
    }
}

impl ResponseConfig {
    /// Replace the body string value (for formatting).
    /// Only modifies inline body/template strings, not file references.
    pub fn set_body(&mut self, new_body: String) {
        match self {
            ResponseConfig::Template(s) => *s = new_body,
            ResponseConfig::StatusShortcuts(shortcuts) => {
                if let Some(body) = shortcuts.get_mut(&200) {
                    *body = new_body;
                } else if let Some(body) = shortcuts.values_mut().next() {
                    *body = new_body;
                }
            }
            ResponseConfig::Structured { body, .. } => {
                *body = Some(new_body);
            }
        }
    }

    /// Replace the template string value (for formatting).
    pub fn set_template(&mut self, new_template: String) {
        match self {
            ResponseConfig::Template(_) | ResponseConfig::StatusShortcuts(_) => {}
            ResponseConfig::Structured { template, .. } => {
                *template = Some(new_template);
            }
        }
    }

    /// Get the body field (for pattern matching during consolidation)
    pub fn body(&self) -> Option<&String> {
        match self {
            ResponseConfig::Template(s) => Some(s),
            ResponseConfig::StatusShortcuts(shortcuts) => {
                // Return the first status code body (typically 200)
                shortcuts.get(&200).or_else(|| shortcuts.values().next())
            }
            ResponseConfig::Structured { body, .. } => body.as_ref(),
        }
    }

    /// Get the template field
    pub fn template(&self) -> Option<&String> {
        match self {
            ResponseConfig::Template(_) | ResponseConfig::StatusShortcuts(_) => None,
            ResponseConfig::Structured { template, .. } => template.as_ref(),
        }
    }

    /// Get the file field
    pub fn file_ref(&self) -> Option<&String> {
        match self {
            ResponseConfig::Template(_) | ResponseConfig::StatusShortcuts(_) => None,
            ResponseConfig::Structured { file, .. } => file.as_ref(),
        }
    }

    /// Get the template_file field
    pub fn template_file_ref(&self) -> Option<&String> {
        match self {
            ResponseConfig::Template(_) | ResponseConfig::StatusShortcuts(_) => None,
            ResponseConfig::Structured { template_file, .. } => template_file.as_ref(),
        }
    }

    /// Get the json field
    pub fn json(&self) -> Option<&serde_json::Value> {
        match self {
            ResponseConfig::Template(_) | ResponseConfig::StatusShortcuts(_) => None,
            ResponseConfig::Structured { json, .. } => Some(json),
        }
    }

    /// Get the status field (for pattern matching during consolidation)
    pub fn status(&self) -> Option<u16> {
        match self {
            ResponseConfig::Template(_) => None,
            ResponseConfig::StatusShortcuts(shortcuts) => {
                // Return the first status code (typically 200)
                shortcuts.keys().next().copied()
            }
            ResponseConfig::Structured { status, .. } => *status,
        }
    }

    /// Get the headers field (for validation)
    pub fn headers(&self) -> Option<&FxHashMap<String, String>> {
        match self {
            ResponseConfig::Template(_) => None,
            ResponseConfig::StatusShortcuts(_) => None,
            ResponseConfig::Structured { headers, .. } => Some(headers),
        }
    }

    /// Returns true if this config defines a full mock (has body content of any type)
    pub fn is_full_mock(&self) -> bool {
        match self {
            ResponseConfig::Template(_) => true,
            ResponseConfig::StatusShortcuts(_) => true,
            ResponseConfig::Structured {
                body,
                template,
                file,
                template_file,
                json,
                ..
            } => {
                body.is_some()
                    || template.is_some()
                    || file.is_some()
                    || template_file.is_some()
                    || !json.is_null()
            }
        }
    }

    /// Backward-compatibility alias for `into_resolved_response`
    #[deprecated(note = "Use into_resolved_response() instead")]
    pub fn into_response_config(self) -> ResolvedResponse {
        self.into_resolved_response()
    }

    /// Convert to ResolvedResponse
    pub fn into_resolved_response(self) -> ResolvedResponse {
        match self {
            // Bare string variant: response = "static body"
            // Always treated as static inline body (no template detection)
            ResponseConfig::Template(body_str) => ResolvedResponse {
                status: 200,
                headers: FxHashMap::default(),
                body: BodyConfig::Inline { inline: body_str },
            },

            // Status shortcuts variant: response.200 = "body"
            // Always treated as static inline body (no template detection)
            ResponseConfig::StatusShortcuts(shortcuts) => {
                let (status, body_str) =
                    shortcuts.into_iter().next().unwrap_or((200, String::new()));

                ResolvedResponse {
                    status,
                    headers: FxHashMap::default(),
                    body: BodyConfig::Inline { inline: body_str },
                }
            }

            // Structured variant: explicit fields determine body type
            // Priority: template > template_file > file > json (static) > body (static) > Empty
            ResponseConfig::Structured {
                status,
                mut headers,
                body,
                template,
                file,
                template_file,
                json,
            } => {
                let status = status.unwrap_or(200);

                let body = if let Some(template_str) = template {
                    // Inline Tera template (always processed)
                    BodyConfig::Template {
                        template: template_str,
                    }
                } else if let Some(tf_path) = template_file {
                    // Template loaded from file (processed by Tera)
                    BodyConfig::TemplateFile {
                        template_file: tf_path,
                    }
                } else if let Some(file_path) = file {
                    // Body from file (static, no template processing)
                    BodyConfig::File { file: file_path }
                } else if !json.is_null() {
                    // JSON shorthand (static, no template processing)
                    // Auto-add Content-Type: application/json
                    if !headers
                        .keys()
                        .any(|k| k.eq_ignore_ascii_case("content-type"))
                    {
                        headers.insert("Content-Type".to_string(), "application/json".to_string());
                    }
                    if let Some(json_str) = json.as_str() {
                        BodyConfig::Inline {
                            inline: json_str.to_string(),
                        }
                    } else {
                        // Compact JSON serialization (faster than pretty-print, smaller body)
                        let json_string =
                            serde_json::to_string(&json).unwrap_or_else(|_| "null".to_string());
                        BodyConfig::Inline {
                            inline: json_string,
                        }
                    }
                } else if let Some(body_str) = body {
                    // Static inline body (no template processing)
                    BodyConfig::Inline { inline: body_str }
                } else {
                    BodyConfig::Empty
                };

                ResolvedResponse {
                    status,
                    headers,
                    body,
                }
            }
        }
    }
}

/// Body configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum BodyConfig {
    /// Inline body content
    Inline { inline: String },

    /// Body from file
    File { file: String },

    /// Template body content
    Template { template: String },

    /// Template from file
    TemplateFile { template_file: String },

    /// Empty body (default)
    #[default]
    Empty,
}

impl BodyConfig {
    /// Returns true if this is an empty body
    pub fn is_empty(&self) -> bool {
        matches!(self, BodyConfig::Empty)
    }
}

/// Resolved response configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ResolvedResponse {
    /// HTTP status code
    #[serde(default = "default_status")]
    pub status: u16,

    /// Response headers
    #[serde(default, skip_serializing_if = "FxHashMap::is_empty")]
    #[allow(clippy::disallowed_types)]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::HashMap<String, String>")
    )]
    pub headers: FxHashMap<String, String>,

    /// Response body configuration
    #[serde(default, skip_serializing_if = "BodyConfig::is_empty")]
    pub body: BodyConfig,
}

impl ResolvedResponse {
    /// Convert to response generator with config directory for resolving relative file paths
    pub async fn into_response_generator_with_dir(
        self,
        config_dir: Option<&std::path::Path>,
    ) -> Result<ResponseGenerator, String> {
        self.into_response_generator_with_caching(true, config_dir)
            .await
    }

    /// Internal implementation that can optionally pre-cache files
    async fn into_response_generator_with_caching(
        self,
        cache_files: bool,
        config_dir: Option<&std::path::Path>,
    ) -> Result<ResponseGenerator, String> {
        let status = StatusCode::from_u16(self.status)
            .map_err(|e| format!("Invalid status code {}: {}", self.status, e))?;

        let body = match self.body {
            BodyConfig::Inline { inline } => {
                // Always use Arc for zero-copy
                BodySource::Inline(std::sync::Arc::new(bytes::Bytes::from(inline)))
            }
            BodyConfig::File { file } => {
                // Resolve file path relative to config directory if provided
                let path = if let Some(dir) = config_dir {
                    dir.join(&file)
                } else {
                    std::path::PathBuf::from(&file)
                };

                if cache_files {
                    // Pre-load file contents into memory for performance
                    match tokio::fs::read(&path).await {
                        Ok(content) => {
                            // Cache the file content in memory
                            BodySource::FileCached(std::sync::Arc::new(bytes::Bytes::from(content)))
                        }
                        Err(e) => {
                            // Fall back to on-demand loading if file can't be read at config time
                            eprintln!(
                                "Warning: Failed to pre-load file {:?}: {}. Will load on demand.",
                                path, e
                            );
                            BodySource::File(path)
                        }
                    }
                } else {
                    // Don't pre-cache (for tests/scenarios)
                    BodySource::File(path)
                }
            }
            BodyConfig::Template { template } => BodySource::template(template),
            BodyConfig::TemplateFile { template_file } => {
                let path = if let Some(dir) = config_dir {
                    dir.join(&template_file)
                } else {
                    std::path::PathBuf::from(&template_file)
                };

                let template = tokio::fs::read_to_string(&path)
                    .await
                    .map_err(|e| format!("Failed to read template file {:?}: {}", path, e))?;

                BodySource::template(template)
            }
            BodyConfig::Empty => BodySource::inline(""),
        };

        let mut response = ResponseGenerator::new(status, body);
        response.headers = self.headers;

        Ok(response)
    }
}

/// Convert ResponsePatchesConfig to PatchOperations
pub fn parse_patches_config(config: ResponsePatchesConfig) -> Result<Vec<PatchOperation>, String> {
    let mut operations = Vec::new();

    // Parse JSON Patch operations (RFC 6902)
    if !config.operations.is_empty() {
        let json_patch_str = serde_json::to_string(&config.operations)
            .map_err(|e| format!("Failed to serialize JSON Patch operations: {}", e))?;
        let json_patch: json_patch::Patch = serde_json::from_str(&json_patch_str)
            .map_err(|e| format!("Failed to parse JSON Patch operations: {}", e))?;
        operations.push(PatchOperation::JsonPatch(json_patch));
    }

    // Parse JSONPath patches
    for (path, value) in config.jsonpath {
        operations.push(PatchOperation::JsonPath { path, value });
    }

    // Parse regex replacements
    for regex_config in config.regex {
        let pattern = regex::Regex::new(&regex_config.pattern)
            .map_err(|e| format!("Invalid regex pattern '{}': {}", regex_config.pattern, e))?;
        operations.push(PatchOperation::RegexReplace {
            pattern,
            replacement: regex_config.replacement,
        });
    }

    // Parse header additions
    for (name, value) in config.headers.add {
        operations.push(PatchOperation::HeaderAdd { name, value });
    }

    // Parse header removals
    for name in config.headers.remove {
        operations.push(PatchOperation::HeaderRemove { name });
    }

    Ok(operations)
}

/// Parse duration string (e.g., "100ms", "1s", "500us")
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();

    // Check for microseconds first (before 's' check)
    if let Some(us) = s.strip_suffix("us") {
        let value: u64 = us
            .trim()
            .parse()
            .map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_micros(value))
    } else if let Some(ms) = s.strip_suffix("ms") {
        let value: u64 = ms
            .trim()
            .parse()
            .map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_millis(value))
    } else if let Some(s_val) = s.strip_suffix('s') {
        let value: u64 = s_val
            .trim()
            .parse()
            .map_err(|_| format!("Invalid duration: {}", s))?;
        Ok(Duration::from_secs(value))
    } else {
        Err(format!(
            "Invalid duration format: {}. Expected format: '100ms', '1s', '500us'",
            s
        ))
    }
}

fn default_status() -> u16 {
    200
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(
            parse_duration("100ms").expect("Failed to parse duration"),
            Duration::from_millis(100)
        );
        assert_eq!(
            parse_duration("1s").expect("Failed to parse duration"),
            Duration::from_secs(1)
        );
        assert_eq!(
            parse_duration("500us").expect("Failed to parse duration"),
            Duration::from_micros(500)
        );
        assert_eq!(
            parse_duration("0ms").expect("Failed to parse duration"),
            Duration::from_millis(0)
        );
    }

    #[test]
    fn test_parse_duration_invalid() {
        assert!(parse_duration("100").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_resolved_response_default_status() {
        let yaml = r#"
body:
  inline: "{}"
"#;

        let config: ResolvedResponse =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.status, 200);
    }

    #[tokio::test]
    async fn test_response_headers() {
        let yaml = r#"
status: 200
body:
  inline: "{}"
headers:
  content-type: "application/json"
  x-custom: "value"
"#;

        let config: ResolvedResponse =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let generator = config
            .into_response_generator_with_dir(None)
            .await
            .expect("Failed to convert to response generator");

        assert_eq!(generator.headers.len(), 2);
        assert_eq!(
            generator
                .headers
                .get("content-type")
                .expect("content-type header should exist"),
            "application/json"
        );
        assert_eq!(
            generator
                .headers
                .get("x-custom")
                .expect("x-custom header should exist"),
            "value"
        );
    }

    #[tokio::test]
    async fn test_valid_status_codes() {
        let yaml = r#"
status: 200
body:
  inline: "{}"
"#;

        let config: ResolvedResponse =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let result = config.into_response_generator_with_dir(None).await;

        assert!(result.is_ok());
        assert_eq!(result.expect("Result should be Ok").status.as_u16(), 200);
    }

    #[test]
    fn test_return_json_auto_content_type() {
        // When using response.json, Content-Type should be auto-added
        let yaml = r#"
status: 200
json:
  message: "hello"
  count: 42
"#;

        let config: ResponseConfig =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let resolved_response = config.into_resolved_response();

        assert_eq!(
            resolved_response.headers.get("Content-Type"),
            Some(&"application/json".to_string()),
            "Content-Type should be auto-added for response.json"
        );
    }

    #[test]
    fn test_return_json_string_auto_content_type() {
        // When using response.json with a string value, Content-Type should be auto-added
        let yaml = r#"
status: 200
json: '{"message": "hello"}'
"#;

        let config: ResponseConfig =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let resolved_response = config.into_resolved_response();

        assert_eq!(
            resolved_response.headers.get("Content-Type"),
            Some(&"application/json".to_string()),
            "Content-Type should be auto-added for response.json string"
        );
    }

    #[test]
    fn test_return_json_does_not_override_explicit_content_type() {
        // If user explicitly sets Content-Type, it should not be overridden
        let yaml = r#"
status: 200
headers:
  Content-Type: "application/vnd.api+json"
json:
  message: "hello"
"#;

        let config: ResponseConfig =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let resolved_response = config.into_resolved_response();

        assert_eq!(
            resolved_response.headers.get("Content-Type"),
            Some(&"application/vnd.api+json".to_string()),
            "Explicit Content-Type should not be overridden"
        );
    }

    #[test]
    fn test_return_body_no_auto_content_type() {
        // When using response.body (not json), Content-Type should NOT be auto-added
        let yaml = r#"
status: 200
body: '{"message": "hello"}'
"#;

        let config: ResponseConfig =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let resolved_response = config.into_resolved_response();

        assert!(
            !resolved_response.headers.contains_key("Content-Type"),
            "Content-Type should NOT be auto-added for response.body"
        );
    }
}
