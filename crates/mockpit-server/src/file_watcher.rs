//! File watcher using the notify crate

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use regex::Regex;
use rustc_hash::FxHashSet;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{debug, error};

/// File system event types we care about
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

impl FileEvent {
    pub fn path(&self) -> &Path {
        match self {
            Self::Created(p) | Self::Modified(p) | Self::Removed(p) => p,
        }
    }
}

/// Filter configuration for file events
#[derive(Debug, Clone)]
pub struct FileEventFilter {
    /// File extensions to watch (e.g., "json", "yaml")
    pub extensions: FxHashSet<String>,
    /// Patterns to ignore (regex)
    pub ignore_patterns: Vec<Regex>,
}

impl FileEventFilter {
    /// Create a new filter with common defaults
    #[must_use]
    pub fn new() -> Self {
        Self {
            extensions: FxHashSet::default(),
            ignore_patterns: vec![
                Regex::new(r"\.swp$").expect("valid regex"),
                Regex::new(r"\.tmp$").expect("valid regex"),
                Regex::new(r"~$").expect("valid regex"),
                Regex::new(r"\.DS_Store$").expect("valid regex"),
                Regex::new(r"/\.git/").expect("valid regex"),
                Regex::new(r"/node_modules/").expect("valid regex"),
                Regex::new(r"\.swx$").expect("valid regex"),
                Regex::new(r"^\.\#").expect("valid regex"),
            ],
        }
    }

    /// Add multiple extensions
    #[must_use]
    pub fn with_extensions(mut self, exts: &[&str]) -> Self {
        for ext in exts {
            self.extensions.insert((*ext).to_string());
        }
        self
    }

    /// Check if a path should be processed
    pub fn should_process(&self, path: &Path) -> bool {
        if !self.extensions.is_empty() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if !self.extensions.contains(ext) {
                    return false;
                }
            } else {
                return false;
            }
        }

        let path_str = path.to_string_lossy();
        for pattern in &self.ignore_patterns {
            if pattern.is_match(&path_str) {
                debug!("Ignoring file (matches pattern): {}", path_str);
                return false;
            }
        }

        true
    }
}

impl Default for FileEventFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// File watcher using notify crate
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::UnboundedReceiver<FileEvent>,
    #[allow(dead_code)]
    filter: FileEventFilter,
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new(filter: FileEventFilter) -> Result<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        let filter_clone = filter.clone();

        let watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| match res {
                Ok(event) => match event.kind {
                    EventKind::Create(_) => {
                        for path in event.paths {
                            if filter_clone.should_process(&path) {
                                debug!("File created: {}", path.display());
                                let _ = tx.send(FileEvent::Created(path));
                            }
                        }
                    }
                    EventKind::Modify(_) => {
                        for path in event.paths {
                            if filter_clone.should_process(&path) {
                                debug!("File modified: {}", path.display());
                                let _ = tx.send(FileEvent::Modified(path));
                            }
                        }
                    }
                    EventKind::Remove(_) => {
                        for path in event.paths {
                            if filter_clone.should_process(&path) {
                                debug!("File removed: {}", path.display());
                                let _ = tx.send(FileEvent::Removed(path));
                            }
                        }
                    }
                    _ => {}
                },
                Err(e) => {
                    error!("File watcher error: {}", e);
                }
            },
            notify::Config::default(),
        )
        .context("Failed to create file watcher")?;

        Ok(Self {
            _watcher: watcher,
            rx,
            filter,
        })
    }

    /// Watch a path (file or directory)
    pub fn watch(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        self._watcher
            .watch(path, RecursiveMode::Recursive)
            .with_context(|| format!("Failed to watch path: {}", path.display()))?;
        debug!("Watching path: {}", path.display());
        Ok(())
    }

    /// Get the next file event (async)
    pub async fn next_event(&mut self) -> Option<FileEvent> {
        self.rx.recv().await
    }
}
