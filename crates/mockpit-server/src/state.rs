//! Mock system state management

use mockpit_consolidator::ConsolidationStats;
use mockpit_engine::{MockMatcher, MockRegistry};
use mockpit_recorder::MockRecorder;
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
}
