//! Config file parsing — thin NAPI wrapper over mockpit::services::config.

use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Parse a mockpit config file (YAML or JSON). Detects format from extension.
#[napi]
pub fn parse_config_file(file_path: String) -> Result<serde_json::Value> {
    let config = mockpit::services::config::parse_config_file(&file_path)
        .map_err(|e| Error::from_reason(e.to_string()))?;
    serde_json::to_value(&config).map_err(|e| Error::from_reason(e.to_string()))
}

/// Parse a mockpit config from a string with explicit format ("yaml", "json").
#[napi]
pub fn parse_config_string(content: String, format: String) -> Result<serde_json::Value> {
    let config = mockpit::services::config::parse_config_string(&content, &format)
        .map_err(|e| Error::from_reason(e.to_string()))?;
    serde_json::to_value(&config).map_err(|e| Error::from_reason(e.to_string()))
}

/// Auto-discover a mockpit config file in the given directory.
/// Returns the path if found, null otherwise.
#[napi]
pub fn discover_config_file(dir: Option<String>) -> Option<String> {
    let dir = dir.unwrap_or_else(|| ".".into());
    mockpit::services::config::discover_config_file(&dir)
}
