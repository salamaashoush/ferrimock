//! Recording session management utilities

use crate::types::RecordingSession;
use anyhow::Result;
use std::path::Path;

/// Load a recording session from disk
pub async fn load_session(path: impl AsRef<Path>) -> Result<RecordingSession> {
    let content = tokio::fs::read_to_string(path.as_ref()).await?;

    // Try JSON first, then YAML
    if let Ok(session) = serde_json::from_str::<RecordingSession>(&content) {
        return Ok(session);
    }

    if let Ok(session) = serde_yaml::from_str::<RecordingSession>(&content) {
        return Ok(session);
    }

    Err(anyhow::anyhow!(
        "Failed to parse recording file as JSON or YAML"
    ))
}

/// Clone minimal data needed for export (used for auto-export on error)
pub(super) fn create_export_session_name(session_name: &str) -> String {
    use chrono::Utc;
    format!(
        "{}-error-{}",
        session_name,
        Utc::now().format("%Y%m%d-%H%M%S")
    )
}
