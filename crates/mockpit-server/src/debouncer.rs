//! Event debouncer for file system events

use crate::file_watcher::FileEvent;
use rustc_hash::FxHashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::debug;

/// Batched file events after debouncing
#[derive(Debug, Clone)]
pub struct DebouncedEvent {
    pub events: Vec<FileEvent>,
}

/// Debouncer for file system events
///
/// Collects events within a time window and deduplicates them.
/// This prevents reload storms from editors that save files multiple times.
pub struct EventDebouncer {
    pending_events: Arc<Mutex<FxHashMap<PathBuf, FileEvent>>>,
    debounce_duration: Duration,
}

impl EventDebouncer {
    /// Create a new debouncer with the given duration
    pub fn new(debounce_duration: Duration) -> Self {
        Self {
            pending_events: Arc::new(Mutex::new(FxHashMap::default())),
            debounce_duration,
        }
    }

    /// Add an event to the debouncer
    pub async fn add_event(&self, event: FileEvent) {
        let mut pending = self.pending_events.lock().await;
        let path = event.path().to_path_buf();
        debug!(
            "Debouncer: added event for {} ({:?})",
            path.display(),
            event
        );
        // Last event wins for deduplication
        pending.insert(path, event);
    }

    /// Process events with debouncing
    ///
    /// This runs forever, polling on the debounce interval and dispatching batches.
    pub async fn process_events<F>(&self, mut on_batch: F) -> !
    where
        F: FnMut(DebouncedEvent),
    {
        loop {
            sleep(self.debounce_duration).await;
            let events = {
                let mut pending = self.pending_events.lock().await;
                if pending.is_empty() {
                    continue;
                }
                pending.drain().map(|(_, e)| e).collect::<Vec<_>>()
            };
            if !events.is_empty() {
                debug!("Debouncer: flushing {} event(s)", events.len());
                on_batch(DebouncedEvent { events });
            }
        }
    }

    /// Flush all pending events immediately without waiting
    pub async fn flush(&self) -> Option<DebouncedEvent> {
        let events = {
            let mut pending = self.pending_events.lock().await;
            if pending.is_empty() {
                return None;
            }
            pending.drain().map(|(_, e)| e).collect::<Vec<_>>()
        };
        if events.is_empty() {
            None
        } else {
            debug!("Debouncer: flushed {} event(s)", events.len());
            Some(DebouncedEvent { events })
        }
    }
}
