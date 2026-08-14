//! Mock system state management

use crate::consolidator::{ConsolidationStats, ConsolidatorOptions};
use crate::engine::recorder_ext::MockRecorderConsolidationExt;
use crate::engine::{MockMatcher, MockRegistry};
use crate::recorder::{MockRecorder, RecordingFormat};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Mock system state (registry + matcher + recorder)
#[allow(clippy::struct_field_names)]
#[derive(Clone)]
pub struct MockState {
    pub mock_registry: Arc<MockRegistry>,
    pub mock_matcher: Arc<MockMatcher>,
    /// Mock recorder - can be started/stopped at runtime via RwLock
    pub mock_recorder: Arc<RwLock<Option<Arc<MockRecorder>>>>,
}

impl MockState {
    /// Begin recording into `storage_dir`, returning the session name.
    ///
    /// The session file is created up front, so a recording that captures
    /// nothing still leaves a well-formed collection behind rather than no
    /// file at all.
    ///
    /// # Errors
    /// Fails when a recording is already running, or when the session file
    /// cannot be created.
    pub async fn start_recording(
        &self,
        storage_dir: impl Into<PathBuf>,
        session_name: Option<String>,
        format: RecordingFormat,
    ) -> crate::Result<String> {
        let mut recorder_guard = self.mock_recorder.write().await;
        if recorder_guard.is_some() {
            return Err(crate::mp_err!("Recording is already in progress"));
        }

        let session = session_name.unwrap_or_else(|| {
            chrono::Utc::now()
                .format("recording-%Y%m%d-%H%M%S")
                .to_string()
        });

        let recorder = MockRecorder::with_format(&session, storage_dir.into(), format);
        recorder.init_file().await?;
        *recorder_guard = Some(Arc::new(recorder));

        Ok(session)
    }

    /// Stop recording, consolidating the session when options are supplied.
    ///
    /// # Errors
    /// Fails when no recording is running, or when the session file cannot be
    /// finalized. A consolidation failure is not an error: the recording is
    /// already on disk by then, so the path is returned without stats rather
    /// than losing the session over a post-processing step.
    pub async fn stop_recording(
        &self,
        consolidate: Option<ConsolidateOptions>,
    ) -> crate::Result<StopRecordingResult> {
        let mut recorder_guard = self.mock_recorder.write().await;
        let Some(recorder) = recorder_guard.take() else {
            return Err(crate::mp_err!("No recording is in progress"));
        };
        drop(recorder_guard);

        let Some(options) = consolidate else {
            recorder.finalize_file().await?;
            return Ok(StopRecordingResult {
                file_path: recorder.get_file_path().await,
                consolidation_stats: None,
                fidelity: None,
            });
        };

        let consolidator_options = ConsolidatorOptions {
            enable_consolidation: true,
            enable_templates: options.enable_templates,
            min_pattern_threshold: options.min_pattern,
            ..ConsolidatorOptions::default()
        };

        match recorder
            .finalize_and_consolidate(consolidator_options, options.keep_original)
            .await
        {
            Ok(result) => Ok(StopRecordingResult {
                file_path: Some(result.path),
                consolidation_stats: Some(result.stats),
                fidelity: result.fidelity,
            }),
            Err(e) => {
                tracing::warn!("Consolidation failed (recording still saved): {e}");
                Ok(StopRecordingResult {
                    file_path: recorder.get_file_path().await,
                    consolidation_stats: None,
                    fidelity: None,
                })
            }
        }
    }
}

/// Options for consolidation when stopping recording
pub struct ConsolidateOptions {
    pub enable_templates: bool,
    pub keep_original: bool,
    pub min_pattern: usize,
}

/// Result of stopping a recording
pub struct StopRecordingResult {
    pub file_path: Option<PathBuf>,
    pub consolidation_stats: Option<ConsolidationStats>,
    /// How much of the recorded traffic the consolidated file still reproduces.
    /// `None` when nothing was consolidated.
    pub fidelity: Option<crate::consolidator::FidelityReport>,
}
