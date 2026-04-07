use mockpit_server::MockState;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for the mock API server
#[derive(Clone)]
pub struct MockApiConfig {
    /// Directory containing mock collection files
    pub collections_dir: Option<PathBuf>,
    /// Directory containing recorded interactions
    pub recordings_dir: Option<PathBuf>,
}

/// Composed state for the mock management API.
#[derive(Clone)]
pub struct MockApiState {
    pub mock: MockState,
    pub config: Arc<MockApiConfig>,
}
