// schemars(with) attributes reference std::collections::HashMap which clippy disallows
#![cfg_attr(feature = "schema", allow(clippy::disallowed_types))]
//! Configuration parsing for mock definitions
//!
//! This module provides JSON/YAML configuration parsing for defining mock HTTP responses.
//! It's organized into several submodules:
//!
//! - `parser`: Top-level mock collection and mock configuration parsing
//! - `matcher`: Request matching configuration (methods, URLs, headers, body, query)
//! - `response`: Response configuration (status, body, headers, delays, patches)
//! - `patches`: Patch operations for modifying upstream responses
//! - `patterns`: URL pattern parsing utilities (Express-style, glob, regex)
//! - `har`: HAR file loading and conversion to mock configurations

pub mod har;
pub mod matcher;
pub mod parser;
pub mod patches;
pub mod patterns;
pub mod request_transform;
pub mod response;
pub mod template_formatter;

// Re-export commonly used types
pub use har::{DomainFilter, HarLoadOptions, HarLoader};
pub use matcher::{
    BodyMatcherConfig, GraphQLMatchConfig, HeaderMatchConfig, MatchConfig, RequestConfig,
};
pub use parser::{MockCollectionConfig, MockConfig};
pub use patches::{HeaderPatchesConfig, JsonPatchConfig, RegexReplaceConfig};
pub use patterns::{convert_express_to_regex, is_valid_http_method, parse_url_pattern};
pub use request_transform::{BodyPatchesConfig, QueryPatchesConfig, RequestTransformConfig};
pub use response::{
    BodyConfig, ResolvedResponse, ResponseConfig, ResponsePatchesConfig, parse_patches_config,
};

/// Backward-compatibility alias: `ReturnConfig` was renamed to `ResponseConfig`
pub type ReturnConfig = ResponseConfig;

/// Backward-compatibility alias: `PatchesConfig` was renamed to `ResponsePatchesConfig`
pub type PatchesConfig = ResponsePatchesConfig;
pub use template_formatter::format_body;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod patch_tests {
    use super::*;

    #[test]
    fn test_parse_json_patch() {
        let yaml = r#"
mocks:
  - id: test
    match:
      methods: ["GET"]
      url: /test
    patch:
      operations:
        - op: add
          path: /name
          value: test
    "#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        assert_eq!(collection.mocks.len(), 1);

        let mock = &collection.mocks[0];
        let patch = mock.patch.as_ref().expect("patch should exist");

        assert_eq!(patch.operations.len(), 1);
        assert_eq!(patch.operations[0].op, "add");
        assert_eq!(patch.operations[0].path, "/name");
    }

    #[test]
    fn test_parse_jsonpath_patches() {
        let yaml = r#"
mocks:
  - id: test
    match:
      methods: ["GET"]
      url: /test
    patch:
      jsonpath:
        "$.count": 42
        "$.enabled": true
    "#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let patch = collection.mocks[0]
            .patch
            .as_ref()
            .expect("patch should exist");

        assert_eq!(patch.jsonpath.len(), 2);
        assert_eq!(
            patch
                .jsonpath
                .get("$.count")
                .expect("count field should exist"),
            &serde_json::Value::Number(42.into())
        );
        assert_eq!(
            patch
                .jsonpath
                .get("$.enabled")
                .expect("enabled field should exist"),
            &serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn test_parse_regex_patches() {
        let yaml = r#"
mocks:
  - id: test
    match:
      methods: ["GET"]
      url: /test
    patch:
      regex:
        - pattern: Production
          replacement: Development
    "#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let patch = collection.mocks[0]
            .patch
            .as_ref()
            .expect("patch should exist");

        assert_eq!(patch.regex.len(), 1);
        assert_eq!(patch.regex[0].pattern, "Production");
        assert_eq!(patch.regex[0].replacement, "Development");
    }

    #[test]
    fn test_parse_header_patches() {
        let yaml = r#"
mocks:
  - id: test
    match:
      methods: ["GET"]
      url: /test
    patch:
      headers:
        add:
          x-mock: "true"
          x-custom: value
        remove:
          - x-old-header
    "#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let patch = collection.mocks[0]
            .patch
            .as_ref()
            .expect("patch should exist");

        assert_eq!(patch.headers.add.len(), 2);
        assert_eq!(
            patch
                .headers
                .add
                .get("x-mock")
                .expect("x-mock header should exist"),
            "true"
        );
        assert_eq!(patch.headers.remove.len(), 1);
        assert_eq!(patch.headers.remove[0], "x-old-header");
    }

    #[test]
    fn test_parse_combined_patches() {
        let yaml = r#"
mocks:
  - id: test
    match:
      methods: ["POST"]
      url: /test
    patch:
      operations:
        - op: replace
          path: /status
          value: active
      jsonpath:
        "$.verified": true
      regex:
        - pattern: test
          replacement: prod
      headers:
        add:
          location: /resource/1
    "#;

        let collection =
            MockCollectionConfig::from_yaml(yaml).expect("Failed to parse YAML config");
        let patch = collection.mocks[0]
            .patch
            .as_ref()
            .expect("patch should exist");

        assert_eq!(patch.operations.len(), 1);
        assert_eq!(patch.jsonpath.len(), 1);
        assert_eq!(patch.regex.len(), 1);
        assert_eq!(patch.headers.add.len(), 1);
    }
}
