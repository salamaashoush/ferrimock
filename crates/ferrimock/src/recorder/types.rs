//! Types for recording HTTP interactions

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Recorded HTTP interaction (request + response)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedInteraction {
    /// Unique ID for this interaction
    pub id: String,
    /// Timestamp when recorded
    #[serde(serialize_with = "crate::rfc3339::serialize")]
    pub timestamp: jiff::Timestamp,
    /// Recorded request
    pub request: RecordedRequest,
    /// Recorded response
    pub response: RecordedResponse,
    /// Duration of the interaction
    #[serde(with = "duration_serde")]
    pub duration: Duration,
}

/// Recorded HTTP request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedRequest {
    /// HTTP method
    pub method: String,
    /// Request URI/path
    pub uri: String,
    /// Query string
    pub query: Option<String>,
    /// Request headers
    pub headers: Vec<(String, String)>,
    /// Request body (if any)
    pub body: Option<String>,
}

/// Recorded HTTP response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers
    pub headers: Vec<(String, String)>,
    /// Response body
    pub body: String,
}

/// Recording session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSession {
    /// Session ID
    pub id: String,
    /// Session name
    pub name: String,
    /// Creation timestamp
    #[serde(serialize_with = "crate::rfc3339::serialize")]
    pub created_at: jiff::Timestamp,
    /// All recorded interactions
    pub interactions: Vec<RecordedInteraction>,
}

// Custom serialization for Duration
pub mod duration_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_chrono_wrote_reads_back_and_writes_the_same_text() -> serde_json::Result<()> {
        let written = r#"{"id":"s1","name":"acme","created_at":"2024-03-17T05:00:00Z","interactions":[{"id":"i1","timestamp":"2024-03-17T05:00:01.120Z","request":{"method":"GET","uri":"/v1/items","query":null,"headers":[],"body":null},"response":{"status":200,"headers":[],"body":"{}"},"duration":12},{"id":"i2","timestamp":"2024-03-17T05:00:02.123456Z","request":{"method":"GET","uri":"/v1/items","query":null,"headers":[],"body":null},"response":{"status":200,"headers":[],"body":"{}"},"duration":3}]}"#;

        let session: RecordingSession = serde_json::from_str(written)?;

        assert_eq!(session.created_at.as_second(), 1_710_651_600);
        assert_eq!(serde_json::to_string(&session)?, written);
        Ok(())
    }

    #[test]
    fn a_timestamp_written_with_an_offset_reads_as_the_same_instant() -> serde_json::Result<()> {
        let written = r#"{"id":"s1","name":"acme","created_at":"2024-03-17T14:00:00+09:00","interactions":[]}"#;

        let session: RecordingSession = serde_json::from_str(written)?;

        assert_eq!(
            serde_json::to_string(&session)?,
            r#"{"id":"s1","name":"acme","created_at":"2024-03-17T05:00:00Z","interactions":[]}"#
        );
        Ok(())
    }
}
