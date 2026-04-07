//! Types for recording HTTP interactions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Recorded HTTP interaction (request + response)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedInteraction {
    /// Unique ID for this interaction
    pub id: String,
    /// Timestamp when recorded
    pub timestamp: DateTime<Utc>,
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
    pub created_at: DateTime<Utc>,
    /// All recorded interactions
    pub interactions: Vec<RecordedInteraction>,
}

// Custom serialization for Duration
pub(crate) mod duration_serde {
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
