//! File watcher using the notify crate

use crate::Result;
use crate::error::Context;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
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

/// A single ignore rule, either a suffix match, a substring match, or a prefix match
#[derive(Debug, Clone)]
enum IgnoreRule {
    EndsWith(&'static str),
    Contains(&'static str),
    StartsWith(&'static str),
}

impl IgnoreRule {
    fn matches(&self, path_str: &str) -> bool {
        match self {
            Self::EndsWith(s) => path_str.ends_with(s),
            Self::Contains(s) => path_str.contains(s),
            Self::StartsWith(s) => path_str.starts_with(s),
        }
    }
}

/// Filter configuration for file events
#[derive(Debug, Clone)]
pub struct FileEventFilter {
    /// File extensions to watch (e.g., "json", "yaml")
    pub extensions: FxHashSet<String>,
    /// Patterns to ignore
    ignore_rules: Vec<IgnoreRule>,
}

impl FileEventFilter {
    /// Create a new filter with common defaults
    #[must_use]
    pub fn new() -> Self {
        Self {
            extensions: FxHashSet::default(),
            ignore_rules: vec![
                IgnoreRule::EndsWith(".swp"),
                IgnoreRule::EndsWith(".tmp"),
                IgnoreRule::EndsWith("~"),
                IgnoreRule::EndsWith(".DS_Store"),
                IgnoreRule::Contains("/.git/"),
                IgnoreRule::Contains("/node_modules/"),
                IgnoreRule::EndsWith(".swx"),
                IgnoreRule::StartsWith(".#"),
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
        for rule in &self.ignore_rules {
            if rule.matches(&path_str) {
                debug!("Ignoring file (matches rule): {}", path_str);
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
    watcher: RecommendedWatcher,
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
            watcher,
            rx,
            filter,
        })
    }

    /// Watch a path (file or directory)
    pub fn watch(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        self.watcher
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
