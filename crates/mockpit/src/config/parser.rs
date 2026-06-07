//! Mock collection configuration and format parsing

use super::har::HarLoader;
use super::matcher::MatchConfig;
use super::request_transform::RequestTransformConfig;
use super::response::{
    ResponseConfig, ResponsePatchesConfig, parse_duration, parse_patches_config,
};
use crate::types::MockDefinition;
use crate::Result;
use lean_string::LeanString;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Mock collection configuration (top-level structure)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MockCollectionConfig {
    /// Collection metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default = "default_enabled", skip_serializing_if = "is_true")]
    pub enabled: bool,

    /// Collection-level variables available in all mock templates as {{ vars.key }}
    /// These shadow global vars and are shadowed by mock-level vars
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::HashMap<String, serde_json::Value>>")
    )]
    pub vars: Option<serde_json::Map<String, serde_json::Value>>,

    /// List of mock definitions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mocks: Vec<MockConfig>,
}

impl MockCollectionConfig {
    /// Parse from JSON string
    pub fn from_json(content: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(content)
    }

    /// Parse from YAML string
    pub fn from_yaml(content: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(content)
    }

    /// Parse from file (supports JSON, YAML, HAR based on extension)
    ///
    /// HAR files are automatically converted to static mocks with exact URL matching.
    pub async fn from_file(path: impl Into<PathBuf>) -> Result<Self, crate::MockpitError> {
        let path = path.into();
        let content = tokio::fs::read_to_string(&path).await?;

        // Determine format from extension
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| crate::mp_err!("File has no extension"))?;

        match extension {
            "json" => {
                // Auto-detect HAR files by checking for "log" top-level key
                if content.trim_start().starts_with(r#"{"log":"#) || content.contains(r#""log":"#) {
                    Self::from_har(&content).await
                } else {
                    Ok(Self::from_json(&content)?)
                }
            }
            "har" => Self::from_har(&content).await,
            "yaml" | "yml" => Ok(Self::from_yaml(&content)?),
            _ => Err(crate::mp_err!("Unsupported file format: {extension}")),
        }
    }

    /// Parse from HAR (HTTP Archive) file content
    ///
    /// Converts HAR entries to static mocks with exact URL matching.
    /// Use consolidator afterwards for pattern detection and optimization.
    pub async fn from_har(content: &str) -> Result<Self, crate::MockpitError> {
        let har = serde_json::from_str(content)?;
        let loader = HarLoader::new();
        let mocks = loader.convert_har_to_mocks(har).await?;

        Ok(Self {
            name: Some("Mocks from HAR file".to_string()),
            description: Some(
                "Auto-converted from HAR file - all entries loaded as static mocks".to_string(),
            ),
            enabled: true,
            vars: None,
            mocks,
        })
    }

    /// Convert to mock definitions
    pub async fn into_mock_definitions(self) -> crate::Result<Vec<MockDefinition>> {
        self.into_mock_definitions_with_dir(None, None).await
    }

    /// Convert to mock definitions with config directory for resolving relative file paths
    /// and optional global vars to merge with collection-level and mock-level vars.
    pub async fn into_mock_definitions_with_dir(
        self,
        config_dir: Option<&std::path::Path>,
        global_vars: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> crate::Result<Vec<MockDefinition>> {
        // Merge: global <- collection
        let collection_merged = merge_vars(global_vars, self.vars.as_ref());

        let mut definitions = Vec::new();
        for config in self.mocks {
            // Merge: collection_merged <- mock
            let final_vars = merge_vars(collection_merged.as_ref(), config.vars.as_ref());
            let mut def = config.into_mock_definition_with_dir(config_dir).await?;
            def.vars = final_vars;
            definitions.push(def);
        }
        Ok(definitions)
    }
}

fn default_enabled() -> bool {
    true
}

// These functions take references because serde's skip_serializing_if requires &T -> bool
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_true(v: &bool) -> bool {
    *v
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_priority(v: &u32) -> bool {
    *v == 100
}

/// Merge two optional variable maps. Lower-level (overlay) values shadow higher-level (base) values.
fn merge_vars(
    base: Option<&serde_json::Map<String, serde_json::Value>>,
    overlay: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(o)) => Some(o.clone()),
        (Some(b), Some(o)) => {
            let mut merged = b.clone();
            for (k, v) in o {
                merged.insert(k.clone(), v.clone());
            }
            Some(merged)
        }
    }
}

/// Single mock configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MockConfig {
    /// Unique identifier
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    pub id: LeanString,

    /// Human-readable description of what this mock does
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Priority for matching (higher = matched first)
    #[serde(
        default = "default_priority",
        skip_serializing_if = "is_default_priority"
    )]
    pub priority: u32,

    /// Enabled flag
    #[serde(default = "default_enabled", skip_serializing_if = "is_true")]
    pub enabled: bool,

    /// Optional scope for test isolation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    pub scope: Option<LeanString>,

    /// Mock-level variables that shadow collection-level and global vars
    /// Accessible in templates as {{ vars.key }}
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "Option<std::collections::HashMap<String, serde_json::Value>>")
    )]
    pub vars: Option<serde_json::Map<String, serde_json::Value>>,

    /// Flat match configuration (new syntax)
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub match_config: Option<MatchConfig>,

    /// Request transformations (implies passthrough/PatchUpstream mode)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestTransformConfig>,

    /// Response definition (FullMock)
    #[serde(
        rename = "response",
        alias = "return",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub response_config: Option<ResponseConfig>,

    /// Response patches applied to upstream responses (PatchUpstream mode)
    /// Cannot be combined with a full mock response (body/template/json/file/template_file)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<ResponsePatchesConfig>,

    /// Delay before responding (e.g., "100ms", "2s", "500us")
    /// Works in all modes: full mock, passthrough, and patch.
    /// When set alone (no response/patch/request), enables passthrough with delay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<String>,
}

impl MockConfig {
    /// Convert to a MockDefinition
    pub async fn into_mock_definition(self) -> crate::Result<MockDefinition> {
        self.into_mock_definition_with_dir(None).await
    }

    /// Convert to a MockDefinition with config directory for resolving relative file paths
    pub async fn into_mock_definition_with_dir(
        self,
        config_dir: Option<&std::path::Path>,
    ) -> crate::Result<MockDefinition> {
        let match_config = self
            .match_config
            .ok_or_else(|| crate::mp_err!("Missing 'match' configuration"))?;

        let request_config = match_config.into_request_config();

        // Determine if we have request transforms
        let has_request_transforms = self.request.as_ref().is_some_and(|r| !r.is_empty());

        // Resolve the response config
        let response_config = self.response_config;

        // Determine if this is a FullMock or PatchUpstream based on the heuristic:
        // - response.body or response.json set => FullMock
        // - response = 'template string' => FullMock
        // - response.NNN = "body" (status shortcut) => FullMock
        // - request.* set (any field) => PatchUpstream
        // - patch.* set => PatchUpstream
        // - Only response.status/delay/headers (no body/json) => FullMock
        let is_full_mock = response_config
            .as_ref()
            .is_some_and(super::response::ResponseConfig::is_full_mock);

        // Validation: conflicting combinations
        if is_full_mock && has_request_transforms {
            return Err(
        "Cannot combine full mock body (response.body/response.json) with request transforms. \
         Use either a full mock OR passthrough with request transforms."
          .to_string().into(),
      );
        }

        // Validation: patch + full mock response is invalid
        if is_full_mock && self.patch.is_some() {
            return Err("Cannot combine top-level `patch` with full mock response. \
         Use either `patch` (upstream passthrough) or `response` (full mock), not both."
                .to_string().into());
        }

        // Build the resolved response
        let resolved_response = response_config.unwrap_or_default().into_resolved_response();

        // Build response generator
        let mut response = resolved_response
            .into_response_generator_with_dir(config_dir)
            .await?;

        // Apply top-level delay
        if let Some(ref delay_str) = self.delay {
            let delay =
                parse_duration(delay_str).map_err(|e| crate::mp_err!("Invalid top-level delay: {e}"))?;
            response = response.with_delay(delay);
        }

        // Apply top-level patches if configured
        if let Some(patches_config) = self.patch {
            let patch_ops = parse_patches_config(patches_config)?;
            if !patch_ops.is_empty() {
                response = response.with_mode(crate::types::ResponseMode::Patch {
                    operations: patch_ops,
                });
            }
        }

        // Delay-only passthrough: if top-level delay is set but no full mock body and no patches,
        // enter PatchUpstream mode with empty operations for upstream passthrough
        let entered_patch_mode = matches!(response.mode, crate::types::ResponseMode::Patch { .. });
        if self.delay.is_some() && !is_full_mock && !entered_patch_mode && !has_request_transforms {
            response = response.with_mode(crate::types::ResponseMode::Patch { operations: vec![] });
        }

        // Build request transforms if present
        let request_transforms = if has_request_transforms {
            let rt = self
                .request
                .ok_or_else(|| crate::mp_err!("request transforms missing"))?;
            Some(build_request_transforms(rt)?)
        } else {
            None
        };

        Ok(MockDefinition {
            id: self.id,
            priority: self.priority,
            enabled: self.enabled,
            once: false,
            scope: self.scope,
            source_file: None,
            request_transforms,
            request: request_config.into_request_matcher()?,
            response,
            vars: None,
        })
    }
}

fn default_priority() -> u32 {
    100
}

/// Convert RequestTransformConfig into ResolvedRequestTransforms
fn build_request_transforms(
    config: RequestTransformConfig,
) -> crate::Result<crate::types::ResolvedRequestTransforms> {
    use crate::types::{RequestPatch, ResolvedRequestTransforms, UpstreamOptions};

    let mut patches = Vec::new();

    // Header patches
    for (name, value) in config.headers.add {
        patches.push(RequestPatch::HeaderAdd { name, value });
    }
    for name in config.headers.remove {
        patches.push(RequestPatch::HeaderRemove { name });
    }

    // Query patches
    for (name, value) in config.query.add {
        patches.push(RequestPatch::QueryAdd { name, value });
    }
    for name in config.query.remove {
        patches.push(RequestPatch::QueryRemove { name });
    }

    // Body patches - JSONPath
    for (path, value) in config.body.jsonpath {
        patches.push(RequestPatch::JsonPath { path, value });
    }

    // Body patches - RFC 6902
    if !config.body.operations.is_empty() {
        let json_patch_str = serde_json::to_string(&config.body.operations)
            .map_err(|e| crate::mp_err!("Failed to serialize JSON Patch operations: {e}"))?;
        let json_patch: json_patch::Patch = serde_json::from_str(&json_patch_str)
            .map_err(|e| crate::mp_err!("Failed to parse JSON Patch operations: {e}"))?;
        patches.push(RequestPatch::JsonPatch(json_patch));
    }

    // Body patches - Regex
    for regex_config in config.body.regex {
        let pattern = regex::Regex::new(&regex_config.pattern)
            .map_err(|e| crate::mp_err!("Invalid regex pattern '{}': {}", regex_config.pattern, e))?;
        patches.push(RequestPatch::RegexReplace {
            pattern,
            replacement: regex_config.replacement,
        });
    }

    // Parse durations
    let pre_delay = config.delay.map(|d| parse_duration(&d)).transpose()?;

    let timeout = config.timeout.map(|t| parse_duration(&t)).transpose()?;

    Ok(ResolvedRequestTransforms {
        patches,
        pre_delay,
        upstream_options: UpstreamOptions {
            timeout,
            forward_to: config.forward_to,
        },
        rewrite_path: config.rewrite_path,
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_mock_config() {
        let yaml = r#"
mocks:
  - id: test-mock
    priority: 100
    match:
      methods: ["GET"]
      url: /api/users
    response:
      status: 200
      body: '{"success": true}'
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.mocks.len(), 1);
        assert_eq!(config.mocks[0].id, "test-mock");
        assert_eq!(config.mocks[0].priority, 100);
    }

    #[tokio::test]
    async fn test_mock_config_with_headers() {
        let yaml = r#"
mocks:
  - id: test-mock
    match:
      methods: ["POST"]
      url: /api/users
      headers:
        content-type: application/json
    response:
      status: 201
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = config.mocks[0]
            .clone()
            .into_mock_definition()
            .await
            .expect("Failed to convert to mock definition");

        assert_eq!(mock_def.request.header_matchers.len(), 1);
    }

    #[tokio::test]
    async fn test_mock_config_with_delay() {
        let yaml = r#"
mocks:
  - id: test-mock
    match:
      url: /api/users
    delay: 100ms
    response:
      status: 200
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = config.mocks[0]
            .clone()
            .into_mock_definition()
            .await
            .expect("Failed to convert to mock definition");

        assert_eq!(
            mock_def.response.delay,
            Some(std::time::Duration::from_millis(100))
        );
    }

    #[test]
    fn test_mock_collection_metadata() {
        let yaml = r#"
name: User API Mocks
description: Mock responses for user endpoints
enabled: true
mocks:
  - id: test-mock
    match:
      url: /test
    response:
      status: 200
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.name, Some("User API Mocks".to_string()));
        assert_eq!(
            config.description,
            Some("Mock responses for user endpoints".to_string())
        );
        assert!(config.enabled);
    }

    #[test]
    fn test_mock_config_default_priority() {
        let yaml = r#"
id: test
match:
  url: /test
response:
  body: "{}"
"#;

        let config: MockConfig = serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        assert_eq!(config.priority, 100);
    }

    #[test]
    fn test_mock_config_default_enabled() {
        let yaml = r#"
id: test
match:
  url: /test
response:
  body: "{}"
"#;

        let config: MockConfig = serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        assert!(config.enabled);
    }

    #[tokio::test]
    async fn test_complete_mock_with_matchers_and_query() {
        let yaml = r#"
mocks:
  - id: advanced-mock
    priority: 100
    match:
      methods: ["POST"]
      url: /api/data
      body:
        "@important": true
      query:
        auth: "true"
    response:
      status: 200
      body: '{"success": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert!(mock_def[0].request.body_matcher.is_some());
        assert_eq!(mock_def[0].request.query_matchers.len(), 1);
    }

    // ============================================================================
    // Ultra-flat syntax tests
    // ============================================================================

    #[tokio::test]
    async fn test_ultra_flat_string_match() {
        let yaml = r#"
mocks:
  - id: ultra-flat
    match: "GET /api/users"
    response:
      status: 200
      body: '{"users": []}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert_eq!(mock_def[0].request.methods.len(), 1);
        assert_eq!(mock_def[0].request.methods[0], http::Method::GET);
        assert_eq!(mock_def[0].request.url_patterns.len(), 1);
    }

    #[tokio::test]
    async fn test_method_as_key_syntax() {
        let yaml = r#"
mocks:
  - id: method-key
    match:
      GET: /api/health
    response:
      status: 200
      body: '{"healthy": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert_eq!(mock_def[0].request.methods.len(), 1);
        assert_eq!(mock_def[0].request.methods[0], http::Method::GET);
        assert_eq!(mock_def[0].request.url_patterns.len(), 1);
    }

    #[tokio::test]
    async fn test_multiple_method_shortcuts() {
        let yaml = r#"
mocks:
  - id: multi-methods
    match:
      POST: /api/users
      PUT: /api/users/:id
    response:
      status: 200
      body: "{}"
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");

        // Should have 2 methods and 2 URLs
        assert_eq!(mock_def[0].request.methods.len(), 2);
        assert!(mock_def[0].request.methods.contains(&http::Method::POST));
        assert!(mock_def[0].request.methods.contains(&http::Method::PUT));
        assert_eq!(mock_def[0].request.url_patterns.len(), 2);
    }

    #[tokio::test]
    async fn test_status_as_key_syntax() {
        let yaml = r#"
mocks:
  - id: status-key
    match: "GET /api/simple"
    response:
      "200": '{"success": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");
        assert_eq!(mock_def.len(), 1);
        assert_eq!(mock_def[0].response.status.as_u16(), 200);
    }

    #[tokio::test]
    async fn test_status_404_as_key() {
        let yaml = r#"
mocks:
  - id: not-found
    match: "GET /api/missing"
    response:
      "404": '{"error": "not found"}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");

        assert_eq!(mock_def[0].response.status.as_u16(), 404);
    }

    // ============================================================================
    // merge_vars tests
    // ============================================================================

    #[test]
    fn test_merge_vars_both_none() {
        assert!(merge_vars(None, None).is_none());
    }

    #[test]
    fn test_merge_vars_base_only() {
        let mut base = serde_json::Map::new();
        base.insert("key".to_string(), serde_json::json!("value"));
        let result = merge_vars(Some(&base), None);
        assert_eq!(result, Some(base));
    }

    #[test]
    fn test_merge_vars_overlay_only() {
        let mut overlay = serde_json::Map::new();
        overlay.insert("key".to_string(), serde_json::json!("value"));
        let result = merge_vars(None, Some(&overlay));
        assert_eq!(result, Some(overlay));
    }

    #[test]
    fn test_merge_vars_overlay_shadows() {
        let mut base = serde_json::Map::new();
        base.insert("color".to_string(), serde_json::json!("red"));
        let mut overlay = serde_json::Map::new();
        overlay.insert("color".to_string(), serde_json::json!("blue"));

        let result = merge_vars(Some(&base), Some(&overlay)).unwrap();
        assert_eq!(result.get("color").unwrap(), &serde_json::json!("blue"));
    }

    #[test]
    fn test_merge_vars_disjoint_keys() {
        let mut base = serde_json::Map::new();
        base.insert("a".to_string(), serde_json::json!(1));
        let mut overlay = serde_json::Map::new();
        overlay.insert("b".to_string(), serde_json::json!(2));

        let result = merge_vars(Some(&base), Some(&overlay)).unwrap();
        assert_eq!(result.get("a").unwrap(), &serde_json::json!(1));
        assert_eq!(result.get("b").unwrap(), &serde_json::json!(2));
    }

    #[test]
    fn test_collection_vars_parsed() {
        let yaml = r#"
vars:
  api_base: "https://api.example.com"
  version: 2
mocks:
  - id: test
    match:
      url: /test
    response:
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML");
        let vars = config.vars.unwrap();
        assert_eq!(
            vars.get("api_base").unwrap(),
            &serde_json::json!("https://api.example.com")
        );
        assert_eq!(vars.get("version").unwrap(), &serde_json::json!(2));
    }

    #[tokio::test]
    async fn test_mock_level_vars_shadow_collection() {
        let yaml = r#"
vars:
  color: red
  size: 10
mocks:
  - id: test
    vars:
      color: blue
    match:
      url: /test
    response:
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML");
        let defs = config
            .into_mock_definitions()
            .await
            .expect("Failed to convert");
        let vars = defs[0].vars.as_ref().unwrap();
        // Mock-level "color" should shadow collection-level
        assert_eq!(vars.get("color").unwrap(), &serde_json::json!("blue"));
        // Collection-level "size" should be inherited
        assert_eq!(vars.get("size").unwrap(), &serde_json::json!(10));
    }

    #[tokio::test]
    async fn test_global_vars_cascade() {
        let yaml = r#"
vars:
  from_collection: true
  shared: "collection"
mocks:
  - id: test
    vars:
      shared: "mock"
      from_mock: true
    match:
      url: /test
    response:
      body: "{}"
"#;

        let config = MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML");
        let mut global_vars = serde_json::Map::new();
        global_vars.insert("from_global".to_string(), serde_json::json!(true));
        global_vars.insert("shared".to_string(), serde_json::json!("global"));

        let defs = config
            .into_mock_definitions_with_dir(None, Some(&global_vars))
            .await
            .expect("Failed to convert");
        let vars = defs[0].vars.as_ref().unwrap();

        // Mock-level wins for "shared"
        assert_eq!(vars.get("shared").unwrap(), &serde_json::json!("mock"));
        // Collection-level "from_collection" inherited
        assert_eq!(
            vars.get("from_collection").unwrap(),
            &serde_json::json!(true)
        );
        // Global-level "from_global" inherited
        assert_eq!(vars.get("from_global").unwrap(), &serde_json::json!(true));
        // Mock-level "from_mock" present
        assert_eq!(vars.get("from_mock").unwrap(), &serde_json::json!(true));
    }

    #[test]
    fn test_invalid_match_string_format() {
        let yaml = r#"
mocks:
  - id: invalid
    match: "GET"
    response:
      body: "{}"
"#;

        let result = serde_yaml::from_str::<MockCollectionConfig>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_http_method() {
        let yaml = r#"
mocks:
  - id: invalid
    match: "INVALID /api/test"
    response:
      body: "{}"
"#;

        let result = serde_yaml::from_str::<MockCollectionConfig>(yaml);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_combined_ultra_flat_syntax() {
        let yaml = r#"
mocks:
  - id: combined
    match: "POST /api/users/:id"
    response:
      "201": '{"id": "{{ captures.id }}", "created": true}'
"#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let mock_def = collection
            .into_mock_definitions()
            .await
            .expect("Failed to convert to mock definitions");

        assert_eq!(mock_def[0].request.methods[0], http::Method::POST);
        assert_eq!(mock_def[0].response.status.as_u16(), 201);
        // URL pattern should be parsed as Express-style
        assert_eq!(mock_def[0].request.url_patterns.len(), 1);
    }
}
