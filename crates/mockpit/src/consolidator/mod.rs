//! Smart consolidation engine for recorded mocks
//!
//! This module provides intelligent consolidation of recorded mock interactions
//! to dramatically reduce file size while maintaining behavioral accuracy.

pub mod analysis;
pub mod pattern;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::needless_collect
)]
mod tests;

use analysis::{ResponseAnalysis, ResponseAnalyzer};
use anyhow::{Context, Result};
use crate::codegen::TemplateGenerator;
use crate::config::{MockCollectionConfig, MockConfig, ReturnConfig};
use pattern::PatternDetector;
use rustc_hash::FxHashMap;
use std::path::Path;
use std::path::PathBuf;

/// Consolidation statistics
#[derive(Debug, Clone)]
pub struct ConsolidationStats {
    pub original_count: usize,
    pub consolidated_count: usize,
    pub reduction_ratio: f64,
    pub patterns_detected: usize,
    pub duplicates_removed: usize,
    pub templates_created: usize,
}

impl ConsolidationStats {
    pub fn print_report(&self) {
        println!("\n╔══════════════════════════════════════════════════╗");
        println!("║     Mock Consolidation Report                   ║");
        println!("╠══════════════════════════════════════════════════╣");
        println!(
            "║  Original mocks:        {:>6}                  ║",
            self.original_count
        );
        println!(
            "║  Consolidated mocks:    {:>6}                  ║",
            self.consolidated_count
        );
        println!(
            "║  Reduction ratio:       {:>5.1}%                 ║",
            self.reduction_ratio * 100.0
        );
        println!("║  ─────────────────────────────────────────────  ║");
        println!(
            "║  Patterns detected:     {:>6}                  ║",
            self.patterns_detected
        );
        println!(
            "║  Duplicates removed:    {:>6}                  ║",
            self.duplicates_removed
        );
        println!(
            "║  Templates created:     {:>6}                  ║",
            self.templates_created
        );
        println!("╚══════════════════════════════════════════════════╝\n");
    }
}

/// Consolidator configuration options
#[derive(Debug, Clone)]
pub struct ConsolidatorOptions {
    /// Enable pattern consolidation
    pub enable_consolidation: bool,
    /// Enable template extraction for variable responses
    pub enable_templates: bool,
    /// Minimum number of similar requests to form a pattern
    pub min_pattern_threshold: usize,
    /// Enable stateful pagination using persistent storage
    pub enable_stateful_pagination: bool,
    /// Template for storage key pattern (e.g., "api.{path}.total")
    pub pagination_storage_key_template: String,
}

impl Default for ConsolidatorOptions {
    fn default() -> Self {
        Self {
            enable_consolidation: true,
            enable_templates: true,
            min_pattern_threshold: 3,
            enable_stateful_pagination: true,
            pagination_storage_key_template: "api.{path}.total".to_string(),
        }
    }
}

/// Main consolidation engine
pub struct MockConsolidator {
    options: ConsolidatorOptions,
    stats: ConsolidationStats,
    response_analyzer: ResponseAnalyzer,
    template_generator: TemplateGenerator,
}

impl MockConsolidator {
    /// Create a new consolidator with default options
    pub fn new() -> Self {
        Self::with_options(ConsolidatorOptions::default())
    }

    /// Create a new consolidator with custom options
    pub fn with_options(options: ConsolidatorOptions) -> Self {
        let response_analyzer = ResponseAnalyzer::new(options.enable_stateful_pagination);
        let template_generator =
            TemplateGenerator::new(options.pagination_storage_key_template.clone());

        Self {
            options,
            stats: ConsolidationStats {
                original_count: 0,
                consolidated_count: 0,
                reduction_ratio: 0.0,
                patterns_detected: 0,
                duplicates_removed: 0,
                templates_created: 0,
            },
            response_analyzer,
            template_generator,
        }
    }

    /// Consolidate a mock collection from file
    pub async fn consolidate_file(
        &mut self,
        input_path: impl AsRef<Path>,
    ) -> Result<MockCollectionConfig> {
        let path_buf = PathBuf::from(input_path.as_ref());
        let collection = MockCollectionConfig::from_file(path_buf)
            .await
            .context("Failed to load mock collection")?;

        self.consolidate(collection)
    }

    /// Consolidate a mock collection in memory
    #[allow(clippy::cast_precision_loss)] // Mock counts are small enough for f64 to be exact
    pub fn consolidate(
        &mut self,
        collection: MockCollectionConfig,
    ) -> Result<MockCollectionConfig> {
        self.stats.original_count = collection.mocks.len();

        println!(
            "Analyzing {} mocks for consolidation...",
            self.stats.original_count
        );

        let groups = PatternDetector::group_similar_mocks(&collection.mocks);
        println!("   ✓ Grouped into {} request patterns", groups.len());

        let mut consolidated_mocks = Vec::new();
        for (group_id, group) in groups.iter().enumerate() {
            let processed = self.process_mock_group(group_id, group)?;
            consolidated_mocks.extend(processed);
        }

        self.stats.consolidated_count = consolidated_mocks.len();
        self.stats.reduction_ratio =
            1.0 - (self.stats.consolidated_count as f64 / self.stats.original_count.max(1) as f64);

        let consolidated_name = collection
            .name
            .map(|n| format!("{n} (Consolidated)"))
            .or_else(|| Some("Consolidated Mocks".to_string()));

        Ok(MockCollectionConfig {
            name: consolidated_name,
            description: Some(format!(
                "Consolidated from {} mocks. Reduction: {:.1}%",
                self.stats.original_count,
                self.stats.reduction_ratio * 100.0
            )),
            enabled: collection.enabled,
            vars: None,
            mocks: consolidated_mocks,
        })
    }

    /// Process a group of similar mocks using generic data-driven algorithm
    #[allow(clippy::indexing_slicing)] // `group[0]` guarded by `group.len() == 1` early return above
    fn process_mock_group(
        &mut self,
        group_id: usize,
        group: &[MockConfig],
    ) -> Result<Vec<MockConfig>> {
        if group.len() == 1 {
            return Ok(group.to_vec());
        }

        println!("   Processing group {} ({} mocks)", group_id, group.len());

        if PatternDetector::are_duplicates(group) {
            self.stats.duplicates_removed += group.len() - 1;
            self.stats.patterns_detected += 1;
            println!("      ↳ Removed {} duplicate mocks", group.len() - 1);
            return Ok(vec![group[0].clone()]);
        }

        if !self.options.enable_consolidation {
            println!("      ↳ Consolidation disabled, keeping mocks separate");
            return Ok(group.to_vec());
        }

        if group.len() < self.options.min_pattern_threshold {
            println!(
                "      ↳ Group size ({}) below threshold ({}), keeping mocks separate",
                group.len(),
                self.options.min_pattern_threshold
            );
            return Ok(group.to_vec());
        }

        let url_pattern = PatternDetector::generate_smart_url_pattern(group);
        let response_analysis = self.response_analyzer.analyze_response_patterns(group)?;

        // Analyze GraphQL variables if this is a GraphQL group
        let graphql_analysis = ResponseAnalyzer::analyze_graphql_variables(group);

        // Log GraphQL variable analysis if detected
        if graphql_analysis.has_variables {
            if graphql_analysis.has_varying_variables {
                println!(
                    "      ↳ Detected {} varying GraphQL variables: {:?}",
                    graphql_analysis.varying_variables.len(),
                    graphql_analysis.varying_variables
                );
            }
            if !graphql_analysis.constant_variables.is_empty() {
                println!(
                    "      ↳ Detected {} constant GraphQL variables",
                    graphql_analysis.constant_variables.len()
                );
            }
        }

        if response_analysis.varying_fields.is_empty() {
            self.stats.patterns_detected += 1;
            println!("      ↳ Identical responses -> single mock with pattern: {url_pattern}");
            let mut consolidated = group[0].clone();
            consolidated.id = format!("{}-consolidated", group[0].id).into();
            if let Some(ref mut match_config) = consolidated.match_config {
                match_config.urls = vec![url_pattern];
                match_config.url = None;
            }
            Ok(vec![consolidated])
        } else if self.options.enable_templates && response_analysis.is_json {
            self.stats.patterns_detected += 1;
            println!(
                "      ↳ Creating smart template with {} varying fields (pattern: {})",
                response_analysis.varying_fields.len(),
                url_pattern
            );
            self.stats.templates_created += 1;
            Ok(self.create_smart_template_mock(
                group,
                &url_pattern,
                &response_analysis,
                &graphql_analysis,
            ))
        } else {
            println!(
                "      ↳ Keeping mocks separate (non-JSON or templates disabled) (pattern: {url_pattern})"
            );
            Ok(group.to_vec())
        }
    }

    /// Create a smart template-based mock using Tera templates
    #[allow(clippy::indexing_slicing)] // `group[0]` guarded by callers ensuring non-empty group
    fn create_smart_template_mock(
        &self,
        group: &[MockConfig],
        pattern: &str,
        analysis: &ResponseAnalysis,
        graphql_analysis: &analysis::GraphQLVariableAnalysis,
    ) -> Vec<MockConfig> {
        let base_path = PatternDetector::extract_base_path(&group[0]);

        // Convert consolidator types to codegen types
        let response_structure: crate::codegen::ResponseStructure = analysis.into();
        let graphql_info: crate::codegen::GraphQLVariableInfo = graphql_analysis.into();

        let template_body = self.template_generator.generate_tera_template(
            &response_structure,
            &base_path,
            &graphql_info,
        );

        if let Err(e) = crate::template::validate_template(&template_body) {
            eprintln!("  Warning: Generated template has validation errors:");
            eprintln!("{e}");
            eprintln!("Template content:\n{template_body}");
            println!(
                "      ↳ Falling back to keeping mocks separate due to template validation error"
            );
            return group.to_vec();
        }

        let mut template_mock = group[0].clone();
        template_mock.id = format!("{}-smart-template", group[0].id).into();
        if let Some(ref mut match_config) = template_mock.match_config {
            match_config.urls = vec![pattern.to_string()];
            match_config.url = None;
        }

        // Extract common headers and status from the group
        let common_status = Self::extract_common_status(group);
        let common_headers = Self::extract_common_headers(group);

        template_mock.response_config = Some(ReturnConfig::Structured {
            status: common_status,
            headers: common_headers,
            body: None,
            template: Some(template_body),
            file: None,
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        });

        println!(
            "      ↳ Generated smart template with {} dynamic fields",
            analysis.varying_fields.len()
        );

        vec![template_mock]
    }

    /// Get consolidation statistics
    pub fn stats(&self) -> &ConsolidationStats {
        &self.stats
    }

    /// Extract common status code from a group of mocks (if all are the same)
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.is_empty()` returns early
    fn extract_common_status(group: &[MockConfig]) -> Option<u16> {
        if group.is_empty() {
            return None;
        }

        let first_status = group[0]
            .response_config
            .as_ref()
            .and_then(crate::config::ResponseConfig::status);

        // Check if all mocks have the same status
        let all_same = group.iter().all(|mock| {
            mock.response_config
                .as_ref()
                .and_then(crate::config::ResponseConfig::status)
                == first_status
        });

        if all_same { first_status } else { None }
    }

    /// Extract common headers from a group of mocks
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.is_empty()` returns early
    fn extract_common_headers(group: &[MockConfig]) -> FxHashMap<String, String> {
        if group.is_empty() {
            return FxHashMap::default();
        }

        // Get headers from first mock
        let first_headers = group[0]
            .response_config
            .as_ref()
            .and_then(|r| r.headers())
            .cloned()
            .unwrap_or_default();

        // Find headers that are common across all mocks (same key and value)
        let mut common_headers = FxHashMap::default();

        for (key, value) in &first_headers {
            let is_common = group.iter().all(|mock| {
                mock.response_config
                    .as_ref()
                    .and_then(|r| r.headers())
                    .and_then(|h| h.get(key))
                    .is_some_and(|v| v == value)
            });

            if is_common {
                common_headers.insert(key.clone(), value.clone());
            }
        }

        common_headers
    }
}

impl Default for MockConsolidator {
    fn default() -> Self {
        Self::new()
    }
}
