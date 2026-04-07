//! Type definitions for the mock management API

use mockpit_config::MockConfig;
use lean_string::LeanString;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

// ============================================================================
// Mock Management Types
// ============================================================================

/// Mock request that accepts config syntax directly
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockRequest {
  /// Uses MockConfig directly - flat or structured syntax
  #[serde(flatten)]
  pub config: MockConfig,
}

/// Response for mock operations
#[derive(Debug, Serialize)]
pub struct MockOperationResponse {
  pub success: bool,
  pub mock_id: LeanString,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub created: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub message: Option<String>,
}

/// Patch mock request (partial update)
#[derive(Debug, Deserialize)]
pub struct PatchMockRequest {
  pub changes: serde_json::Value,
}

// ============================================================================
// Bulk Operations Types
// ============================================================================

/// Bulk operation request
#[derive(Debug, Deserialize)]
pub struct BulkOperationRequest {
  pub operations: Vec<BulkOperation>,
  #[serde(default)]
  pub atomic: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum BulkOperation {
  Create {
    mock: MockConfig,
  },
  Update {
    id: String,
    mock: MockConfig,
  },
  Patch {
    id: String,
    changes: serde_json::Value,
  },
  Delete {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    filter: Option<String>,
  },
  Enable {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    ids: Option<Vec<String>>,
    #[serde(default)]
    filter: Option<String>,
  },
  Disable {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    ids: Option<Vec<String>>,
    #[serde(default)]
    filter: Option<String>,
  },
}

/// Bulk operation response
#[derive(Debug, Serialize)]
pub struct BulkOperationResponse {
  pub success: bool,
  pub results: Vec<BulkOpResult>,
}

#[derive(Debug, Serialize)]
pub struct BulkOpResult {
  pub op: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub id: Option<LeanString>,
  pub success: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub affected: Option<usize>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
}

// ============================================================================
// Inspector Types
// ============================================================================

/// Inspector request
#[derive(Debug, Deserialize)]
pub struct InspectRequest {
  pub method: String,
  pub path: String,
  #[serde(default)]
  pub query: Option<String>,
  #[serde(default)]
  pub headers: Option<FxHashMap<String, String>>,
  #[serde(default)]
  pub body: Option<String>,
}

/// Inspector response
#[derive(Debug, Serialize)]
pub struct InspectResponse {
  #[serde(skip_serializing_if = "Option::is_none")]
  pub matched: Option<MatchedMock>,
  pub evaluated: Vec<EvaluatedMock>,
  pub execution_time_us: u64,
  pub cache_hit: bool,
}

#[derive(Debug, Serialize)]
pub struct MatchedMock {
  pub id: LeanString,
  pub priority: u32,
  pub score: u32,
  pub captures: FxHashMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct EvaluatedMock {
  pub id: LeanString,
  pub priority: u32,
  pub matched: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub reason: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub match_details: Option<MatchDetails>,
}

#[derive(Debug, Serialize)]
pub struct MatchDetails {
  pub method: String,
  pub url: String,
  pub headers: String,
  pub query: String,
  pub body: String,
}

// ============================================================================
// Status & Metrics Types
// ============================================================================

/// System status response
#[derive(Debug, Serialize)]
pub struct StatusResponse {
  pub enabled: bool,
  pub total_mocks: usize,
  pub enabled_mocks: usize,
  pub disabled_mocks: usize,
  pub scopes: ScopeStatus,
  // Top-level recording fields for backwards compatibility
  pub recording_enabled: bool,
  pub recordings_count: usize,
  // Nested recording object (optional, for detailed info)
  #[serde(skip_serializing_if = "Option::is_none")]
  pub recording: Option<RecordingStatus>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub call_tracking: Option<CallTrackingStatus>,
}

#[derive(Debug, Serialize)]
pub struct ScopeStatus {
  pub total: usize,
  pub active: Vec<LeanString>,
}

#[derive(Debug, Serialize)]
pub struct RecordingStatus {
  pub enabled: bool,
  pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct CallTrackingStatus {
  pub enabled_mocks: usize,
  pub total_calls: usize,
}

// ============================================================================
// Persistence Store Types
// ============================================================================

/// Store get all response
#[derive(Debug, Serialize)]
pub struct StoreGetAllResponse {
  pub store: FxHashMap<String, serde_json::Value>,
  pub metadata: StoreMetadata,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  pub keys_with_ttl: Vec<KeyTtlInfo>,
}

#[derive(Debug, Serialize)]
pub struct StoreMetadata {
  pub total_keys: usize,
  pub memory_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct KeyTtlInfo {
  pub key: String,
  pub ttl_seconds: u64,
  pub expires_at: String,
}

/// Store set value request
#[derive(Debug, Deserialize)]
pub struct StoreSetRequest {
  pub value: serde_json::Value,
  #[serde(default)]
  pub ttl_seconds: Option<u64>,
}

/// Store delete response
#[derive(Debug, Serialize)]
pub struct StoreDeleteResponse {
  pub success: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub deleted: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub keys_deleted: Option<usize>,
}

// ============================================================================
// Query Language Types
// ============================================================================

/// Query filter
#[derive(Debug, Clone)]
pub struct QueryFilter {
  pub field: String,
  pub operator: FilterOperator,
  pub value: String,
}

#[derive(Debug, Clone)]
pub enum FilterOperator {
  Equal,
  NotEqual,
  GreaterThan,
  LessThan,
  GreaterOrEqual,
  LessOrEqual,
  Regex,
  StartsWith,
  EndsWith,
  Contains,
}

impl std::str::FromStr for FilterOperator {
  type Err = String;

  fn from_str(op: &str) -> Result<Self, Self::Err> {
    match op {
      "=" => Ok(Self::Equal),
      "!=" => Ok(Self::NotEqual),
      ">" => Ok(Self::GreaterThan),
      "<" => Ok(Self::LessThan),
      ">=" => Ok(Self::GreaterOrEqual),
      "<=" => Ok(Self::LessOrEqual),
      "~=" => Ok(Self::Regex),
      "^=" => Ok(Self::StartsWith),
      "$=" => Ok(Self::EndsWith),
      "*=" => Ok(Self::Contains),
      _ => Err(format!("Unknown operator: {op}")),
    }
  }
}
