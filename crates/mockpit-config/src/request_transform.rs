//! Request transformation configuration
//!
//! Defines the `request` section in mock config files for modifying requests
//! before forwarding to upstream.

use super::patches::{HeaderPatchesConfig, JsonPatchConfig, RegexReplaceConfig};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Request transformation config (top-level `request` section in mock config)
///
/// All fields are optional. Any field being set implies passthrough (PatchUpstream) mode.
///
/// # Example
///
/// ```yaml
/// request:
///   delay: "500ms"
///   timeout: "10s"
///   forward_to: "https://staging.example.com"
///   rewrite_path: "/v2/users/{{ captures.id }}"
///   headers:
///     add:
///       x-trace-id: "{{ fake_uuid() }}"
///
/// [request.headers]
/// remove = ["x-real-ip"]
///
/// [request.query.add]
/// debug = "true"
///
/// [request.query]
/// remove = ["sensitive_key"]
///
/// [request.body.jsonpath]
/// "$.metadata.proxied" = true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RequestTransformConfig {
  /// Delay before forwarding to upstream (e.g., "500ms", "2s")
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub delay: Option<String>,

  /// Custom timeout for upstream request (e.g., "30s", "5000ms")
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub timeout: Option<String>,

  /// Override upstream host/URL (e.g., `https://staging.example.com`)
  #[serde(default, alias = "forwardTo", skip_serializing_if = "Option::is_none")]
  pub forward_to: Option<String>,

  /// Rewrite request path (supports Tera templates with captures)
  #[serde(default, alias = "rewritePath", skip_serializing_if = "Option::is_none")]
  pub rewrite_path: Option<String>,

  /// Header modifications (add/remove)
  #[serde(default, skip_serializing_if = "HeaderPatchesConfig::is_empty")]
  pub headers: HeaderPatchesConfig,

  /// Query parameter modifications (add/remove)
  #[serde(default, skip_serializing_if = "QueryPatchesConfig::is_empty")]
  pub query: QueryPatchesConfig,

  /// Request body modifications
  #[serde(default, skip_serializing_if = "BodyPatchesConfig::is_empty")]
  pub body: BodyPatchesConfig,
}

impl RequestTransformConfig {
  /// Returns true if any field is set (non-default)
  pub fn is_empty(&self) -> bool {
    self.delay.is_none()
      && self.timeout.is_none()
      && self.forward_to.is_none()
      && self.rewrite_path.is_none()
      && self.headers.add.is_empty()
      && self.headers.remove.is_empty()
      && self.query.add.is_empty()
      && self.query.remove.is_empty()
      && self.body.jsonpath.is_empty()
      && self.body.regex.is_empty()
      && self.body.operations.is_empty()
  }
}

/// Query parameter patch operations
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct QueryPatchesConfig {
  /// Query parameters to add (overwrites existing key if present)
  #[serde(default, skip_serializing_if = "FxHashMap::is_empty")]
  #[allow(clippy::disallowed_types)]
  #[cfg_attr(feature = "schema", schemars(with = "std::collections::HashMap<String, String>"))]
  pub add: FxHashMap<String, String>,

  /// Query parameter names to remove
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub remove: Vec<String>,
}

impl QueryPatchesConfig {
  /// Returns true if no query patches are configured
  pub fn is_empty(&self) -> bool {
    self.add.is_empty() && self.remove.is_empty()
  }
}

/// Body patch operations (used for both request and response body modifications)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct BodyPatchesConfig {
  /// JSONPath-style patches (e.g., "$.field" = "value")
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

  /// RFC 6902 JSON Patch operations
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub operations: Vec<JsonPatchConfig>,
}

impl BodyPatchesConfig {
  /// Returns true if no patches are configured
  pub fn is_empty(&self) -> bool {
    self.jsonpath.is_empty() && self.regex.is_empty() && self.operations.is_empty()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_parse_request_transform_full() {
    let yaml = r#"
delay: "500ms"
timeout: "10s"
forward_to: "https://staging.example.com"
rewrite_path: "/v2/users/{{ captures.id }}"
headers:
  add:
    x-trace-id: "{{ fake_uuid() }}"
    x-forwarded-for: "mock-proxy"
  remove:
    - "x-real-ip"
query:
  add:
    debug: "true"
    source: "mock"
  remove:
    - "sensitive_key"
body:
  jsonpath:
    "$.metadata.proxied": true
    "$.clientId": "test-client"
    "#;

    let config: RequestTransformConfig = serde_yaml::from_str(yaml).expect("Failed to parse");
    assert_eq!(config.delay.as_deref(), Some("500ms"));
    assert_eq!(config.timeout.as_deref(), Some("10s"));
    assert_eq!(config.forward_to.as_deref(), Some("https://staging.example.com"));
    assert_eq!(config.rewrite_path.as_deref(), Some("/v2/users/{{ captures.id }}"));
    assert_eq!(config.headers.add.len(), 2);
    assert_eq!(config.headers.remove.len(), 1);
    assert_eq!(config.query.add.len(), 2);
    assert_eq!(config.query.remove.len(), 1);
    assert_eq!(config.body.jsonpath.len(), 2);
    assert!(!config.is_empty());
  }

  #[test]
  fn test_parse_request_transform_minimal() {
    let yaml = r#"
delay: "100ms"
    "#;

    let config: RequestTransformConfig = serde_yaml::from_str(yaml).expect("Failed to parse");
    assert_eq!(config.delay.as_deref(), Some("100ms"));
    assert!(config.timeout.is_none());
    assert!(config.forward_to.is_none());
    assert!(!config.is_empty());
  }

  #[test]
  fn test_parse_request_transform_empty() {
    let yaml = "{}";
    let config: RequestTransformConfig = serde_yaml::from_str(yaml).expect("Failed to parse");
    assert!(config.is_empty());
  }

  #[test]
  fn test_parse_headers_only() {
    let yaml = r#"
headers:
  add:
    x-internal-auth: "secret-token"
    "#;

    let config: RequestTransformConfig = serde_yaml::from_str(yaml).expect("Failed to parse");
    assert_eq!(config.headers.add.len(), 1);
    assert_eq!(
      config.headers.add.get("x-internal-auth"),
      Some(&"secret-token".to_string())
    );
    assert!(!config.is_empty());
  }

  #[test]
  fn test_parse_query_only() {
    let yaml = r#"
query:
  add:
    debug: "true"
    verbose: "1"
  remove:
    - "sensitive_key"
    - "token"
    "#;

    let config: RequestTransformConfig = serde_yaml::from_str(yaml).expect("Failed to parse");
    assert_eq!(config.query.add.len(), 2);
    assert_eq!(config.query.remove.len(), 2);
  }

  #[test]
  fn test_parse_body_patches() {
    let yaml = r#"
body:
  jsonpath:
    "$.metadata.proxied": true
    "$.count": 42
  regex:
    - pattern: "old-value"
      replacement: "new-value"
  operations:
    - op: "add"
      path: "/newField"
      value: "added"
    "#;

    let config: RequestTransformConfig = serde_yaml::from_str(yaml).expect("Failed to parse");
    assert_eq!(config.body.jsonpath.len(), 2);
    assert_eq!(config.body.regex.len(), 1);
    assert_eq!(config.body.operations.len(), 1);
    assert!(!config.body.is_empty());
  }
}
