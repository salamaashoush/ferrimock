//! Mock registry for storing and managing mock definitions
//!
//! `Arc<Vec<Arc<MockDefinition>>>` is intentional: the sorted-mocks cache is
//! shared by refcount clone, and the elements are already `Arc`. Allow rc_buffer.
#![allow(clippy::rc_buffer)]

use super::scope::{ScopeInfo, ScopeManager};
use crate::core::PersistenceStore;
use crate::engine::types::LeanString;
use crate::engine::types::MockDefinition;
use crate::recorder::RecordedInteraction;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use nohash_hasher::BuildNoHashHasher;
use parking_lot::{Mutex, RwLock};
use rustc_hash::{FxHashMap, FxHasher};
use serde::Serialize;
#[allow(clippy::disallowed_types)]
use std::collections::HashMap;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Type alias for the exact match index: hash(method, path) -> highest-priority mock.
/// Uses pre-computed u64 hash keys with nohash-hasher (zero-cost hashing for pre-hashed keys).
/// std HashMap is required here because FxHashMap doesn't support custom hashers.
#[allow(clippy::disallowed_types)]
type ExactMatchIndex = HashMap<u64, Arc<MockDefinition>, BuildNoHashHasher<u64>>;

/// Suffixes marking an OpenAPI document written in an ordinary data format.
///
/// Duplicated from `spec::source` rather than imported, because the loader has
/// to route the file the same way whether or not the crate was built with the
/// `spec` feature — without it, the file has to reach the "spec files ignored"
/// warning instead of being parsed as a mock collection and failing.
pub(crate) const OPENAPI_SUFFIXES: [&str; 3] = [".openapi.yaml", ".openapi.yml", ".openapi.json"];

fn is_openapi_document(path: &std::path::Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|name| OPENAPI_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)))
}

/// Compute a hash key for (method, path) pairs without allocating.
#[inline]
fn exact_match_key(method: &str, path: &str) -> u64 {
    let mut hasher = FxHasher::default();
    method.hash(&mut hasher);
    0u8.hash(&mut hasher);
    path.hash(&mut hasher);
    hasher.finish()
}

/// Options for [`MockRegistry::load_from_directory_with`].
#[derive(Debug, Clone, Copy)]
pub struct DirLoadOptions {
    /// Load `.js`/`.mjs` script mocks (requires the `scripting` feature).
    /// Disable when a JS runtime alongside this process owns those files.
    pub load_scripts: bool,
}

impl Default for DirLoadOptions {
    fn default() -> Self {
        Self { load_scripts: true }
    }
}

/// Represents a single call to a mock
#[derive(Clone, Debug, Serialize)]
pub struct MockCall {
    pub timestamp: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: FxHashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_hash: Option<String>, // SHA256 of body
}

impl MockCall {
    /// Create a new mock call record
    pub fn new(
        method: String,
        path: String,
        query: Option<String>,
        headers: FxHashMap<String, String>,
        body: Option<&[u8]>,
    ) -> Self {
        let body_hash = body.map(|b| {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(b);
            hasher
                .finalize()
                .iter()
                .fold(String::with_capacity(64), |mut acc, b| {
                    use std::fmt::Write;
                    let _ = write!(acc, "{b:02x}");
                    acc
                })
        });

        Self {
            timestamp: Utc::now(),
            method,
            path,
            query,
            headers,
            body_hash,
        }
    }
}

/// Cached sorted list of enabled mocks
/// Uses a version counter to track invalidation
struct SortedMocksCache {
    /// Cached sorted mocks (highest priority first), shared via Arc so readers
    /// clone only a refcount, never the Vec.
    mocks: RwLock<Arc<Vec<Arc<MockDefinition>>>>,
    /// Version counter for cache invalidation
    version: AtomicU64,
    /// Version of the mocks DashMap when cache was last built
    cached_version: AtomicU64,
}

impl SortedMocksCache {
    fn new() -> Self {
        Self {
            mocks: RwLock::new(Arc::new(Vec::new())),
            version: AtomicU64::new(0),
            // u64::MAX never equals the initial version (0) → forces first build,
            // so a legitimately empty result is still cached correctly afterwards.
            cached_version: AtomicU64::new(u64::MAX),
        }
    }

    /// Increment version to invalidate cache
    #[inline]
    fn invalidate(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Get cached mocks if still valid, otherwise None. Clones only the outer Arc.
    fn get_if_valid(&self) -> Option<Arc<Vec<Arc<MockDefinition>>>> {
        let current_version = self.version.load(Ordering::Acquire);
        let cached_version = self.cached_version.load(Ordering::Acquire);

        if current_version == cached_version {
            return Some(Arc::clone(&self.mocks.read()));
        }
        None
    }

    /// Update the cache with new sorted mocks
    fn update(&self, mocks: Arc<Vec<Arc<MockDefinition>>>) {
        let current_version = self.version.load(Ordering::Acquire);
        {
            let mut guard = self.mocks.write();
            *guard = mocks;
        }
        self.cached_version
            .store(current_version, Ordering::Release);
    }
}

/// Registry for managing mock definitions and recordings
/// - mocks: `Vec<MockDefinition>` (we use DashMap for concurrency)
/// - recordings: DashMap<String, RecordedInteraction>
/// - scopes: ScopeManager for test isolation
/// - enabled: AtomicBool
/// - call_tracking: DashMap for tracking mock calls (per-mock opt-in)
/// - persistence_store: PersistenceStore for cross-request state
#[derive(Clone)]
pub struct MockRegistry {
    /// Mock definitions stored by ID (Arc'd for efficient cloning)
    mocks: Arc<DashMap<LeanString, Arc<MockDefinition>>>,
    /// Registration sequence per mock id: the tiebreak among
    /// equal-priority mocks (first registered matches first).
    insertion_seq: Arc<DashMap<LeanString, u64>>,
    insertion_counter: Arc<AtomicU64>,
    /// Recorded interactions (request/response pairs)
    recordings: Arc<DashMap<String, RecordedInteraction>>,
    /// Scope manager for test isolation
    scope_manager: Arc<ScopeManager>,
    /// Global enabled/disabled flag
    enabled: Arc<AtomicBool>,
    /// Call tracking per mock ID (enabled per-mock to prevent memory leaks)
    call_tracking: Arc<DashMap<LeanString, VecDeque<MockCall>>>,
    /// Monotonic match count per mock ID. Always on: one relaxed increment per
    /// match, and one `u64` per mock that has ever matched. Unlike
    /// [`MockRegistry::get_call_count`] this is a true total, so assertions
    /// stay correct past the call-tracking window.
    match_counts: Arc<DashMap<LeanString, AtomicU64>>,
    /// Requests that matched no mock, folded by request line. Only populated
    /// when the matcher is tracking misses.
    unmatched: Arc<DashMap<String, UnmatchedRequest>>,
    /// Every miss, including ones past the distinct-line cap.
    unmatched_total: Arc<AtomicU64>,
    /// Misses whose line was counted but not retained.
    unmatched_dropped: Arc<AtomicU64>,
    /// Distinct unmatched request lines retained before dropping new ones.
    max_unmatched: usize,
    /// Maximum calls to track per mock (prevent memory leak)
    max_tracked_calls: usize,
    /// Persistence store for stateful mock scenarios
    persistence_store: Arc<PersistenceStore>,
    /// Cached sorted list of enabled mocks (invalidated on mock changes)
    sorted_mocks_cache: Arc<SortedMocksCache>,
    /// Fast-path index: exact (method, path) -> highest-priority mock ID
    /// Only populated for mocks with exact URL patterns and no conditional matchers.
    /// Keyed by (method_str, exact_path) for O(1) lookup.
    exact_match_index: Arc<RwLock<ExactMatchIndex>>,
    /// Bumped by every mock mutation. The index is current only while the
    /// epoch it was built from is still the live one — a counter the rebuild
    /// reads *before* the mocks, so a mutation landing mid-rebuild is not
    /// mistaken for one the rebuild already saw.
    exact_index_epoch: Arc<AtomicU64>,
    /// The epoch the current index was built from.
    exact_index_built_epoch: Arc<AtomicU64>,
    /// Single-flight guard: ensures only one thread rebuilds the exact index at
    /// a time, preventing a thundering-herd rebuild when the index goes stale
    /// under concurrent load (e.g. after a `once` mock is consumed).
    index_rebuild_lock: Arc<Mutex<()>>,
    /// Whether any enabled mock has conditional matchers (header/body/query/graphql).
    /// When false, the LRU cache can be used more aggressively.
    has_conditional_mocks: Arc<AtomicBool>,
    /// Whether any enabled mock matches on the request body (body or graphql matcher).
    /// Lets callers (e.g. the fetch interceptor) skip reading the request body when
    /// no mock could ever use it.
    has_body_dependent_mocks: Arc<AtomicBool>,
    /// Whether any enabled mock needs request headers (header matchers, handler
    /// mocks, or header-referencing templates). Lets the interceptor skip
    /// marshalling headers when no mock could ever use them.
    has_header_dependent_mocks: Arc<AtomicBool>,
    /// Whether any enabled mock carries an `UrlPattern::HrefRegex`
    /// (`ws.link(RegExp)`). When false, the matcher skips the per-candidate
    /// Host-header href reconstruction fallback entirely.
    has_href_regex_mocks: Arc<AtomicBool>,
    /// Global variables from MockConfig.vars, cascaded into all loaded collections
    global_vars: Arc<RwLock<Option<serde_json::Map<String, serde_json::Value>>>>,
    /// Typed, relational state: the entities a schema populates and every
    /// lane shares. Sits beside `persistence_store` on purpose — one is
    /// untyped scratch state, the other is the world a mocked API pretends
    /// to have, and neither belongs to whatever loaded it.
    world: Arc<crate::core::World>,
    /// Live WS/SSE connections per mock id; removal paths close them so
    /// reloaded definitions never keep serving through stale handlers
    streaming_conns: Arc<crate::streaming::StreamingConnections>,
    /// Script engines behind `.js`/`.mjs` mock files (one per file)
    #[cfg(feature = "scripting")]
    script_host: Arc<crate::scripting::ScriptHost>,
}

/// How many distinct unmatched request lines are retained before new ones are
/// counted but not kept.
const DEFAULT_MAX_UNMATCHED: usize = 1000;

fn request_line(method: &str, path: &str, query: Option<&str>) -> String {
    match query {
        Some(q) if !q.is_empty() => format!("{method} {path}?{q}"),
        _ => format!("{method} {path}"),
    }
}

/// One mock's line in a [`CoverageReport`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MockCoverage {
    /// Mock id.
    pub mock_id: String,
    /// Requests it served since the last reset.
    pub count: u64,
    /// Match priority.
    pub priority: u32,
    /// Whether the mock is still enabled. A `once` mock that fired reads as
    /// served but disabled; one that never fired but is disabled was never
    /// eligible in the first place.
    pub enabled: bool,
    /// File the mock was loaded from, so an unused entry names what to fix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
}

/// Which of the loaded mocks actually ran.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    /// Mocks that served at least one request, busiest first.
    pub served: Vec<MockCoverage>,
    /// Mocks that served none, highest priority first.
    pub unused: Vec<MockCoverage>,
    /// Requests served across every listed mock.
    pub total_matches: u64,
    /// States a declared machine has and nothing reached, as
    /// `machine.state`.
    ///
    /// A mock that never ran is one question; a *branch* of a mock that never
    /// ran is another, and template branching cannot answer it because the
    /// branches are not data. Declared edges are, which is the whole reason
    /// for declaring them.
    pub unreached_states: Vec<String>,
    /// Moves a declared machine has and nothing took, as
    /// `machine.state --event->`. Waiting is spelled `<after>`.
    pub untaken_edges: Vec<String>,
}

impl CoverageReport {
    /// How many mocks are loaded.
    #[must_use]
    pub fn total_mocks(&self) -> usize {
        self.served.len() + self.unused.len()
    }

    /// Share of loaded mocks that served at least one request, 0-100. An empty
    /// registry covers nothing, so it reports 0 rather than a vacuous 100.
    #[must_use]
    pub fn percent_covered(&self) -> f64 {
        let total = self.total_mocks();
        if total == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            (self.served.len() as f64) * 100.0 / (total as f64)
        }
    }
}

/// A request line that matched no mock, with how often it was seen.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedRequest {
    /// HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// Query string, when the request carried one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    /// How many times this exact line missed.
    pub count: u64,
    /// When it first missed.
    pub first_seen: DateTime<Utc>,
    /// When it last missed.
    pub last_seen: DateTime<Utc>,
}

/// Every request the registry could not serve.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedReport {
    /// Distinct request lines, most frequent first.
    pub requests: Vec<UnmatchedRequest>,
    /// Total misses, including any not retained.
    pub total: u64,
    /// Misses whose request line was not retained because the distinct-line
    /// cap was reached. Non-zero means `requests` is an undercount.
    pub dropped: u64,
}

/// How many times a mock was expected to serve a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    /// Served exactly this many times.
    Exactly(u64),
    /// Served at least this many times.
    AtLeast(u64),
    /// Served at most this many times.
    AtMost(u64),
    /// Never served.
    Never,
}

impl Expected {
    /// Whether an actual count satisfies this expectation.
    #[must_use]
    pub const fn satisfied_by(self, actual: u64) -> bool {
        match self {
            Self::Exactly(n) => actual == n,
            Self::AtLeast(n) => actual >= n,
            Self::AtMost(n) => actual <= n,
            Self::Never => actual == 0,
        }
    }
}

impl std::fmt::Display for Expected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exactly(n) => write!(f, "exactly {n}"),
            Self::AtLeast(n) => write!(f, "at least {n}"),
            Self::AtMost(n) => write!(f, "at most {n}"),
            Self::Never => f.write_str("never"),
        }
    }
}

/// A failed [`MockRegistry::verify`].
#[derive(Debug, Clone)]
pub struct VerifyError {
    /// The mock being asserted on.
    pub mock_id: String,
    /// What the caller expected.
    pub expected: Expected,
    /// What actually happened.
    pub actual: u64,
    /// Whether a mock with this id is registered at all. A typo in the id
    /// otherwise reads as "expected 1, got 0".
    pub known_mock: bool,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mock \"{}\" expected to serve {} request(s), served {}",
            self.mock_id, self.expected, self.actual
        )?;
        if !self.known_mock {
            f.write_str(" (no mock is registered with that id)")?;
        }
        Ok(())
    }
}

impl std::error::Error for VerifyError {}

impl MockRegistry {
    /// Create a new empty mock registry
    pub fn new() -> Self {
        Self::with_world(crate::core::global_world())
    }

    /// A registry over a world of the caller's choosing.
    ///
    /// Templates and scripts reach the world through the process-global handle,
    /// so the first registry built this way publishes its world there — without
    /// that, `entity_*` in a template reads a different world than the routes
    /// serve, which looks like the world being empty.
    ///
    /// A *second* registry cannot publish over the first. That one keeps its
    /// own world for matching and the HTTP API, and templates go on reading the
    /// world that got there first; the isolation is real, its reach is not.
    pub fn with_world(world: Arc<crate::core::World>) -> Self {
        if crate::core::set_global_world(Arc::clone(&world)).is_err()
            && !Arc::ptr_eq(&crate::core::global_world(), &world)
        {
            tracing::debug!(
                "a world is already installed process-wide; template and script                  `entity_*` calls will read that one rather than this registry's"
            );
        }
        // Get or create the global persistence store and share it with templates
        let persistence_store = crate::template::get_global_persistence_store();

        Self {
            mocks: Arc::new(DashMap::new()),
            insertion_seq: Arc::new(DashMap::new()),
            insertion_counter: Arc::new(AtomicU64::new(0)),
            recordings: Arc::new(DashMap::new()),
            scope_manager: Arc::new(ScopeManager::new()),
            enabled: Arc::new(AtomicBool::new(true)),
            call_tracking: Arc::new(DashMap::new()),
            match_counts: Arc::new(DashMap::new()),
            unmatched: Arc::new(DashMap::new()),
            unmatched_total: Arc::new(AtomicU64::new(0)),
            unmatched_dropped: Arc::new(AtomicU64::new(0)),
            max_unmatched: DEFAULT_MAX_UNMATCHED,
            max_tracked_calls: 100, // Default: track up to 100 calls per mock
            persistence_store,
            sorted_mocks_cache: Arc::new(SortedMocksCache::new()),
            exact_match_index: {
                #[allow(clippy::disallowed_types)]
                let idx = HashMap::with_hasher(BuildNoHashHasher::default());
                Arc::new(RwLock::new(idx))
            },
            exact_index_epoch: Arc::new(AtomicU64::new(1)),
            exact_index_built_epoch: Arc::new(AtomicU64::new(0)),
            index_rebuild_lock: Arc::new(Mutex::new(())),
            has_conditional_mocks: Arc::new(AtomicBool::new(false)),
            has_body_dependent_mocks: Arc::new(AtomicBool::new(false)),
            has_header_dependent_mocks: Arc::new(AtomicBool::new(false)),
            has_href_regex_mocks: Arc::new(AtomicBool::new(false)),
            global_vars: Arc::new(RwLock::new(None)),
            world,
            streaming_conns: Arc::new(crate::streaming::StreamingConnections::default()),
            #[cfg(feature = "scripting")]
            script_host: Arc::new(crate::scripting::ScriptHost::new()),
        }
    }

    /// Live streaming-connection tracker shared with the serve layer.
    pub fn streaming_connections(&self) -> Arc<crate::streaming::StreamingConnections> {
        Arc::clone(&self.streaming_conns)
    }

    /// The host owning the engines behind `.js`/`.mjs` mock files.
    /// Use it to tune [`crate::scripting::ScriptEngineConfig`] before loading.
    #[cfg(feature = "scripting")]
    pub fn script_host(&self) -> &Arc<crate::scripting::ScriptHost> {
        &self.script_host
    }

    /// Set global variables that will be cascaded into all loaded mock collections.
    /// These are the lowest-priority vars, shadowed by collection-level and mock-level vars.
    pub fn set_global_vars(&self, vars: Option<serde_json::Map<String, serde_json::Value>>) {
        *self.global_vars.write() = vars;
    }

    /// Get the current global variables
    pub fn global_vars(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        self.global_vars.read().clone()
    }

    /// Create a registry with the given mocks
    pub fn with_mocks(mocks: Vec<MockDefinition>) -> Self {
        let registry = Self::new();
        for mock in mocks {
            registry.add_mock(mock);
        }
        registry
    }

    /// Load mock collections from a directory
    ///
    /// Scans the given directory for mock definition files and loads all mock definitions.
    /// Also scans for scenario files in the scenarios subdirectory.
    /// Returns the number of mocks loaded.
    pub async fn load_from_directory(
        &self,
        dir_path: impl AsRef<std::path::Path>,
    ) -> crate::Result<usize> {
        self.load_from_directory_with(dir_path, DirLoadOptions::default())
            .await
    }

    /// [`Self::load_from_directory`] with explicit options.
    ///
    /// `load_scripts: false` silently leaves `.js`/`.mjs` files to another
    /// runtime — the NAPI addon uses it because Node loads script mocks
    /// itself (V8), so the embedded engine must not double-load them.
    // Takes `AsRef<Path>` rather than `&str` so a caller holding a real path —
    // the NAPI addon does — never has to round-trip it through UTF-8.
    pub async fn load_from_directory_with(
        &self,
        dir_path: impl AsRef<std::path::Path>,
        options: DirLoadOptions,
    ) -> crate::Result<usize> {
        let path = dir_path.as_ref();

        if !path.exists() {
            return Ok(0); // Directory doesn't exist, no mocks to load
        }

        if !path.is_dir() {
            return Err(crate::mp_err!("{} is not a directory", path.display()));
        }

        // Read directory entries
        let mut entries = tokio::fs::read_dir(path)
            .await
            .map_err(|e| crate::mp_err!("Failed to read directory {}: {e}", path.display()))?;

        // Collect all mock collection files (JSON, YAML), HAR files, and scripts
        let mut collection_files = Vec::new();
        let mut har_files = Vec::new();
        let mut script_files = Vec::new();
        let mut spec_files = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| crate::mp_err!("Failed to read directory entry: {e}"))?
        {
            let entry_path = entry.path();
            if let Some(ext) = entry_path.extension().and_then(|s| s.to_str()) {
                if is_openapi_document(&entry_path) {
                    spec_files.push(entry_path);
                } else if matches!(ext, "json" | "yaml" | "yml") {
                    collection_files.push(entry_path);
                } else if ext == "har" {
                    har_files.push(entry_path);
                } else if matches!(ext, "js" | "mjs" | "ts" | "mts") {
                    script_files.push(entry_path);
                } else if matches!(ext, "graphql" | "gql") {
                    spec_files.push(entry_path);
                }
            }
        }

        if !options.load_scripts {
            script_files.clear();
        }

        #[cfg(not(feature = "spec"))]
        if !spec_files.is_empty() {
            tracing::warn!(
                ignored = spec_files.len(),
                directory = %path.display(),
                "spec files ignored; build with the `spec` feature to serve .graphql \
                 schemas and .openapi.yaml documents"
            );
            spec_files.clear();
        }

        #[cfg(not(feature = "scripting"))]
        if !script_files.is_empty() {
            tracing::warn!(
                ignored = script_files.len(),
                directory = %path.display(),
                "script mock files ignored; build with the `scripting` feature to load .js/.mjs mocks"
            );
            script_files.clear();
        }

        // Parse every collection before registering any of them: a `serve:`
        // mock resolves against the world, and the world is not complete until
        // every collection's `world:` block and every schema has been read.
        let parsed_collections = futures::future::join_all(
            collection_files
                .iter()
                .map(crate::config::MockCollectionConfig::from_file),
        )
        .await;

        let mut collections = Vec::with_capacity(parsed_collections.len());
        for (i, result) in parsed_collections.into_iter().enumerate() {
            let Some(file) = collection_files.get(i) else {
                continue;
            };
            match result {
                Ok(collection) if collection.enabled => {
                    collections.push((file.clone(), collection));
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(file = %file.display(), error = %e, "failed to parse mock collection");
                }
            }
        }

        // World phase: settings and schemas, from collections first so a
        // `world.counts` is in force before anything seeds, then the schemas
        // dropped bare in the directory.
        for (file, collection) in &collections {
            Self::apply_collection_machines(collection);
            if let Err(e) = self.apply_collection_world(collection, file).await {
                tracing::warn!(file = %file.display(), error = %e, "failed to build the world");
            }
        }

        #[cfg(feature = "spec")]
        for file in &spec_files {
            match self.load_schema_file(file).await {
                Ok(entities) => {
                    tracing::debug!(file = %file.display(), entities, "loaded a schema");
                }
                Err(e) => {
                    tracing::warn!(file = %file.display(), error = %e, "failed to load schema");
                }
            }
        }

        #[cfg_attr(not(feature = "spec"), allow(unused_variables))]
        let any_serve = collections
            .iter()
            .any(|(_, collection)| collection.mocks.iter().any(|mock| mock.serve.is_some()));

        // Register phase: `serve:` can now resolve.
        let collection_results = futures::future::join_all(collections.into_iter().map(
            |(file, collection)| async move {
                let path = file.clone();
                self.register_collection(collection, &path)
                    .await
                    .map_err(|e| (file, e))
            },
        ))
        .await;

        // Load all HAR files in parallel
        let har_tasks: Vec<_> = har_files
            .iter()
            .map(|path| self.load_har_file(path))
            .collect();

        let har_results = futures::future::join_all(har_tasks).await;

        // Sum up loaded counts and log errors for collection files
        let mut loaded_count = 0;
        for result in collection_results {
            match result {
                Ok(count) => {
                    loaded_count += count;
                }
                Err((file, e)) => {
                    tracing::warn!(file = %file.display(), error = %e, "failed to load mock collection");
                }
            }
        }

        // Sum up loaded counts and log errors for HAR files
        for (i, result) in har_results.into_iter().enumerate() {
            match result {
                Ok(count) => {
                    loaded_count += count;
                }
                Err(e) => {
                    if let Some(file) = har_files.get(i) {
                        tracing::warn!(file = %file.display(), error = %e, "failed to load HAR file");
                    }
                }
            }
        }

        // A schema that nobody serves is almost always a mistake — most often
        // a `serve:` that was never written — so say so rather than starting
        // a server that answers nothing.
        #[cfg(feature = "spec")]
        if !self.world.is_empty() && !any_serve {
            tracing::warn!(
                entities = self.world.entities().len(),
                "the world has entities but no mock serves them; add a mock with \
                 `serve: graphql` and a `match.url` saying where the API answers"
            );
        }

        // Load script files in parallel (each gets its own engine)
        #[cfg(feature = "scripting")]
        {
            let script_tasks: Vec<_> = script_files
                .iter()
                .map(|file| self.load_script_file(file, Some(path)))
                .collect();
            for (i, result) in futures::future::join_all(script_tasks)
                .await
                .into_iter()
                .enumerate()
            {
                match result {
                    Ok(count) => {
                        loaded_count += count;
                    }
                    Err(e) => {
                        if let Some(file) = script_files.get(i) {
                            tracing::warn!(file = %file.display(), error = %e, "failed to load script mocks");
                        }
                    }
                }
            }
        }

        Ok(loaded_count)
    }

    /// Load a single `.js`/`.mjs` mock script file. `root` bounds what
    /// the script may import (defaults to the file's parent directory).
    #[cfg(feature = "scripting")]
    pub async fn load_script_file(
        &self,
        path: &std::path::Path,
        root: Option<&std::path::Path>,
    ) -> crate::Result<usize> {
        let definitions = self.script_host.load_file(path, root).await?;
        let count = definitions.len();
        for mock in definitions {
            self.add_mock(mock);
        }
        Ok(count)
    }

    /// The entity world this registry serves.
    ///
    /// Shared with templates, scripts and the HTTP API: a write through any
    /// one of them is visible to the others, and to every spec-derived route.
    pub fn world(&self) -> &Arc<crate::core::World> {
        &self.world
    }

    /// Read a schema file into the world.
    ///
    /// Registers no mocks. A schema declares *entities*; where they are served
    /// is a mock's business, because a schema file has nowhere to write down
    /// that it lives behind `https://api.example.com` rather than on localhost.
    #[cfg(feature = "spec")]
    pub async fn load_schema_file(&self, path: &std::path::Path) -> crate::Result<usize> {
        let loaded = crate::spec::load_schema_file(path, &self.world, false).await?;
        Self::report_schema_load(path, &loaded);
        Ok(loaded.entities)
    }

    /// Say what reading a schema cost, rather than letting it pass silently.
    ///
    /// A document with three unreadable `$ref`s is still worth serving, and a
    /// schema an item path could not key is still worth knowing about — but
    /// neither is worth discovering from a wrong answer later.
    #[cfg(feature = "spec")]
    fn report_schema_load(path: &std::path::Path, loaded: &crate::spec::source::SchemaLoad) {
        for defect in &loaded.repaired {
            tracing::warn!(file = %path.display(), defect = %defect, "repaired a malformed schema");
        }
        for defect in &loaded.defects {
            tracing::warn!(file = %path.display(), defect = %defect, "could not read part of the document");
        }
        for skipped in &loaded.skipped {
            tracing::warn!(file = %path.display(), skipped = %skipped, "a path addresses something the world cannot hold");
        }
        for conflict in &loaded.conflicts {
            tracing::warn!(
                file = %path.display(),
                conflict = %conflict,
                "a write could not be carried onto the rebuilt world"
            );
        }
    }

    /// Load a single mock collection file.
    ///
    /// Runs the collection's `world:` block first, because a `serve:` mock in
    /// the same file resolves against the world that block populates. Loading
    /// one collection in isolation therefore rebuilds the world's store; the
    /// directory loader batches the phase across every file instead.
    pub async fn load_collection_file(&self, path: &std::path::Path) -> crate::Result<usize> {
        use crate::config::MockCollectionConfig;

        let collection = MockCollectionConfig::from_file(path)
            .await
            .map_err(|e| crate::mp_err!("Failed to parse {}: {}", path.display(), e))?;

        // Only load if collection is enabled
        if !collection.enabled {
            return Ok(0);
        }

        Self::apply_collection_machines(&collection);
        self.apply_collection_world(&collection, path).await?;
        self.register_collection(collection, path).await
    }

    /// Install the machines a collection declares, for templates to read.
    ///
    /// Beside the world rather than inside it: a machine is not an entity's
    /// idea, and a collection with no `world:` block at all can still declare
    /// one and drive routes with it.
    fn apply_collection_machines(collection: &crate::config::MockCollectionConfig) {
        let Some(declared) = &collection.machines else {
            return;
        };
        let built = declared.iter().filter_map(|(name, machine)| {
            crate::config::machine_of(machine)
                .ok()
                .map(|built| (LeanString::from(name.as_str()), built))
        });
        crate::template::set_global_machines(std::sync::Arc::new(
            crate::core::machine::Machines::new(built),
        ));
    }

    /// Populate the world from a collection's `world:` block.
    ///
    /// Settings first, then schemas: both rebuild the store, and applying the
    /// counts before seeding avoids building a census twice.
    #[cfg(feature = "spec")]
    async fn apply_collection_world(
        &self,
        collection: &crate::config::MockCollectionConfig,
        path: &std::path::Path,
    ) -> crate::Result<()> {
        let Some(config) = &collection.world else {
            return Ok(());
        };

        let settings = crate::core::WorldSettings {
            seed: config.seed,
            default_count: config.count,
            scale: config.scale,
            counts: config
                .counts
                .iter()
                .flatten()
                .map(|(entity, count)| (LeanString::from(entity.as_str()), *count))
                .collect(),
            cascade_delete: config.cascade_delete,
            viewer: config.viewer.as_deref().map(LeanString::from),
            // Relative to the collection that named it, like every other path
            // a collection carries.
            persistence: config.persistence.as_ref().map(|name| {
                path.parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(name)
            }),
            overrides: config.field_rules(collection.machines.as_ref())?,
        };
        self.world.configure(&settings, path)?;

        let config_dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
        for schema in &config.schemas {
            let resolved = config_dir.join(schema);
            let loaded =
                crate::spec::load_schema_file(&resolved, &self.world, config.lenient).await?;
            Self::report_schema_load(&resolved, &loaded);
        }

        // After the schemas, not before: a rule is judged against the world it
        // was meant to describe, and settings are applied first.
        for rejected in self.world.rejected_overrides() {
            tracing::warn!(
                file = %path.display(),
                rejected = %rejected,
                "a `world.fields` rule was not applied"
            );
        }

        Ok(())
    }

    #[cfg(not(feature = "spec"))]
    #[allow(clippy::unused_async)]
    async fn apply_collection_world(
        &self,
        collection: &crate::config::MockCollectionConfig,
        path: &std::path::Path,
    ) -> crate::Result<()> {
        if collection.world.is_some() {
            tracing::warn!(
                file = %path.display(),
                "`world:` ignored; build with the `spec` feature to load schemas"
            );
        }
        Ok(())
    }

    /// Register a parsed collection's mocks, expanding any `serve:` against
    /// the world.
    async fn register_collection(
        &self,
        collection: crate::config::MockCollectionConfig,
        path: &std::path::Path,
    ) -> crate::Result<usize> {
        // Extract the directory of the config file for resolving relative paths
        let config_dir = path.parent();

        // Read global vars for cascading
        let global_vars = self.global_vars.read().clone();

        let definitions = collection
            .into_mock_definitions_in(config_dir, global_vars.as_ref(), Some(&self.world))
            .await
            .map_err(|e| {
                crate::mp_err!("Failed to convert mocks from {}: {}", path.display(), e)
            })?;

        // Validate all templates after conversion
        for def in &definitions {
            Self::validate_mock_templates(def)?;
        }

        let count = definitions.len();
        let source_path = path.to_string_lossy().to_string();
        for mut mock in definitions {
            // Set source file for hot reload tracking
            mock.source_file = Some(source_path.clone());
            self.add_mock(mock);
        }

        Ok(count)
    }

    /// Load a single HAR file and convert to mocks
    async fn load_har_file(&self, path: &std::path::Path) -> crate::Result<usize> {
        use crate::config::HarLoader;

        let loader = HarLoader::new();
        let mock_configs = loader
            .load_from_file(path)
            .await
            .map_err(|e| crate::mp_err!("Failed to load HAR file {}: {}", path.display(), e))?;

        let count = mock_configs.len();
        let source_path = path.to_string_lossy().to_string();

        // Convert MockConfigs to MockDefinitions and add them
        for config in mock_configs {
            let mut definition = config
                .into_mock_definition()
                .await
                .map_err(|e| crate::mp_err!("Failed to convert HAR entry to mock: {e}"))?;
            // Set source file for hot reload tracking
            definition.source_file = Some(source_path.clone());
            self.add_mock(definition);
        }

        Ok(count)
    }

    /// Check if the mock system is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Enable the mock system
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Relaxed);
    }

    /// Disable the mock system
    pub fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }

    /// Add a mock definition to the registry
    pub fn add_mock(&self, mock: MockDefinition) {
        // Registration order is the tiebreak among equal-priority mocks
        // (first registered matches first, MSW semantics) — DashMap
        // iteration order is arbitrary, so record an explicit sequence.
        let seq = self.insertion_counter.fetch_add(1, Ordering::Relaxed);
        self.insertion_seq.insert(mock.id.clone(), seq);
        self.mocks.insert(mock.id.clone(), Arc::new(mock));
        self.sorted_mocks_cache.invalidate();
        self.invalidate_exact_index();
    }

    /// Remove a mock definition by ID.
    ///
    /// Live WS/SSE connections served by the removed mock are closed
    /// (WS close 1001, SSE stream end): a reload replaces the
    /// definition, and connections must not keep running on the stale
    /// handler.
    pub fn remove_mock(&self, id: &str) -> Option<Arc<MockDefinition>> {
        let result = self.mocks.remove(id).map(|(_, v)| v);
        if result.is_some() {
            self.insertion_seq.remove(id);
            self.sorted_mocks_cache.invalidate();
            self.invalidate_exact_index();
            self.streaming_conns.close_mock(id);
        }
        result
    }

    /// Get a mock definition by ID
    pub fn get_mock(&self, id: &str) -> Option<Arc<MockDefinition>> {
        self.mocks.get(id).map(|r| Arc::clone(r.value()))
    }

    /// Get all mock definitions
    pub fn get_all_mocks(&self) -> Vec<Arc<MockDefinition>> {
        self.mocks.iter().map(|r| Arc::clone(r.value())).collect()
    }

    /// All mock definitions in registration order (`listHandlers()` shows
    /// handlers as the user wrote them; the DashMap iterates arbitrarily).
    pub fn get_all_mocks_in_registration_order(&self) -> Vec<Arc<MockDefinition>> {
        let mut mocks: Vec<(u64, Arc<MockDefinition>)> = self
            .mocks
            .iter()
            .map(|r| {
                let seq = self
                    .insertion_seq
                    .get(r.key())
                    .map_or(u64::MAX, |s| *s.value());
                (seq, Arc::clone(r.value()))
            })
            .collect();
        mocks.sort_by_key(|(seq, _)| *seq);
        mocks.into_iter().map(|(_, mock)| mock).collect()
    }

    /// Get all enabled mock definitions sorted by priority (highest first)
    /// Uses an internal cache to avoid re-sorting on every request.
    /// Cache is invalidated when mocks are added, removed, enabled, or disabled.
    pub fn get_enabled_mocks(&self) -> Vec<Arc<MockDefinition>> {
        (*self.get_enabled_mocks_arc()).clone()
    }

    /// Hot-path variant: returns the shared `Arc<Vec<..>>` directly, so callers
    /// on the request path clone only a refcount instead of the whole Vec.
    pub fn get_enabled_mocks_arc(&self) -> Arc<Vec<Arc<MockDefinition>>> {
        // Try to return cached result
        if let Some(cached) = self.sorted_mocks_cache.get_if_valid() {
            return cached;
        }

        // Cache miss - rebuild sorted list
        let mut mocks: Vec<_> = self
            .mocks
            .iter()
            .map(|r| Arc::clone(r.value()))
            .filter(|m| m.enabled)
            .collect();

        // Sort by priority (highest first), then registration order
        // (first registered wins ties — MSW handler-order semantics).
        mocks.sort_by_key(|m| {
            let seq = self
                .insertion_seq
                .get(&m.id)
                .map_or(u64::MAX, |entry| *entry.value());
            (std::cmp::Reverse(m.priority), seq)
        });

        let arc = Arc::new(mocks);
        self.sorted_mocks_cache.update(Arc::clone(&arc));
        arc
    }

    /// Invalidate the exact match index (called when mocks change)
    fn invalidate_exact_index(&self) {
        self.exact_index_epoch.fetch_add(1, Ordering::Release);
    }

    /// Returns true when the exact index is current (no rebuild needed).
    ///
    /// Comparing the index against the *sorted cache's* version cannot work:
    /// invalidation bumps both, so once the sorted cache is refreshed the two
    /// agree again and a stale index reads as current — which let a retired
    /// `once` mock keep matching from the index.
    #[inline]
    fn exact_index_is_current(&self) -> bool {
        self.exact_index_built_epoch.load(Ordering::Acquire)
            == self.exact_index_epoch.load(Ordering::Acquire)
    }

    /// Rebuild the exact match index if needed.
    ///
    /// Single-flighted: when stale under concurrent load, only one thread
    /// rebuilds (others block on `index_rebuild_lock` then see the fresh index
    /// via the double-checked guard), instead of every request rebuilding and
    /// contending on the index write lock.
    fn ensure_exact_index(&self) {
        // Fast, lock-free check on the common (already-current) path.
        if self.exact_index_is_current() {
            return;
        }

        // Stale: serialize rebuilds. The first thread rebuilds; the rest re-check
        // under the guard and return early once the index is fresh.
        let _rebuild_guard = self.index_rebuild_lock.lock();
        if self.exact_index_is_current() {
            return;
        }

        // Read the epoch BEFORE the mocks: a mutation landing between here and
        // the store below advances it, so the index is not marked current for
        // an epoch it never saw.
        let epoch = self.exact_index_epoch.load(Ordering::Acquire);

        // Rebuild: get all enabled mocks sorted by priority
        let enabled = self.get_enabled_mocks_arc();

        #[allow(clippy::disallowed_types)]
        let mut index: ExactMatchIndex = HashMap::with_hasher(BuildNoHashHasher::default());
        let mut has_conditional = false;
        let mut has_body_dependent = false;
        let mut has_header_dependent = false;
        let mut has_href_regex = false;

        for mock in enabled.iter() {
            // Two different questions, kept apart. *Conditional* is about
            // matching: whether a mock can be picked by (method, path) alone.
            // *Dependent* is about response generation: whether the caller has
            // to marshal headers/body before matching at all. Handler mocks
            // answer the second, never the first — their matching predicate is
            // whatever matchers they carry, and folding handler-ness into
            // `is_conditional` disabled the O(1) exact index registry-wide for
            // every other mock in the process.
            let matches_on_body =
                mock.request.body_matcher.is_some() || mock.request.graphql_matcher.is_some();

            // A handler reads whatever it declared; a foreign one is opaque and
            // declares everything. Static and template bodies are left out on
            // purpose — that is the pre-existing rule, and widening it here
            // would make every inline mock force body marshalling.
            let handler_reads_body = match &mock.response.body {
                crate::types::BodySource::Handler { needs, .. } => needs.body,
                crate::types::BodySource::ForeignHandler(_) => true,
                _ => false,
            };

            if matches_on_body || handler_reads_body {
                has_body_dependent = true;
            }

            if !mock.request.header_matchers.is_empty() || mock.response.context_uses_headers {
                has_header_dependent = true;
            }

            let is_conditional = !mock.request.header_matchers.is_empty()
                || matches_on_body
                || !mock.request.query_matchers.is_empty();

            if is_conditional {
                has_conditional = true;
            }

            if mock
                .request
                .url_patterns
                .iter()
                .any(|p| matches!(p, crate::types::UrlPattern::HrefRegex(_)))
            {
                has_href_regex = true;
            }

            // Only index mocks with exactly one Exact URL pattern and no conditionals
            if !is_conditional
                && mock.request.url_patterns.len() == 1
                && mock.request_transforms.is_none()
                && let Some(crate::types::UrlPattern::Exact(path)) =
                    mock.request.url_patterns.first()
            {
                if mock.request.methods.is_empty() {
                    for method in &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"] {
                        let key = exact_match_key(method, path);
                        index.entry(key).or_insert_with(|| Arc::clone(mock));
                    }
                } else {
                    for method in &mock.request.methods {
                        let key = exact_match_key(method.as_str(), path);
                        index.entry(key).or_insert_with(|| Arc::clone(mock));
                    }
                }
            }
        }

        self.has_conditional_mocks
            .store(has_conditional, Ordering::Release);
        self.has_body_dependent_mocks
            .store(has_body_dependent, Ordering::Release);
        self.has_header_dependent_mocks
            .store(has_header_dependent, Ordering::Release);
        self.has_href_regex_mocks
            .store(has_href_regex, Ordering::Release);
        *self.exact_match_index.write() = index;
        self.exact_index_built_epoch.store(epoch, Ordering::Release);
    }

    /// Try to find an exact match via the index. O(1) lookup, zero allocation.
    /// Returns None if no indexed mock matches.
    pub fn try_exact_match(
        &self,
        method: &http::Method,
        path: &str,
    ) -> Option<Arc<MockDefinition>> {
        self.ensure_exact_index();
        let idx = self.exact_match_index.read();
        let key = exact_match_key(method.as_str(), path);
        idx.get(&key).map(Arc::clone)
    }

    /// Combined fast-path lookup: ensures the index once, returns an exact match
    /// only when no conditional mocks exist (so the simple O(1) path is safe).
    /// Folds `has_conditional_mocks()` + `try_exact_match()` into a single
    /// `ensure_exact_index()` call, halving the index checks on the hot path.
    pub fn try_exact_match_simple(
        &self,
        method: &http::Method,
        path: &str,
    ) -> Option<Arc<MockDefinition>> {
        self.ensure_exact_index();
        if self.has_conditional_mocks.load(Ordering::Acquire) {
            return None;
        }
        let idx = self.exact_match_index.read();
        let key = exact_match_key(method.as_str(), path);
        idx.get(&key).map(Arc::clone)
    }

    /// Check if any enabled mock has conditional matchers (headers, body, query, graphql).
    /// When false, cache lookups can be used more aggressively.
    pub fn has_conditional_mocks(&self) -> bool {
        self.ensure_exact_index();
        self.has_conditional_mocks.load(Ordering::Acquire)
    }

    /// Whether any enabled mock matches on the request body (body or graphql matcher).
    /// Callers can skip reading the request body entirely when this is false.
    pub fn needs_request_body(&self) -> bool {
        self.ensure_exact_index();
        self.has_body_dependent_mocks.load(Ordering::Acquire)
    }

    /// Whether any enabled mock needs request headers (header matchers, handler
    /// mocks, or header-referencing templates). Callers can skip marshalling
    /// headers entirely when this is false.
    pub fn needs_request_headers(&self) -> bool {
        self.ensure_exact_index();
        self.has_header_dependent_mocks.load(Ordering::Acquire)
    }

    /// Whether any enabled mock has an `UrlPattern::HrefRegex` pattern.
    /// When false, URL matching never attempts the Host-header href
    /// reconstruction fallback.
    pub fn has_href_regex_mocks(&self) -> bool {
        self.ensure_exact_index();
        self.has_href_regex_mocks.load(Ordering::Acquire)
    }

    /// Clear all mocks from the registry (closing live WS/SSE connections)
    pub fn clear(&self) {
        self.mocks.clear();
        self.insertion_seq.clear();
        self.sorted_mocks_cache.invalidate();
        self.invalidate_exact_index();
        #[allow(clippy::disallowed_types)]
        let empty = HashMap::with_hasher(BuildNoHashHasher::default());
        *self.exact_match_index.write() = empty;
        self.streaming_conns.close_all();
    }

    /// Get the number of mocks in the registry
    pub fn len(&self) -> usize {
        self.mocks.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.mocks.is_empty()
    }

    /// Update a mock definition
    pub fn update_mock(&self, mock: MockDefinition) -> crate::Result<()> {
        if !self.mocks.contains_key(&mock.id) {
            return Err(crate::mp_err!("Mock with ID '{}' not found", mock.id));
        }
        self.mocks.insert(mock.id.clone(), Arc::new(mock));
        self.sorted_mocks_cache.invalidate();
        self.invalidate_exact_index();
        Ok(())
    }

    /// Enable a specific mock by ID
    pub fn enable_mock(&self, id: &str) -> crate::Result<()> {
        // Clone the Arc first, then drop the read lock before inserting
        let arc_mock = self.mocks.get(id).map(|r| Arc::clone(r.value()));

        if let Some(arc_mock) = arc_mock {
            let mut mock = (*arc_mock).clone();
            mock.enabled = true;
            self.mocks.insert(id.into(), Arc::new(mock));
            self.sorted_mocks_cache.invalidate();
            self.invalidate_exact_index();
            Ok(())
        } else {
            Err(crate::mp_err!("Mock with ID '{id}' not found"))
        }
    }

    /// Disable a specific mock by ID
    pub fn disable_mock(&self, id: &str) -> crate::Result<()> {
        let arc_mock = self.mocks.get(id).map(|r| Arc::clone(r.value()));

        if let Some(arc_mock) = arc_mock {
            let mut mock = (*arc_mock).clone();
            mock.enabled = false;
            self.mocks.insert(id.into(), Arc::new(mock));
            self.sorted_mocks_cache.invalidate();
            self.invalidate_exact_index();
            Ok(())
        } else {
            Err(crate::mp_err!("Mock with ID '{id}' not found"))
        }
    }

    // ===== Recording Methods =====

    /// Add a recorded interaction
    pub fn add_recording(&self, id: String, interaction: RecordedInteraction) {
        self.recordings.insert(id, interaction);
    }

    /// Get all recordings
    pub fn get_all_recordings(&self) -> Vec<RecordedInteraction> {
        self.recordings.iter().map(|r| r.value().clone()).collect()
    }

    /// Get count of recordings
    pub fn recordings_count(&self) -> usize {
        self.recordings.len()
    }

    /// Clear all recordings
    pub fn clear_recordings(&self) {
        self.recordings.clear();
    }

    /// Create a new scope for test isolation
    pub fn create_scope(
        &self,
        id: LeanString,
        ttl: Option<std::time::Duration>,
    ) -> crate::Result<ScopeInfo> {
        self.scope_manager.create_scope(id, ttl)
    }

    /// Delete a scope and all its associated mocks
    pub fn delete_scope(&self, scope_id: &str) -> crate::Result<usize> {
        // First delete all mocks in this scope
        let mocks_deleted = self.remove_mocks_by_scope(scope_id);

        // Then delete the scope itself
        self.scope_manager.delete_scope(scope_id)?;

        Ok(mocks_deleted)
    }

    /// Get scope information (including mock count)
    pub fn get_scope_info(&self, scope_id: &str) -> Option<ScopeInfo> {
        let mut info = self.scope_manager.get_scope(scope_id)?;

        // Count mocks in this scope
        info.mock_count = self
            .mocks
            .iter()
            .filter(|entry| entry.value().scope.as_deref() == Some(scope_id))
            .count();

        Some(info)
    }

    /// List all scopes
    pub fn list_scopes(&self) -> Vec<LeanString> {
        self.scope_manager.list_scopes()
    }

    /// Get all mocks belonging to a specific scope
    pub fn get_mocks_by_scope(&self, scope_id: &str) -> Vec<Arc<MockDefinition>> {
        self.mocks
            .iter()
            .filter_map(|entry| {
                let mock = entry.value();
                if mock.scope.as_deref() == Some(scope_id) {
                    Some(Arc::clone(mock))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Remove all mocks belonging to a specific scope
    /// Returns the number of mocks removed
    pub fn remove_mocks_by_scope(&self, scope_id: &str) -> usize {
        let mock_ids: Vec<LeanString> = self
            .mocks
            .iter()
            .filter_map(|entry| {
                if entry.value().scope.as_deref() == Some(scope_id) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let count = mock_ids.len();
        for id in mock_ids {
            self.mocks.remove(&id);
        }

        if count > 0 {
            self.sorted_mocks_cache.invalidate();
        }

        count
    }

    /// Cleanup expired scopes and their mocks
    /// Returns the number of scopes cleaned up
    pub fn cleanup_expired_scopes(&self) -> usize {
        let expired_scopes = self.scope_manager.cleanup_expired();
        let count = expired_scopes.len();

        // Delete mocks for each expired scope
        for scope_id in expired_scopes {
            self.remove_mocks_by_scope(&scope_id);
        }

        count
    }

    /// Check if a scope exists
    pub fn scope_exists(&self, scope_id: &str) -> bool {
        self.scope_manager.exists(scope_id)
    }

    // ===== Call Tracking Methods =====

    /// Enable call tracking for a specific mock
    /// max_calls parameter limits how many calls to store (prevents memory leak)
    pub fn enable_call_tracking(&self, mock_id: &str, max_calls: Option<usize>) {
        let max = max_calls.unwrap_or(self.max_tracked_calls);
        // Insert empty deque to enable tracking for this mock
        self.call_tracking
            .insert(mock_id.into(), VecDeque::with_capacity(max));
    }

    /// Disable call tracking for a specific mock
    pub fn disable_call_tracking(&self, mock_id: &str) {
        self.call_tracking.remove(mock_id);
    }

    /// Check if call tracking is enabled for a mock
    pub fn is_call_tracking_enabled(&self, mock_id: &str) -> bool {
        self.call_tracking.contains_key(mock_id)
    }

    /// Record a call to a mock (only if tracking is enabled for this mock)
    pub fn record_call(&self, mock_id: &str, call: MockCall) {
        if let Some(mut calls) = self.call_tracking.get_mut(mock_id) {
            // Get the max limit from the deque's capacity (set during enable_call_tracking)
            let max_limit = calls.capacity().max(1); // At least 1

            // Limit to max to prevent unbounded memory growth
            // O(1) pop_front instead of O(n) Vec::remove(0)
            while calls.len() >= max_limit {
                calls.pop_front();
            }
            calls.push_back(call);
        }
    }

    /// Get all calls for a specific mock
    pub fn get_calls(&self, mock_id: &str) -> Option<Vec<MockCall>> {
        self.call_tracking
            .get(mock_id)
            .map(|v| v.value().iter().cloned().collect())
    }

    /// How many recorded calls are currently *retained* for a mock.
    ///
    /// Bounded by the tracking window (100 by default), so it stops rising once
    /// the buffer is full and is zero when tracking was never enabled. For
    /// assertions use [`MockRegistry::match_count`].
    pub fn get_call_count(&self, mock_id: &str) -> usize {
        self.call_tracking.get(mock_id).map_or(0, |v| v.len())
    }

    // ===== Match Counting (always on) =====

    /// Record that a mock served a request.
    pub(crate) fn record_match(&self, mock_id: &str) {
        if let Some(counter) = self.match_counts.get(mock_id) {
            counter.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.match_counts
            .entry(mock_id.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Total times a mock has served a request since the last reset.
    pub fn match_count(&self, mock_id: &str) -> u64 {
        self.match_counts
            .get(mock_id)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }

    /// Match counts for every mock that has served at least one request.
    pub fn match_counts(&self) -> Vec<(String, u64)> {
        let mut counts: Vec<(String, u64)> = self
            .match_counts
            .iter()
            .map(|entry| {
                (
                    entry.key().to_string(),
                    entry.value().load(Ordering::Relaxed),
                )
            })
            .collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        counts
    }

    /// Requests served across every mock since the last reset.
    pub fn total_match_count(&self) -> u64 {
        self.match_counts
            .iter()
            .map(|entry| entry.value().load(Ordering::Relaxed))
            .sum()
    }

    /// Reset every match count — call between tests.
    pub fn reset_match_counts(&self) {
        self.match_counts.clear();
    }

    /// Assert how many times a mock served a request.
    ///
    /// # Errors
    /// Returns [`VerifyError`] when the actual count does not satisfy `expected`.
    pub fn verify(&self, mock_id: &str, expected: Expected) -> Result<u64, VerifyError> {
        let actual = self.match_count(mock_id);
        if expected.satisfied_by(actual) {
            Ok(actual)
        } else {
            Err(VerifyError {
                mock_id: mock_id.to_string(),
                expected,
                actual,
                known_mock: self.get_mock(mock_id).is_some(),
            })
        }
    }

    // ===== Coverage =====

    /// Which mocks served a request and which never did.
    ///
    /// Reports the mocks currently registered, so counts left behind by a mock
    /// that was removed (a hot reload, a dropped scope) do not appear — a
    /// coverage report names files a reader can go and fix.
    pub fn coverage(&self) -> CoverageReport {
        let mut served = Vec::new();
        let mut unused = Vec::new();
        let missing = crate::template::get_global_machines().unreached();
        let mut unreached_states: Vec<String> = missing
            .states
            .iter()
            .map(|(machine, state)| format!("{machine}.{state}"))
            .collect();
        let mut untaken_edges: Vec<String> = missing
            .edges
            .iter()
            .map(|(machine, state, event)| format!("{machine}.{state} --{event}->"))
            .collect();
        unreached_states.sort_unstable();
        untaken_edges.sort_unstable();

        for mock in self.get_all_mocks() {
            let entry = MockCoverage {
                mock_id: mock.id.to_string(),
                count: self.match_count(mock.id.as_str()),
                priority: mock.priority,
                enabled: mock.enabled,
                source_file: mock.source_file.clone(),
            };
            if entry.count > 0 {
                served.push(entry);
            } else {
                unused.push(entry);
            }
        }

        served.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.mock_id.cmp(&b.mock_id))
        });
        unused.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.mock_id.cmp(&b.mock_id))
        });

        let total_matches = served.iter().map(|m| m.count).sum();
        CoverageReport {
            served,
            unused,
            total_matches,
            unreached_states,
            untaken_edges,
        }
    }

    // ===== Unmatched Requests =====

    /// Record a request that matched no mock, keyed by its request line.
    ///
    /// Repeats of the same line fold into one entry: a replay run that polls a
    /// missing endpoint would otherwise fill the report with one path.
    pub fn record_unmatched(&self, method: &str, path: &str, query: Option<&str>) {
        self.unmatched_total.fetch_add(1, Ordering::Relaxed);

        let line = request_line(method, path, query);
        let now = Utc::now();

        if let Some(mut entry) = self.unmatched.get_mut(&line) {
            entry.count += 1;
            entry.last_seen = now;
            return;
        }

        // Approximate: a racing insert can overshoot the cap by the number of
        // threads missing at once. Bounding memory is the point, not exactness.
        if self.unmatched.len() >= self.max_unmatched {
            self.unmatched_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        self.unmatched.insert(
            line,
            UnmatchedRequest {
                method: method.to_string(),
                path: path.to_string(),
                query: query.map(ToString::to_string),
                count: 1,
                first_seen: now,
                last_seen: now,
            },
        );
    }

    /// Every distinct request line that matched no mock, most frequent first.
    ///
    /// Carries no explanation: mocks reload, so a stored one goes stale. Post a
    /// line back to the inspector to find out why it missed.
    pub fn unmatched_requests(&self) -> UnmatchedReport {
        let mut requests: Vec<UnmatchedRequest> =
            self.unmatched.iter().map(|e| e.value().clone()).collect();
        requests.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.method.cmp(&b.method))
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.query.cmp(&b.query))
        });

        UnmatchedReport {
            requests,
            total: self.unmatched_total.load(Ordering::Relaxed),
            dropped: self.unmatched_dropped.load(Ordering::Relaxed),
        }
    }

    /// Forget every unmatched request — call between tests.
    pub fn reset_unmatched(&self) {
        self.unmatched.clear();
        self.unmatched_total.store(0, Ordering::Relaxed);
        self.unmatched_dropped.store(0, Ordering::Relaxed);
    }

    /// Clear all recorded calls for a specific mock (keeps tracking enabled)
    pub fn clear_calls(&self, mock_id: &str) {
        if let Some(mut calls) = self.call_tracking.get_mut(mock_id) {
            calls.clear();
        }
    }

    /// Clear all call tracking data (disable tracking for all mocks)
    pub fn clear_all_call_tracking(&self) {
        self.call_tracking.clear();
    }

    /// Get all mock IDs that have call tracking enabled
    pub fn get_tracked_mock_ids(&self) -> Vec<String> {
        self.call_tracking
            .iter()
            .map(|entry| entry.key().to_string())
            .collect()
    }

    // ===== Persistence Store Methods =====

    /// Get access to the persistence store for debugging/inspection
    pub fn get_persistence_store(&self) -> Arc<PersistenceStore> {
        Arc::clone(&self.persistence_store)
    }

    // ===== Hot Reload Methods =====

    /// Get all mock IDs that were loaded from a specific source file
    pub fn get_mocks_by_source(&self, source_file: &str) -> Vec<LeanString> {
        // Normalize the source path for comparison (try to canonicalize, fallback to as-is)
        let source_path = std::path::Path::new(source_file);
        let normalized_source = source_path
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(std::string::ToString::to_string))
            .unwrap_or_else(|| source_file.to_string());

        self.mocks
            .iter()
            .filter_map(|entry| {
                let mock = entry.value();

                if let Some(mock_source) = &mock.source_file {
                    // Try exact match first
                    if mock_source == source_file || mock_source == &normalized_source {
                        return Some(mock.id.clone());
                    }

                    // Try canonicalizing the stored path and compare
                    if let Ok(canonical_mock) = std::path::Path::new(mock_source).canonicalize()
                        && let Some(canonical_str) = canonical_mock.to_str()
                        && (canonical_str == source_file || canonical_str == normalized_source)
                    {
                        return Some(mock.id.clone());
                    }
                }

                None
            })
            .collect()
    }

    /// Reload a single file incrementally
    ///
    /// This removes all mocks from the given file and reloads them.
    /// Returns the number of mocks loaded.
    pub async fn reload_file(&self, path: &std::path::Path) -> crate::Result<usize> {
        let path_str = path.to_string_lossy().to_string();

        // Get all mocks from this file
        let existing_ids = self.get_mocks_by_source(&path_str);

        // Remove all existing mocks from this file
        for id in &existing_ids {
            self.remove_mock(id);
        }

        // An OpenAPI document carries an ordinary data extension, so it has to
        // be recognised by name before `.yaml` is read as a mock collection.
        #[cfg(feature = "spec")]
        if is_openapi_document(path) {
            return self.reload_schema_file(path).await;
        }

        // Check file extension to determine how to load it
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            match ext {
                "json" | "yaml" | "yml" => self.load_collection_file(path).await,
                "har" => self.load_har_file(path).await,
                #[cfg(feature = "scripting")]
                "js" | "mjs" | "ts" | "mts" => {
                    let definitions = self.script_host.reload_file(path).await?;
                    let count = definitions.len();
                    for mock in definitions {
                        self.add_mock(mock);
                    }
                    Ok(count)
                }
                #[cfg(feature = "spec")]
                "graphql" | "gql" | "graphqls" => self.reload_schema_file(path).await,
                _ => Err(crate::mp_err!("Unsupported file extension: {ext}")),
            }
        } else {
            Err(crate::mp_err!("File has no extension"))
        }
    }

    /// Re-read a schema and rebuild the routes bound to it.
    ///
    /// Reading the schema alone is not enough: entity *data* is read through
    /// the world on every request, so live routes pick that up, but the
    /// executable schema is built once from the SDL. A renamed field would
    /// otherwise keep serving under its old name until restart.
    ///
    /// Only collections that actually carry a `serve:` are re-registered, and
    /// only in the schema's own directory — a schema in one tree cannot be
    /// bound by a collection the loader never looked at.
    #[cfg(feature = "spec")]
    async fn reload_schema_file(&self, path: &std::path::Path) -> crate::Result<usize> {
        let entities = self.load_schema_file(path).await?;
        tracing::debug!(file = %path.display(), entities, "reloaded a schema");

        let Some(dir) = path.parent() else {
            return Ok(0);
        };
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            return Ok(0);
        };

        let mut reloaded = 0;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let candidate = entry.path();
            if !candidate
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "json" | "yaml" | "yml"))
            {
                continue;
            }

            let Ok(collection) = crate::config::MockCollectionConfig::from_file(&candidate).await
            else {
                continue;
            };
            if !collection.enabled || collection.mocks.iter().all(|mock| mock.serve.is_none()) {
                continue;
            }

            for id in self.get_mocks_by_source(&candidate.to_string_lossy()) {
                self.remove_mock(&id);
            }
            match self.register_collection(collection, &candidate).await {
                Ok(count) => reloaded += count,
                Err(e) => {
                    tracing::warn!(file = %candidate.display(), error = %e, "failed to rebind after a schema reload");
                }
            }
        }

        Ok(reloaded)
    }

    /// Remove all mocks from a specific source file
    ///
    /// Returns the number of mocks removed.
    pub fn remove_file_mocks(&self, source_file: &str) -> usize {
        let ids = self.get_mocks_by_source(source_file);
        let count = ids.len();
        for id in ids {
            self.remove_mock(&id);
        }
        #[cfg(feature = "scripting")]
        {
            let path = std::path::Path::new(source_file);
            if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "js" | "mjs" | "ts" | "mts"))
            {
                self.script_host.unload_file(path);
            }
        }
        count
    }

    /// Validate templates in a mock definition
    ///
    /// This validates template syntax after conversion to MockDefinition,
    /// to catch errors during config load rather than at runtime.
    fn validate_mock_templates(mock: &crate::engine::types::MockDefinition) -> crate::Result<()> {
        // Check if the response body is a template
        if let crate::engine::types::BodySource::Template {
            source: template, ..
        } = &mock.response.body
            && let Err(e) = crate::template::validate_template(template)
        {
            return Err(crate::mp_err!(
                "Mock '{}': Template validation failed: {}",
                mock.id,
                e
            ));
        }

        Ok(())
    }
}

impl Default for MockRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::engine::types::{BodySource, RequestMatcher, ResponseGenerator};
    use http::{Method, StatusCode};
    use smallvec::smallvec;

    fn create_test_mock(id: &str, priority: u32, enabled: bool) -> MockDefinition {
        MockDefinition {
            id: id.into(),
            priority,
            enabled,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![],
                header_matchers: smallvec![],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
            vars: None,
            streaming: None,
        }
    }

    fn create_test_mock_with_scope(
        id: &str,
        priority: u32,
        enabled: bool,
        scope: Option<LeanString>,
    ) -> MockDefinition {
        MockDefinition {
            id: id.into(),
            priority,
            enabled,
            once: false,
            scope,
            source_file: None,
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![],
                header_matchers: smallvec![],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
            vars: None,
            streaming: None,
        }
    }

    fn handler_body(needs: crate::types::ContextNeeds) -> BodySource {
        BodySource::handler_with_needs(
            std::sync::Arc::new(|_ctx| {
                Box::pin(async {
                    Ok(crate::types::DynamicResponse::body_only(
                        bytes::Bytes::from_static(b"{}"),
                    ))
                })
            }),
            needs,
        )
    }

    fn exact_mock(id: &str, path: &str, body: BodySource) -> MockDefinition {
        let mut mock = create_test_mock(id, 100, true);
        mock.request.url_patterns = smallvec![crate::types::UrlPattern::Exact(path.to_string())];
        mock.response = ResponseGenerator::new(StatusCode::OK, body);
        mock
    }

    #[test]
    fn handler_mocks_do_not_disable_the_exact_index() {
        let registry = MockRegistry::with_mocks(vec![
            exact_mock("inline", "/a", BodySource::inline("{}")),
            exact_mock(
                "handler",
                "/b",
                handler_body(crate::types::ContextNeeds::ALL),
            ),
        ]);

        // A handler matches on method+path like any other mock; treating it as
        // conditional used to bail the O(1) path for every mock in the process.
        assert!(!registry.has_conditional_mocks());
        assert!(
            registry
                .try_exact_match_simple(&Method::GET, "/a")
                .is_some_and(|m| m.id == "inline")
        );
        assert!(
            registry
                .try_exact_match_simple(&Method::GET, "/b")
                .is_some_and(|m| m.id == "handler")
        );
    }

    #[test]
    fn declared_handler_needs_narrow_body_marshalling() {
        let registry = MockRegistry::with_mocks(vec![exact_mock(
            "handler",
            "/b",
            handler_body(crate::types::ContextNeeds::headers_only()),
        )]);
        assert!(!registry.needs_request_body());
        assert!(registry.needs_request_headers());

        let opaque = MockRegistry::with_mocks(vec![exact_mock(
            "foreign",
            "/b",
            BodySource::foreign_handler(std::sync::Arc::new(|_ctx| {
                Box::pin(async {
                    Ok(crate::types::DynamicResponse::body_only(
                        bytes::Bytes::from_static(b"{}"),
                    ))
                })
            })),
        )]);
        assert!(opaque.needs_request_body());
    }

    /// A retired `once` mock must not keep matching from the exact index.
    /// It did: invalidation bumped the index version to exactly what a rebuild
    /// would store, so as soon as anything refreshed the sorted cache the stale
    /// index read as current.
    #[test]
    fn retired_once_mock_leaves_the_exact_index() {
        for body in [
            BodySource::inline("{}"),
            handler_body(crate::types::ContextNeeds::ALL),
        ] {
            let mut once_mock = exact_mock("once", "/api/once", body.clone());
            once_mock.once = true;
            let fallback = exact_mock("fallback", "/api/once", body);

            let registry = MockRegistry::with_mocks(vec![once_mock, fallback]);
            let matcher = crate::engine::matcher::MockMatcher::new(registry);
            let hm = http::HeaderMap::new();

            let first = matcher
                .find_match(&Method::GET, "/api/once", None, &hm, None)
                .map(|m| m.mock.id.to_string());
            assert_eq!(first.as_deref(), Some("once"));

            // The NAPI lane keeps walking the chain after the first hit, and
            // that walk takes the slow path, which refreshes the sorted cache.
            let _ = matcher.find_match_excluding(
                &Method::GET,
                "/api/once",
                None,
                &hm,
                None,
                &["once".to_string()],
            );

            let second = matcher
                .find_match(&Method::GET, "/api/once", None, &hm, None)
                .map(|m| m.mock.id.to_string());
            assert_eq!(second.as_deref(), Some("fallback"));
        }
    }

    #[test]
    fn test_new_registry() {
        let registry = MockRegistry::new();
        assert!(registry.is_enabled());
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_registry_with_mocks() {
        let mocks = vec![
            create_test_mock("mock1", 100, true),
            create_test_mock("mock2", 200, true),
        ];
        let registry = MockRegistry::with_mocks(mocks);

        assert_eq!(registry.len(), 2);
        assert!(registry.get_mock("mock1").is_some());
        assert!(registry.get_mock("mock2").is_some());
    }

    #[test]
    fn test_enable_disable() {
        let registry = MockRegistry::new();

        assert!(registry.is_enabled());

        registry.disable();
        assert!(!registry.is_enabled());

        registry.enable();
        assert!(registry.is_enabled());
    }

    #[test]
    fn test_add_mock() {
        let registry = MockRegistry::new();
        let mock = create_test_mock("test", 100, true);

        registry.add_mock(mock);
        assert_eq!(registry.len(), 1);
        assert!(registry.get_mock("test").is_some());
    }

    #[test]
    fn test_remove_mock() {
        let registry = MockRegistry::new();
        let mock = create_test_mock("test", 100, true);

        registry.add_mock(mock);
        assert_eq!(registry.len(), 1);

        let removed = registry.remove_mock("test");
        assert!(removed.is_some());
        assert_eq!(registry.len(), 0);

        let not_found = registry.remove_mock("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_get_mock() {
        let registry = MockRegistry::new();
        let mock = create_test_mock("test", 100, true);

        registry.add_mock(mock);

        let retrieved = registry.get_mock("test");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, "test");

        let not_found = registry.get_mock("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_get_all_mocks() {
        let registry = MockRegistry::new();
        registry.add_mock(create_test_mock("mock1", 100, true));
        registry.add_mock(create_test_mock("mock2", 200, true));
        registry.add_mock(create_test_mock("mock3", 150, true));

        let all_mocks = registry.get_all_mocks();
        assert_eq!(all_mocks.len(), 3);
    }

    #[test]
    fn test_get_enabled_mocks() {
        let registry = MockRegistry::new();
        registry.add_mock(create_test_mock("mock1", 100, true));
        registry.add_mock(create_test_mock("mock2", 200, false));
        registry.add_mock(create_test_mock("mock3", 150, true));

        let enabled = registry.get_enabled_mocks();
        assert_eq!(enabled.len(), 2);

        // Should be sorted by priority (highest first)
        assert_eq!(enabled[0].id, "mock3");
        assert_eq!(enabled[0].priority, 150);
        assert_eq!(enabled[1].id, "mock1");
        assert_eq!(enabled[1].priority, 100);
    }

    #[test]
    fn test_clear() {
        let registry = MockRegistry::new();
        registry.add_mock(create_test_mock("mock1", 100, true));
        registry.add_mock(create_test_mock("mock2", 200, true));

        assert_eq!(registry.len(), 2);

        registry.clear();
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
    }

    #[test]
    fn test_update_mock() {
        let registry = MockRegistry::new();
        registry.add_mock(create_test_mock("test", 100, true));

        let updated = create_test_mock("test", 200, false);
        let result = registry.update_mock(updated);
        assert!(result.is_ok());

        let retrieved = registry.get_mock("test").unwrap();
        assert_eq!(retrieved.priority, 200);
        assert!(!retrieved.enabled);
    }

    #[test]
    fn test_update_nonexistent_mock() {
        let registry = MockRegistry::new();
        let mock = create_test_mock("nonexistent", 100, true);

        let result = registry.update_mock(mock);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_enable_disable_mock() {
        let registry = MockRegistry::new();
        registry.add_mock(create_test_mock("test", 100, true));

        let result = registry.disable_mock("test");
        assert!(result.is_ok());
        assert!(!registry.get_mock("test").unwrap().enabled);

        let result = registry.enable_mock("test");
        assert!(result.is_ok());
        assert!(registry.get_mock("test").unwrap().enabled);
    }

    #[test]
    fn test_enable_disable_nonexistent_mock() {
        let registry = MockRegistry::new();

        let result = registry.enable_mock("nonexistent");
        assert!(result.is_err());

        let result = registry.disable_mock("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_priority_sorting() {
        let registry = MockRegistry::new();
        registry.add_mock(create_test_mock("low", 10, true));
        registry.add_mock(create_test_mock("high", 1000, true));
        registry.add_mock(create_test_mock("medium", 100, true));

        let enabled = registry.get_enabled_mocks();
        assert_eq!(enabled[0].id, "high");
        assert_eq!(enabled[1].id, "medium");
        assert_eq!(enabled[2].id, "low");
    }

    // ===== Scope Tests =====

    #[test]
    fn test_create_scope() {
        let registry = MockRegistry::new();

        let info = registry.create_scope("test-scope".into(), None).unwrap();
        assert_eq!(info.id, "test-scope");
        assert!(info.expires_at.is_none());
        assert_eq!(info.mock_count, 0);
    }

    #[test]
    fn test_create_scope_with_ttl() {
        let registry = MockRegistry::new();

        let ttl = std::time::Duration::from_hours(1);
        let info = registry
            .create_scope("test-scope".into(), Some(ttl))
            .unwrap();

        assert_eq!(info.id, "test-scope");
        assert!(info.expires_at.is_some());
    }

    #[test]
    fn test_delete_scope() {
        let registry = MockRegistry::new();

        registry.create_scope("test-scope".into(), None).unwrap();
        assert!(registry.scope_exists("test-scope"));

        let count = registry.delete_scope("test-scope").unwrap();
        assert_eq!(count, 0); // No mocks in the scope
        assert!(!registry.scope_exists("test-scope"));
    }

    #[test]
    fn test_delete_scope_with_mocks() {
        let registry = MockRegistry::new();

        // Create scope
        registry.create_scope("test-scope".into(), None).unwrap();

        // Add mocks to scope
        registry.add_mock(create_test_mock_with_scope(
            "mock1",
            100,
            true,
            Some("test-scope".into()),
        ));
        registry.add_mock(create_test_mock_with_scope(
            "mock2",
            200,
            true,
            Some("test-scope".into()),
        ));
        registry.add_mock(create_test_mock("mock3", 150, true)); // Not in scope

        assert_eq!(registry.len(), 3);

        // Delete scope and its mocks
        let count = registry.delete_scope("test-scope").unwrap();
        assert_eq!(count, 2); // 2 mocks deleted
        assert_eq!(registry.len(), 1); // Only mock3 remains
        assert!(registry.get_mock("mock3").is_some());
        assert!(registry.get_mock("mock1").is_none());
        assert!(registry.get_mock("mock2").is_none());
    }

    #[test]
    fn test_get_scope_info() {
        let registry = MockRegistry::new();

        registry.create_scope("test-scope".into(), None).unwrap();
        registry.add_mock(create_test_mock_with_scope(
            "mock1",
            100,
            true,
            Some("test-scope".into()),
        ));
        registry.add_mock(create_test_mock_with_scope(
            "mock2",
            200,
            true,
            Some("test-scope".into()),
        ));

        let info = registry.get_scope_info("test-scope").unwrap();
        assert_eq!(info.id, "test-scope");
        assert_eq!(info.mock_count, 2);
    }

    #[test]
    fn test_list_scopes() {
        let registry = MockRegistry::new();

        assert_eq!(registry.list_scopes().len(), 0);

        registry.create_scope("scope1".into(), None).unwrap();
        registry.create_scope("scope2".into(), None).unwrap();

        let scopes = registry.list_scopes();
        assert_eq!(scopes.len(), 2);
        assert!(scopes.contains(&LeanString::from("scope1")));
        assert!(scopes.contains(&LeanString::from("scope2")));
    }

    #[test]
    fn test_get_mocks_by_scope() {
        let registry = MockRegistry::new();

        registry.create_scope("scope1".into(), None).unwrap();
        registry.add_mock(create_test_mock_with_scope(
            "mock1",
            100,
            true,
            Some("scope1".into()),
        ));
        registry.add_mock(create_test_mock_with_scope(
            "mock2",
            200,
            true,
            Some("scope1".into()),
        ));
        registry.add_mock(create_test_mock("mock3", 150, true)); // No scope

        let scope_mocks = registry.get_mocks_by_scope("scope1");
        assert_eq!(scope_mocks.len(), 2);
        assert!(scope_mocks.iter().any(|m| m.id == "mock1"));
        assert!(scope_mocks.iter().any(|m| m.id == "mock2"));
    }

    #[test]
    fn test_cleanup_expired_scopes() {
        let registry = MockRegistry::new();

        // Create scope with very short TTL
        registry
            .create_scope("expired".into(), Some(std::time::Duration::from_nanos(1)))
            .unwrap();
        registry.add_mock(create_test_mock_with_scope(
            "mock1",
            100,
            true,
            Some("expired".into()),
        ));

        // Create scope without TTL
        registry.create_scope("permanent".into(), None).unwrap();
        registry.add_mock(create_test_mock_with_scope(
            "mock2",
            200,
            true,
            Some("permanent".into()),
        ));

        assert_eq!(registry.len(), 2);

        // Sleep to ensure expiry
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Cleanup expired scopes
        let cleaned = registry.cleanup_expired_scopes();
        assert_eq!(cleaned, 1);

        // Verify expired scope and its mock are gone
        assert!(!registry.scope_exists("expired"));
        assert!(registry.get_mock("mock1").is_none());

        // Verify permanent scope and its mock remain
        assert!(registry.scope_exists("permanent"));
        assert!(registry.get_mock("mock2").is_some());
        assert_eq!(registry.len(), 1);
    }
}
