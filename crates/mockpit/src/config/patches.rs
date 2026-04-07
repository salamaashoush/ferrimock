//! Shared patch configuration types
//!
//! These types are used by both request and response patch configurations.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// JSON Patch operation (RFC 6902)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct JsonPatchConfig {
    /// Operation: "add", "remove", "replace", "copy", "move", "test"
    pub op: String,
    /// JSON Pointer path
    pub path: String,
    /// Value for add/replace operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    /// Source path for copy/move operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Regex replacement configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RegexReplaceConfig {
    /// Regex pattern
    pub pattern: String,
    /// Replacement string
    pub replacement: String,
}

/// Header patch operations (add/remove headers)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct HeaderPatchesConfig {
    /// Headers to add
    #[serde(default, skip_serializing_if = "FxHashMap::is_empty")]
    #[allow(clippy::disallowed_types)]
    #[cfg_attr(
        feature = "schema",
        schemars(with = "std::collections::HashMap<String, String>")
    )]
    pub add: FxHashMap<String, String>,
    /// Headers to remove (list of header names)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove: Vec<String>,
}

impl HeaderPatchesConfig {
    /// Returns true if no header patches are configured
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}
