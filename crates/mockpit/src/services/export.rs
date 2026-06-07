//! Export mocks to HAR format service.

use crate::engine::MockRegistry;

/// Input for exporting mocks to HAR.
#[derive(Debug, Clone, Default)]
pub struct ExportInput {
    pub mocks_dir: Option<String>,
    pub filter: Option<String>,
}

/// Result of HAR export.
#[derive(Debug, Clone)]
pub struct ExportResult {
    pub content: String,
    pub mocks_exported: usize,
}

/// Export mocks to HAR format.
pub async fn export(input: ExportInput) -> Result<ExportResult, crate::MockpitError> {
    let dir = input.mocks_dir.unwrap_or_else(|| {
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
    });

    let registry = MockRegistry::new();
    registry
        .load_from_directory(&dir)
        .await
        .map_err(|e| crate::mp_err!(e))?;

    let all_mocks = registry.get_all_mocks();

    let filtered: Vec<_> = if let Some(ref filter) = input.filter {
        all_mocks
            .iter()
            .filter(|m| m.id.contains(filter))
            .cloned()
            .collect()
    } else {
        all_mocks
    };

    let mocks_exported = filtered.len();
    let har = crate::engine::export_mocks_to_har(&filtered)?;
    let content = serde_json::to_string_pretty(&har)?;

    Ok(ExportResult {
        content,
        mocks_exported,
    })
}
