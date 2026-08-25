//! What every request handler shares.

use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;

use super::client::UpstreamClient;
use super::config::ProxyConfig;
use crate::engine::MockMatcher;
use crate::recorder::{MockRecorder, RecordingFormat};

/// Shared, immutable-per-request proxy state.
pub struct ProxyState {
    /// Listener, routes, limits.
    pub config: ProxyConfig,
    /// The mock engine, absent when the proxy runs as a plain gateway.
    pub matcher: Option<MockMatcher>,
    /// The pooled upstream client.
    pub client: UpstreamClient,
    /// The recorder, when one is running.
    ///
    /// A `parking_lot` lock rather than an async one: this is read on every
    /// forwarded request and the guard never crosses an await, so an async
    /// lock would buy nothing and cost a scheduler interaction per request.
    recorder: RwLock<Option<Arc<MockRecorder>>>,
}

impl ProxyState {
    /// Assemble the state a running proxy needs.
    ///
    /// # Errors
    /// Fails when the upstream client cannot be built.
    pub fn new(config: ProxyConfig, matcher: Option<MockMatcher>) -> crate::Result<Self> {
        let client = UpstreamClient::new(&config.upstream)?;
        Ok(Self {
            config,
            matcher,
            client,
            recorder: RwLock::new(None),
        })
    }

    /// Whether a request should be offered to the matcher at all.
    ///
    /// Both halves matter: `mocks_enabled` is the operator's switch and
    /// `is_enabled` is the registry's own, which the management API toggles.
    pub fn mocks_enabled(&self) -> bool {
        self.config.mocks_enabled
            && self
                .matcher
                .as_ref()
                .is_some_and(|matcher| matcher.registry().is_enabled())
    }

    /// The running recorder, if any.
    pub fn recorder(&self) -> Option<Arc<MockRecorder>> {
        self.recorder.read().clone()
    }

    /// Begin recording forwarded traffic into `storage_dir`.
    ///
    /// Only traffic that reaches an upstream is recorded. A mocked response is
    /// not an observation of anything, and recording one would feed the
    /// consolidator its own output.
    ///
    /// # Errors
    /// Fails when a recording is already running, or the session file cannot
    /// be created.
    pub async fn start_recording(
        &self,
        storage_dir: impl Into<PathBuf>,
        session_name: Option<String>,
        format: RecordingFormat,
    ) -> crate::Result<String> {
        if self.recorder.read().is_some() {
            return Err(crate::mp_err!("recording is already in progress"));
        }

        let session = session_name.unwrap_or_else(|| {
            chrono::Utc::now()
                .format("recording-%Y%m%d-%H%M%S")
                .to_string()
        });

        // The file is created before the lock is taken, so a failure leaves
        // the proxy in the state it was already in.
        let recorder = MockRecorder::with_format(&session, storage_dir.into(), format);
        recorder.init_file().await?;
        *self.recorder.write() = Some(Arc::new(recorder));

        Ok(session)
    }

    /// Stop recording and finalize the session file.
    ///
    /// # Errors
    /// Fails when no recording is running, or the session file cannot be
    /// finalized.
    pub async fn stop_recording(&self) -> crate::Result<Option<PathBuf>> {
        let recorder = self.recorder.write().take();
        let Some(recorder) = recorder else {
            return Err(crate::mp_err!("no recording is in progress"));
        };

        recorder.finalize_file().await?;
        Ok(recorder.get_file_path().await)
    }
}

impl std::fmt::Debug for ProxyState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyState")
            .field("routes", &self.config.routes.len())
            .field("mocks_enabled", &self.mocks_enabled())
            .field("recording", &self.recorder.read().is_some())
            .finish_non_exhaustive()
    }
}
