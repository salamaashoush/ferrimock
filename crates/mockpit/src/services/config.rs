//! Mockpit configuration file parsing.
//!
//! Universal config format supported across all ecosystems.
//! Supports YAML, JSON. TS/JS config files are handled by the Node.js layer.

use serde::{Deserialize, Serialize};

/// Mockpit configuration.
///
/// Can be defined in `mockpit.config.yaml`, `mockpit.config.json`,
/// or (via the Node.js layer) `mockpit.config.ts` / `mockpit.config.js`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MockpitConfig {
    /// Port to listen on (default: 3006)
    pub port: Option<u16>,
    /// Host to bind to (default: "127.0.0.1")
    pub host: Option<String>,
    /// Directory containing mock collection files (default: "mocks/collections")
    #[serde(alias = "mocks_dir")]
    pub mocks_dir: Option<String>,
    /// Additional mock files to load (YAML/JSON/HAR)
    #[serde(alias = "mock_files")]
    pub mock_files: Option<Vec<String>>,
    /// Enable CORS headers
    pub cors: Option<bool>,
    /// Watch for file changes and hot-reload
    pub watch: Option<bool>,
    /// Enable verbose logging
    pub verbose: Option<bool>,
    /// Log mock match details for every request
    #[serde(alias = "log_matches")]
    pub log_matches: Option<bool>,
}

/// Parse a config file from disk. Detects format from extension.
pub fn parse_config_file(path: &str) -> Result<MockpitConfig, anyhow::Error> {
    let path = std::path::Path::new(path);
    anyhow::ensure!(path.exists(), "Config file not found: {}", path.display());

    let content = std::fs::read_to_string(path)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    parse_config_string(&content, ext)
}

/// Parse a config from a string with explicit format.
pub fn parse_config_string(content: &str, format: &str) -> Result<MockpitConfig, anyhow::Error> {
    match format {
        "json" => Ok(serde_json::from_str(content)?),
        "yaml" | "yml" => Ok(serde_yaml::from_str(content)?),
        other => anyhow::bail!("Unsupported config format: .{other} (use .yaml, .yml, or .json)"),
    }
}

/// Auto-discover a config file in the given directory.
///
/// Searches for `mockpit.config.{yaml,yml,json,ts,js,mjs,mts}` in order.
/// Returns the path if found, None otherwise.
pub fn discover_config_file(dir: &str) -> Option<String> {
    let dir = std::path::Path::new(dir);
    let names = [
        "mockpit.config.ts",
        "mockpit.config.js",
        "mockpit.config.mjs",
        "mockpit.config.mts",
        "mockpit.config.yaml",
        "mockpit.config.yml",
        "mockpit.config.json",
    ];

    for name in &names {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }

    None
}
