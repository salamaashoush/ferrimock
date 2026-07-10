/// Configuration file support for ferrimock CLI
///
/// Looks for config in this order:
/// 1. Path from --config flag or FERRIMOCK_CONFIG env
/// 2. ./ferrimock.toml (current directory)
/// 3. ~/.config/ferrimock/config.toml
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Default, Deserialize)]
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

static CONFIG: OnceLock<Config> = OnceLock::new();
static QUIET: AtomicBool = AtomicBool::new(false);

/// Install the loaded config as the process-wide default (call once from `main`).
pub fn init(config: Config) {
    let _ = CONFIG.set(config);
}

fn get() -> &'static Config {
    CONFIG.get_or_init(Config::default)
}

/// Set quiet mode (suppresses decorative output; errors still print).
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Whether quiet mode is on.
pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Resolve the mock collections directory: `MOCKS_DIR` env > config > default.
pub fn mocks_dir() -> String {
    std::env::var("MOCKS_DIR")
        .ok()
        .or_else(|| get().collections_dir.clone())
        .unwrap_or_else(|| "mocks/collections".to_string())
}

/// Configured default server port, if any.
pub fn default_port() -> Option<u16> {
    get().port
}

/// Configured default server host, if any.
pub fn default_host() -> Option<String> {
    get().host.clone()
}

/// Print a line of decorative output unless quiet mode is on.
#[macro_export]
macro_rules! say {
    ($($arg:tt)*) => {
        if !$crate::config::is_quiet() {
            println!($($arg)*);
        }
    };
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
    let local = Path::new("ferrimock.toml");
    if local.exists() {
        return Some(local.to_path_buf());
    }

    // 3. User config directory
    if let Some(config_dir) = dirs::config_dir() {
        let user_config = config_dir.join("ferrimock").join("config.toml");
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
