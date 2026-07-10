//! Mock consolidation service — optimize mock collections by merging patterns.

use crate::consolidator::{ConsolidatorOptions, MockConsolidator};

/// Input for mock consolidation.
#[derive(Debug, Clone)]
pub struct ConsolidateInput {
    /// Input mock collection file path
    pub input: String,
    /// Output format: "json" or "yaml"
    pub format: String,
    /// Minimum similar requests to form a pattern
    pub min_pattern: usize,
    /// Enable template extraction (convert static to dynamic responses)
    pub enable_templates: bool,
}

impl Default for ConsolidateInput {
    fn default() -> Self {
        Self {
            input: String::new(),
            format: "json".into(),
            min_pattern: 3,
            enable_templates: true,
        }
    }
}

/// Result of mock consolidation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConsolidateResult {
    /// Consolidated output content (JSON or YAML)
    pub content: String,
    /// Number of mocks before consolidation
    pub mocks_before: usize,
    /// Number of mocks after consolidation
    pub mocks_after: usize,
    /// Input file size in bytes
    pub input_size: u64,
    /// Output content size in bytes
    pub output_size: u64,
}

/// Consolidate a mock collection file.
pub async fn consolidate(
    input: ConsolidateInput,
) -> Result<ConsolidateResult, crate::FerrimockError> {
    let options = ConsolidatorOptions {
        enable_consolidation: true,
        enable_templates: input.enable_templates,
        min_pattern_threshold: input.min_pattern,
        enable_stateful_pagination: true,
        pagination_storage_key_template: "api.{path}.total".into(),
    };

    let mut consolidator = MockConsolidator::with_options(options);

    let input_size = tokio::fs::metadata(&input.input)
        .await
        .map_or(0, |m| m.len());

    let collection = consolidator
        .consolidate_file(&input.input)
        .await
        .map_err(|e| crate::mp_err!(e))?;

    let stats = consolidator.stats();
    let mocks_before = stats.original_count;
    let mocks_after = stats.consolidated_count;

    let content = match input.format.as_str() {
        "json" => serde_json::to_string_pretty(&collection)?,
        _ => serde_yaml::to_string(&collection)?,
    };

    let output_size = content.len() as u64;

    Ok(ConsolidateResult {
        content,
        mocks_before,
        mocks_after,
        input_size,
        output_size,
    })
}
