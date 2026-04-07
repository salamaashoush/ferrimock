/// Configuration file support for mockpit CLI
///
/// Looks for config in this order:
/// 1. Path from --config flag or MOCKPIT_CONFIG env
/// 2. ./mockpit.toml (current directory)
/// 3. ~/.config/mockpit/config.toml
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)] // Fields are deserialized from config file; will be wired to commands in future
pub struct Config {
    /// Default mock collections directory
    pub collections_dir: Option<String>,
    /// Default recordings directory
    pub recordings_dir: Option<String>,
    /// Default mock server port
    pub port: Option<u16>,
    /// Default mock server host
    pub host: Option<String>,
}

fn find_config_file(explicit_path: Option<&str>) -> Option<PathBuf> {
    // 1. Explicit path
    if let Some(path) = explicit_path {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
        tracing::warn!("Config file not found: {path}");
        return None;
    }

    // 2. Current directory
    let local = Path::new("mockpit.toml");
    if local.exists() {
        return Some(local.to_path_buf());
    }

    // 3. User config directory
    if let Some(config_dir) = dirs::config_dir() {
        let user_config = config_dir.join("mockpit").join("config.toml");
        if user_config.exists() {
            return Some(user_config);
        }
    }

    None
}

pub fn load_config(explicit_path: Option<&str>) -> Config {
    let Some(path) = find_config_file(explicit_path) else {
        return Config::default();
    };

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(config) => {
                tracing::debug!("Loaded config from {}", path.display());
                config
            }
            Err(e) => {
                tracing::warn!("Failed to parse {}: {e}", path.display());
                Config::default()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read {}: {e}", path.display());
            Config::default()
        }
    }
}
