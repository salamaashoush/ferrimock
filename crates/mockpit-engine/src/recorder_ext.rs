//! Recorder extension for consolidation functionality
//!
//! This module extends `mockpit_recorder::MockRecorder` with consolidation capabilities,
//! allowing recordings to be automatically consolidated after finalization.

use anyhow::Result;
use mockpit_consolidator::{ConsolidationStats, ConsolidatorOptions, MockConsolidator};
use mockpit_recorder::{MockRecorder, RecordingFormat};
use std::path::PathBuf;

/// Extension trait for MockRecorder that adds consolidation functionality
pub trait MockRecorderConsolidationExt {
    /// Finalize and consolidate the recording file
    ///
    /// This will:
    /// 1. Finalize the recording file (close JSON/HAR structures)
    /// 2. Load the file as a mock collection
    /// 3. Consolidate the mocks using the consolidator
    /// 4. Write the consolidated mocks back to the file
    ///
    /// Returns the file path and consolidation statistics
    fn finalize_and_consolidate(
        &self,
        consolidator_options: ConsolidatorOptions,
        keep_original: bool,
    ) -> impl std::future::Future<Output = Result<(PathBuf, ConsolidationStats)>> + Send;
}

impl MockRecorderConsolidationExt for MockRecorder {
    async fn finalize_and_consolidate(
        &self,
        consolidator_options: ConsolidatorOptions,
        keep_original: bool,
    ) -> Result<(PathBuf, ConsolidationStats)> {
        // First, finalize the file normally
        self.finalize_file().await?;

        // Get the file path - we need to access internals here
        // This is a limitation of the extension approach, but we'll work around it
        // by saving and getting the path from the save operation
        let file_path = self
            .get_file_path()
            .await
            .ok_or_else(|| anyhow::anyhow!("No recording file initialized"))?;

        // Check format - HAR cannot be consolidated
        let format = self.get_format();
        if matches!(format, RecordingFormat::Har) {
            // HAR format cannot be consolidated, return empty stats
            let empty_stats = ConsolidationStats {
                original_count: 0,
                consolidated_count: 0,
                reduction_ratio: 0.0,
                patterns_detected: 0,
                duplicates_removed: 0,
                templates_created: 0,
            };
            return Ok((file_path, empty_stats));
        }

        // Create consolidator with provided options
        let mut consolidator = MockConsolidator::with_options(consolidator_options);

        // Consolidate the file
        let consolidated = consolidator
            .consolidate_file(&file_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to consolidate recording: {}", e))?;

        // Backup original file if requested
        if keep_original {
            let original_path = file_path.with_extension("original.json");
            tokio::fs::copy(&file_path, &original_path).await?;
            tracing::debug!("Saved original recording to: {}", original_path.display());
        }

        // Write the consolidated mocks back to the file
        let content = match format {
            RecordingFormat::Json => serde_json::to_string_pretty(&consolidated)?,
            RecordingFormat::Yaml => serde_yaml::to_string(&consolidated)
                .map_err(|e| anyhow::anyhow!("YAML serialization error: {}", e))?,
            RecordingFormat::Har => unreachable!("HAR format already handled above"),
        };

        tokio::fs::write(&file_path, content).await?;

        // Return file path and stats
        Ok((file_path, consolidator.stats().clone()))
    }
}
