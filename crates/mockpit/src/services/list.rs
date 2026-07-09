//! Mock listing service — list all loaded mock definitions.

use crate::engine::MockRegistry;
use crate::types::MockDefinition;

/// Summary of a single mock definition.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MockSummary {
    pub id: String,
    /// Mock kind: "http", "sse", or "ws".
    pub kind: String,
    pub priority: u32,
    pub enabled: bool,
    pub methods: Vec<String>,
    pub url_patterns: Vec<String>,
    pub status: u16,
    pub has_header_matchers: bool,
    pub has_delay: bool,
    pub scope: Option<String>,
}

impl From<&MockDefinition> for MockSummary {
    fn from(m: &MockDefinition) -> Self {
        Self {
            id: m.id.to_string(),
            kind: m
                .streaming
                .as_ref()
                .map_or("http", crate::types::StreamingResponse::kind)
                .to_string(),
            priority: m.priority,
            enabled: m.enabled,
            methods: m
                .request
                .methods
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            url_patterns: m
                .request
                .url_patterns
                .iter()
                .map(|p| format!("{p:?}"))
                .collect(),
            status: m.response.status.as_u16(),
            has_header_matchers: !m.request.header_matchers.is_empty(),
            has_delay: m.response.delay.is_some(),
            scope: m.scope.as_ref().map(std::string::ToString::to_string),
        }
    }
}

/// Input for listing mocks.
#[derive(Debug, Clone, Default)]
pub struct ListInput {
    /// Mock collections directory (defaults to MOCKS_DIR env or mocks/collections)
    pub mocks_dir: Option<String>,
    /// Filter by collection/ID substring
    pub filter: Option<String>,
}

/// Output of listing mocks.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ListOutput {
    pub mocks: Vec<MockSummary>,
    pub total: usize,
}

/// List all loaded mocks from a directory.
pub async fn list(input: ListInput) -> Result<ListOutput, crate::MockpitError> {
    let dir = input.mocks_dir.unwrap_or_else(|| {
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
    });

    let registry = MockRegistry::new();
    registry
        .load_from_directory(&dir)
        .await
        .map_err(|e| crate::mp_err!(e))?;

    let all_mocks = registry.get_all_mocks();

    let mocks: Vec<MockSummary> = all_mocks
        .iter()
        .filter(|m| input.filter.as_ref().is_none_or(|f| m.id.contains(f)))
        .map(|m| MockSummary::from(m.as_ref()))
        .collect();

    let total = mocks.len();
    Ok(ListOutput { mocks, total })
}

/// List mocks from an existing registry (for use when registry is already loaded).
pub fn list_from_registry(registry: &MockRegistry, filter: Option<&str>) -> ListOutput {
    let all_mocks = registry.get_all_mocks();

    let mocks: Vec<MockSummary> = all_mocks
        .iter()
        .filter(|m| filter.is_none_or(|f| m.id.contains(f)))
        .map(|m| MockSummary::from(m.as_ref()))
        .collect();

    let total = mocks.len();
    ListOutput { mocks, total }
}
