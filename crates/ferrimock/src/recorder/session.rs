//! Recording session management utilities

use super::types::{RecordedInteraction, RecordingSession};
use crate::Result;
use std::path::Path;

/// Load a recording session from disk
pub async fn load_session(path: impl AsRef<Path>) -> Result<RecordingSession> {
    let content = tokio::fs::read_to_string(path.as_ref()).await?;

    // Try JSON first, then YAML
    if let Ok(session) = serde_json::from_str::<RecordingSession>(&content) {
        return Ok(session);
    }

    if let Ok(session) = serde_yaml_ng::from_str::<RecordingSession>(&content) {
        return Ok(session);
    }

    Err(crate::mp_err!(
        "Failed to parse recording file as JSON or YAML"
    ))
}

/// Load the request/response pairs a replay can be checked against.
///
/// Accepts a recording session (JSON or YAML) or a HAR file. A saved *mock
/// collection* is not accepted: it keeps only the responses, so nothing in it
/// can say what was asked to produce them.
pub async fn load_interactions(path: impl AsRef<Path>) -> Result<Vec<RecordedInteraction>> {
    let content = tokio::fs::read_to_string(path.as_ref()).await?;

    if let Ok(session) = serde_json::from_str::<RecordingSession>(&content) {
        return Ok(session.interactions);
    }
    if let Ok(session) = serde_yaml_ng::from_str::<RecordingSession>(&content) {
        return Ok(session.interactions);
    }

    let har = crate::config::parse_har(&content).map_err(|_| {
        crate::mp_err!(
            "{} is neither a recording session nor a HAR file; \
             a consolidated mock collection cannot be verified because it does not \
             record the requests",
            path.as_ref().display()
        )
    })?;

    let har::Spec::V1_2(log) = har.log else {
        return Err(crate::mp_err!("Only HAR 1.2 recordings can be replayed"));
    };

    Ok(log
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| super::har::from_har_entry(index, entry))
        .collect())
}

/// Clone minimal data needed for export (used for auto-export on error)
pub fn create_export_session_name(session_name: &str) -> String {
    use chrono::Utc;
    format!(
        "{}-error-{}",
        session_name,
        Utc::now().format("%Y%m%d-%H%M%S")
    )
}
