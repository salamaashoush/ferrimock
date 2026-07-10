//! Hot reload manager for automatic mock reloading

use super::debouncer::{DebouncedEvent, EventDebouncer};
use super::file_watcher::{FileEvent, FileEventFilter, FileWatcher};
use crate::engine::MockRegistry;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

/// Configuration for hot reload behavior
#[derive(Debug, Clone, Copy)]
pub struct HotReloadConfig {
    /// Debounce duration in milliseconds
    pub debounce_ms: u64,
}

impl Default for HotReloadConfig {
    fn default() -> Self {
        Self { debounce_ms: 300 }
    }
}

/// Statistics for a hot reload operation
#[derive(Debug, Clone)]
pub struct ReloadStats {
    pub files_processed: usize,
    pub mocks_added: usize,
    pub mocks_removed: usize,
    pub errors: Vec<String>,
}

/// Hot reload manager for automatic mock reloading
pub struct HotReloadManager {
    mock_registry: Arc<MockRegistry>,
    watcher: FileWatcher,
    debouncer: EventDebouncer,
    watch_dirs: Vec<PathBuf>,
}

impl HotReloadManager {
    /// Create a new hot reload manager
    pub fn new(
        mock_registry: Arc<MockRegistry>,
        watch_dirs: Vec<PathBuf>,
        config: HotReloadConfig,
    ) -> crate::Result<Self> {
        #[cfg(feature = "scripting")]
        let filter = FileEventFilter::new()
            .with_extensions(&["json", "yaml", "yml", "har", "js", "mjs", "ts", "mts"]);
        #[cfg(not(feature = "scripting"))]
        let filter = FileEventFilter::new().with_extensions(&["json", "yaml", "yml", "har"]);
        let watcher = FileWatcher::new(filter)?;
        let debouncer = EventDebouncer::new(Duration::from_millis(config.debounce_ms));

        Ok(Self {
            mock_registry,
            watcher,
            debouncer,
            watch_dirs,
        })
    }

    /// Start watching directories
    pub fn start_watching(&mut self) -> crate::Result<()> {
        for dir in &self.watch_dirs {
            if dir.exists() {
                self.watcher.watch(dir)?;
                debug!("Hot reload: watching {}", dir.display());
            } else {
                warn!("Hot reload: directory does not exist: {}", dir.display());
            }
        }
        Ok(())
    }

    /// Run the hot reload loop (spawns background task)
    pub fn spawn(mut self) -> JoinHandle<()> {
        tokio::spawn(async move {
            debug!("Hot reload manager started");
            loop {
                if let Some(event) = self.watcher.next_event().await {
                    debug!("Hot reload: file event {:?}", event);
                    self.debouncer.add_event(event).await;
                    if let Some(batch) = self.debouncer.flush().await {
                        Box::pin(self.handle_batch(batch)).await;
                    }
                }
            }
        })
    }

    /// Handle a batch of debounced file events
    async fn handle_batch(&self, batch: DebouncedEvent) {
        let event_count = batch.events.len();
        debug!("Hot reload: processing {event_count} file change(s)");

        let mut stats = ReloadStats {
            files_processed: 0,
            mocks_added: 0,
            mocks_removed: 0,
            errors: Vec::new(),
        };

        for event in batch.events {
            match event {
                FileEvent::Created(path) | FileEvent::Modified(path) => {
                    debug!("Hot reload: reloading file {}", path.display());
                    match Box::pin(self.reload_file(&path)).await {
                        Ok(count) => {
                            stats.files_processed += 1;
                            stats.mocks_added += count;
                            debug!("Hot reload: loaded {count} mock(s) from {}", path.display());
                        }
                        Err(e) => {
                            error!("Hot reload: failed to reload {}: {e}", path.display());
                            stats.errors.push(format!("{}: {e}", path.display()));
                        }
                    }
                }
                FileEvent::Removed(path) => {
                    debug!("Hot reload: removing mocks from {}", path.display());
                    let path_str = path.to_string_lossy().to_string();
                    let count = self.mock_registry.remove_file_mocks(&path_str);
                    stats.files_processed += 1;
                    stats.mocks_removed += count;
                    debug!(
                        "Hot reload: removed {count} mock(s) from {}",
                        path.display()
                    );
                }
            }
        }

        if stats.files_processed > 0 {
            debug!(
                "Hot reload: processed {} file(s), added {} mock(s), removed {} mock(s), {} error(s)",
                stats.files_processed,
                stats.mocks_added,
                stats.mocks_removed,
                stats.errors.len()
            );
        }
    }

    /// Reload a single file
    async fn reload_file(&self, path: &Path) -> crate::Result<usize> {
        Box::pin(self.mock_registry.reload_file(path)).await
    }
}
