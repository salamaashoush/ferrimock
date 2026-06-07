//! Global application identity
//!
//! Provides configurable app name so that embedders can set their own
//! branding while mockpit CLI defaults to "mockpit".

use std::sync::OnceLock;

static APP_NAME: OnceLock<String> = OnceLock::new();

/// Set the application name used in HAR exports and other metadata.
/// Must be called before any HAR files are created. Defaults to "mockpit".
pub fn set_app_name(name: impl Into<String>) -> crate::Result<()> {
    APP_NAME
        .set(name.into())
        .map_err(|existing| crate::mp_err!("App name already set to: {existing}"))
}

/// Get the application name. Defaults to "mockpit".
pub fn app_name() -> &'static str {
    APP_NAME.get().map_or("mockpit", |s| s.as_str())
}
