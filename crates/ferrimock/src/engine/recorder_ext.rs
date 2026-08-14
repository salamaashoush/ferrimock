//! Recorder extension for consolidation functionality
//!
//! This module extends `crate::recorder::MockRecorder` with consolidation capabilities,
//! allowing recordings to be automatically consolidated after finalization.

use crate::Result;
use crate::consolidator::{
    ConsolidationStats, ConsolidatorOptions, FidelityOptions, FidelityReport, MockConsolidator,
};
use crate::recorder::{MockRecorder, RecordingFormat};
use std::path::PathBuf;

/// What finalizing and consolidating a recording produced.
pub struct ConsolidatedRecording {
    pub path: PathBuf,
    pub stats: ConsolidationStats,
    /// How much of the recorded traffic the consolidated file still reproduces.
    ///
    /// `None` for HAR recordings, which are not consolidated at all.
    pub fidelity: Option<FidelityReport>,
}

/// Extension trait for MockRecorder that adds consolidation functionality
pub trait MockRecorderConsolidationExt {
    /// Finalize and consolidate the recording file
    ///
    /// This will:
    /// 1. Finalize the recording file (close JSON/HAR structures)
    /// 2. Load the file as a mock collection
    /// 3. Consolidate the mocks using the consolidator
    /// 4. Replay the recorded traffic through the result to measure what it cost
    /// 5. Write the consolidated mocks back to the file
    fn finalize_and_consolidate(
        &self,
        consolidator_options: ConsolidatorOptions,
        keep_original: bool,
    ) -> impl std::future::Future<Output = Result<ConsolidatedRecording>> + Send;
}

impl MockRecorderConsolidationExt for MockRecorder {
    async fn finalize_and_consolidate(
        &self,
        consolidator_options: ConsolidatorOptions,
        keep_original: bool,
    ) -> Result<ConsolidatedRecording> {
        // First, finalize the file normally
        self.finalize_file().await?;

        let file_path = self
            .get_file_path()
            .await
            .ok_or_else(|| crate::mp_err!("No recording file initialized"))?;

        // Check format - HAR cannot be consolidated
        let format = self.get_format();
        if matches!(format, RecordingFormat::Har) {
            return Ok(ConsolidatedRecording {
                path: file_path,
                stats: ConsolidationStats {
                    original_count: 0,
                    consolidated_count: 0,
                    reduction_ratio: 0.0,
                    patterns_detected: 0,
                    duplicates_removed: 0,
                    templates_created: 0,
                },
                fidelity: None,
            });
        }

        let original = crate::config::MockCollectionConfig::from_file(file_path.clone())
            .await
            .map_err(|e| crate::mp_err!("Failed to load recording for consolidation: {e}"))?;

        let mut consolidator = MockConsolidator::with_options(consolidator_options);

        // The recorder still holds the traffic it wrote, so the consolidated
        // file can be checked against the real thing rather than trusted.
        let interactions = self.get_all();
        let fidelity_options = FidelityOptions {
            base_dir: file_path.parent().map(std::path::Path::to_path_buf),
            // Recording has stopped by the time this runs, but the persistence
            // store is process-global and a server may still be serving other
            // mocks from it, so leave it alone.
            reset_persistence: false,
            ..FidelityOptions::default()
        };
        let (consolidated, report) = consolidator
            .consolidate_verified(&interactions, original, &fidelity_options)
            .await
            .map_err(|e| crate::mp_err!("Failed to consolidate recording: {e}"))?;

        // Backup original file if requested
        if keep_original {
            let original_path = file_path.with_extension("original.json");
            tokio::fs::copy(&file_path, &original_path).await?;
            tracing::debug!("Saved original recording to: {}", original_path.display());
        }

        // Write the consolidated mocks back to the file
        let content = match format {
            RecordingFormat::Json => serde_json::to_string_pretty(&consolidated)?,
            RecordingFormat::Yaml => serde_yaml_ng::to_string(&consolidated)
                .map_err(|e| crate::mp_err!("YAML serialization error: {e}"))?,
            RecordingFormat::Har => crate::mp_bail!("HAR format already handled above"),
        };

        tokio::fs::write(&file_path, content).await?;

        Ok(ConsolidatedRecording {
            path: file_path,
            stats: consolidator.stats().clone(),
            fidelity: Some(report),
        })
    }
}
