//! Request/response recording and playback system

use super::filters;
use super::formats;
use super::har;
use super::session;

// Re-export public types
pub use super::types::{RecordedInteraction, RecordingSession};
pub use filters::RecordingFilterOptions;
pub use formats::RecordingFormat;

use crate::Result;
use bytes::Bytes;
use chrono::Utc;
use dashmap::DashMap;
use flate2::read::GzDecoder;
use http::{HeaderMap, Method, StatusCode};
use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

// HAR format support - use ::har to avoid confusion with local har module
use ::har::{Har, Spec, v1_2};

/// Mock recorder for capturing requests and responses
pub struct MockRecorder {
    /// Current session ID
    session_id: String,
    /// Session name
    session_name: String,
    /// In-memory storage of interactions
    interactions: Arc<DashMap<String, RecordedInteraction>>,
    /// Storage directory for recordings
    storage_dir: PathBuf,
    /// Recording format
    format: RecordingFormat,
    /// Active file handle for streaming writes
    file_handle: Arc<Mutex<Option<tokio::fs::File>>>,
    /// File path for current recording
    file_path: Arc<Mutex<Option<PathBuf>>>,
    /// Recording filter options
    filter_options: Arc<RecordingFilterOptions>,
    /// Circular buffer for error context tracking
    error_context_buffer: Arc<Mutex<VecDeque<RecordedInteraction>>>,
    /// Counter for pending write tasks
    pending_writes: Arc<AtomicUsize>,
    /// Atomic counter for unique sequential recording numbers (avoids DashMap race)
    recording_counter: Arc<AtomicUsize>,
    /// Tracks whether the first entry has been written to file (for comma logic)
    is_first_write: Arc<AtomicBool>,
}

impl MockRecorder {
    /// Create a new mock recorder
    pub fn new(session_name: impl Into<String>, storage_dir: impl Into<PathBuf>) -> Self {
        Self::with_format(session_name, storage_dir, RecordingFormat::Json)
    }

    /// Create a new mock recorder with a specific format
    pub fn with_format(
        session_name: impl Into<String>,
        storage_dir: impl Into<PathBuf>,
        format: RecordingFormat,
    ) -> Self {
        Self::with_filters(
            session_name,
            storage_dir,
            format,
            RecordingFilterOptions::default(),
        )
    }

    /// Create a new mock recorder with filter options
    pub fn with_filters(
        session_name: impl Into<String>,
        storage_dir: impl Into<PathBuf>,
        format: RecordingFormat,
        filter_options: RecordingFilterOptions,
    ) -> Self {
        let buffer_size = filter_options.error_context_requests;
        Self {
            session_id: Uuid::new_v4().to_string(),
            session_name: session_name.into(),
            interactions: Arc::new(DashMap::new()),
            storage_dir: storage_dir.into(),
            format,
            file_handle: Arc::new(Mutex::new(None)),
            file_path: Arc::new(Mutex::new(None)),
            filter_options: Arc::new(filter_options),
            error_context_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(buffer_size))),
            pending_writes: Arc::new(AtomicUsize::new(0)),
            recording_counter: Arc::new(AtomicUsize::new(0)),
            is_first_write: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Initialize the recording file (call when starting recording session)
    pub async fn init_file(&self) -> Result<PathBuf> {
        // Create storage directory if it doesn't exist
        tokio::fs::create_dir_all(&self.storage_dir).await?;

        // Generate filename
        let filename = format!(
            "{}-{}.{}",
            self.session_name.replace(' ', "-"),
            self.session_id
                .split('-')
                .next()
                .unwrap_or(&self.session_id),
            self.format.extension()
        );

        let path = self.storage_dir.join(&filename);

        // Create the file and write initial structure
        let mut file = tokio::fs::File::create(&path).await?;

        // Write header/opening based on format - all as mock collections
        match self.format {
            RecordingFormat::Json => {
                // Write initial JSON mock collection structure manually to ensure correct field order
                // We need name, description, enabled BEFORE mocks array, so we can't rely on serde_json
                // alphabetical ordering which puts "mocks" before "name"
                let name_value = format!("Recording: {}", self.session_name);
                let description_value =
                    format!("Auto-generated from recording session {}", self.session_id);

                // Manually construct JSON with correct field order: name, description, enabled, mocks
                let header_json = format!(
                    "{{\n  \"name\": \"{}\",\n  \"description\": \"{}\",\n  \"enabled\": true,\n  \"mocks\": [",
                    name_value.replace('\"', "\\\""),
                    description_value.replace('\"', "\\\"")
                );

                file.write_all(header_json.as_bytes()).await?;
                file.flush().await?;
            }
            RecordingFormat::Yaml => {
                // Write YAML mock collection header
                let header = format!(
                    "name: \"Recording: {}\"\ndescription: \"Auto-generated from recording session {}\"\nenabled: true\nmocks:\n",
                    self.session_name, self.session_id
                );
                file.write_all(header.as_bytes()).await?;
                file.flush().await?;
            }
            RecordingFormat::Har => {
                // Write initial HAR structure
                let har = Har {
                    log: Spec::V1_2(v1_2::Log {
                        creator: v1_2::Creator {
                            name: crate::core::app_name().to_string(),
                            version: env!("CARGO_PKG_VERSION").to_string(),
                            comment: None,
                        },
                        browser: None,
                        pages: None,
                        entries: vec![],
                        comment: None,
                    }),
                };

                let mut json_str = serde_json::to_string_pretty(&har)?;

                // Prepare for streaming by positioning at the entries array
                // Remove the closing brackets and prepare for appending
                if let Some(pos) = json_str.rfind(']') {
                    json_str.truncate(pos);
                    file.write_all(json_str.as_bytes()).await?;
                    file.flush().await?;
                }
            }
        }

        // Store file handle and path
        *self.file_handle.lock().await = Some(file);
        *self.file_path.lock().await = Some(path.clone());

        Ok(path)
    }

    /// Finalize the recording file (call when ending recording session)
    pub async fn finalize_file(&self) -> Result<()> {
        // Wait for all pending write tasks to complete
        let max_wait = std::time::Duration::from_secs(10);
        let start = std::time::Instant::now();

        loop {
            let pending = self.pending_writes.load(Ordering::Acquire);
            if pending == 0 {
                // All writes completed
                break;
            }

            if start.elapsed() > max_wait {
                tracing::warn!(
                    "Timeout waiting for pending writes to complete. {} write(s) still pending.",
                    pending
                );
                break;
            }

            // Sleep a bit before checking again
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Now close the file
        let mut handle_guard = self.file_handle.lock().await;

        if let Some(mut file) = handle_guard.take() {
            match self.format {
                RecordingFormat::Json => {
                    // Close the JSON array and object
                    file.write_all(b"\n  ]\n}\n").await?;
                }
                RecordingFormat::Har => {
                    // Close the HAR entries array and log object
                    file.write_all(b"\n    ]\n  }\n}\n").await?;
                }
                RecordingFormat::Yaml => {
                    // YAML format doesn't need closing
                }
            }
            file.flush().await?;
            file.sync_all().await?;
        }

        Ok(())
    }

    /// Decompress gzip-encoded data if needed
    /// Returns (decompressed_data, was_decompressed)
    fn decompress_if_gzipped(data: &Bytes, headers: &HeaderMap) -> (Bytes, bool) {
        // Check if content is gzip-encoded
        let is_gzipped = headers
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("gzip"));

        if !is_gzipped {
            return (data.clone(), false);
        }

        // Try to decompress
        let mut decoder = GzDecoder::new(&data[..]);
        let mut decompressed = Vec::new();
        match decoder.read_to_end(&mut decompressed) {
            Ok(_) => (Bytes::from(decompressed), true),
            Err(e) => {
                tracing::warn!("Failed to decompress gzip data: {}, using original", e);
                (data.clone(), false)
            }
        }
    }

    /// Check if a request should be recorded based on filter options
    fn should_record(
        &self,
        uri: &str,
        status: StatusCode,
        duration: Duration,
        _response_headers: Option<&HeaderMap>,
    ) -> bool {
        // Check URL filter (include only matching URLs)
        if let Some(ref filter_url) = self.filter_options.filter_url
            && !filter_url.is_match(uri)
        {
            return false;
        }

        // Check exclude patterns (exclude matching URLs)
        for pattern in &self.filter_options.exclude_patterns {
            if pattern.is_match(uri) {
                return false;
            }
        }

        // Check status code filters (mutually exclusive)
        let status_code = status.as_u16();
        if self.filter_options.capture_errors_only {
            // Only capture errors (4xx, 5xx)
            if !(400..=599).contains(&status_code) {
                return false;
            }
        } else if self.filter_options.capture_success_only {
            // Only capture successful responses (2xx) - default behavior
            if !(200..=299).contains(&status_code) {
                return false;
            }
        }
        // If neither flag is set, record all status codes

        // Check minimum duration
        if let Some(min_duration) = self.filter_options.min_duration
            && duration < min_duration
        {
            return false;
        }

        true
    }

    /// Record a request/response interaction
    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        &self,
        method: &Method,
        uri: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        request_body: Option<&Bytes>,
        status: StatusCode,
        response_headers: &HeaderMap,
        response_body: &Bytes,
        duration: Duration,
    ) -> Result<String> {
        // Check if this request should be recorded
        if !self.should_record(uri, status, duration, Some(response_headers)) {
            return Ok(String::new()); // Return empty ID for filtered requests
        }

        let interaction_id = Uuid::new_v4().to_string();
        let is_error = status.is_client_error() || status.is_server_error();

        // Convert request headers to serializable format
        let req_headers: Vec<(String, String)> = headers
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();

        // Decompress response if gzipped
        let (decompressed_response, was_decompressed) =
            Self::decompress_if_gzipped(response_body, response_headers);

        // Convert response headers to serializable format, filtering out problematic headers if decompressed
        let resp_headers: Vec<(String, String)> = response_headers
            .iter()
            .filter_map(|(k, v)| {
                let key_lower = k.as_str().to_lowercase();

                // Skip content-encoding if we decompressed the response
                if was_decompressed && key_lower == "content-encoding" {
                    return None;
                }

                // Update content-length to match decompressed size if we decompressed
                if was_decompressed && key_lower == "content-length" {
                    return Some((k.to_string(), decompressed_response.len().to_string()));
                }

                v.to_str().ok().map(|v| (k.to_string(), v.to_string()))
            })
            .collect();

        // Convert body to string (handle binary data gracefully)
        let request_body_str = request_body
            .and_then(|b| String::from_utf8(b.to_vec()).ok())
            .or(request_body.map(|b| format!("<binary data: {} bytes>", b.len())));

        let response_body_str = String::from_utf8(decompressed_response.to_vec())
            .unwrap_or_else(|_| format!("<binary data: {} bytes>", decompressed_response.len()));

        let interaction = RecordedInteraction {
            id: interaction_id.clone(),
            timestamp: Utc::now(),
            request: super::types::RecordedRequest {
                method: method.to_string(),
                uri: uri.to_string(),
                query: query.map(String::from),
                headers: req_headers,
                body: request_body_str,
            },
            response: super::types::RecordedResponse {
                status: status.as_u16(),
                headers: resp_headers,
                body: response_body_str,
            },
            duration,
        };

        // Store in memory
        self.interactions
            .insert(interaction_id.clone(), interaction.clone());

        // Handle error context tracking for auto-export
        if self.filter_options.auto_export_on_error
            && self.filter_options.error_context_requests > 0
        {
            let mut buffer = self.error_context_buffer.lock().await;

            if is_error {
                // Error detected - export context
                tracing::warn!(
                    "Error detected (status {}), auto-exporting recording with context",
                    status.as_u16()
                );

                // Trigger auto-export in background
                let recorder_clone = self.clone_for_export();
                let buffer_snapshot: Vec<RecordedInteraction> = buffer.iter().cloned().collect();

                tokio::spawn(async move {
                    if let Err(e) = recorder_clone.export_error_context(buffer_snapshot).await {
                        tracing::error!("Failed to auto-export error context: {}", e);
                    }
                });

                // Clear buffer after export
                buffer.clear();
            } else {
                // Normal request - add to circular buffer
                if buffer.len() >= self.filter_options.error_context_requests {
                    buffer.pop_front();
                }
                buffer.push_back(interaction.clone());
            }
        }

        // Write to file incrementally (non-blocking)
        let file_handle = Arc::clone(&self.file_handle);
        let format = self.format;
        // Use atomic counter for unique sequential recording numbers.
        // This avoids the race condition where concurrent calls to record() both read
        // self.interactions.len() after both inserts, getting the same count value.
        let recording_number = self.recording_counter.fetch_add(1, Ordering::SeqCst) + 1;
        let storage_dir = self.storage_dir.clone();
        let pending_writes = Arc::clone(&self.pending_writes);
        let strip_delay = self.filter_options.strip_delay;
        let is_first_write = Arc::clone(&self.is_first_write);

        // Increment pending writes counter
        pending_writes.fetch_add(1, Ordering::Release);

        // Spawn a task to write asynchronously without blocking the proxy
        tokio::spawn(async move {
            if let Err(e) = Self::append_interaction_to_file(
                file_handle,
                &interaction,
                format,
                recording_number,
                storage_dir,
                strip_delay,
                is_first_write,
            )
            .await
            {
                tracing::error!("Failed to write interaction to file: {}", e);
            }
            // Decrement counter when done
            pending_writes.fetch_sub(1, Ordering::Release);
        });

        Ok(interaction_id)
    }

    /// Determine if response body should be stored in a file vs inline
    fn should_use_file_body(body: &str, headers: &[(String, String)]) -> bool {
        // Size threshold: 100KB
        const SIZE_THRESHOLD: usize = 100 * 1024;

        // Check size first
        if body.len() > SIZE_THRESHOLD {
            return true;
        }

        // Check content-type header for binary/large content types
        let content_type = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.to_lowercase());

        if let Some(ct) = content_type {
            // Binary content types that should use files
            let file_types = [
                "text/html",
                "text/css",
                "image/",
                "video/",
                "audio/",
                "application/pdf",
                "application/zip",
                "application/octet-stream",
                "font/",
            ];

            // Use file for non-JSON binary types
            if file_types.iter().any(|ft| ct.contains(ft)) {
                return true;
            }
        }

        false
    }

    /// Detect if a request is GraphQL and extract operation details
    fn detect_graphql_request(
        uri: &str,
        method: &str,
        request_body: Option<&str>,
    ) -> Option<crate::config::GraphQLMatchConfig> {
        use crate::config::GraphQLMatchConfig;
        use crate::config::matcher::IntrospectionMatchConfig;
        use rustc_hash::FxHashMap;

        // 1. Check if it's a GraphQL endpoint (POST to /graphql or similar)
        if method != "POST" || !uri.to_lowercase().contains("graphql") {
            return None;
        }

        // 2. Parse request body as JSON
        let request_body = request_body?;
        let body_json: serde_json::Value = serde_json::from_str(request_body).ok()?;

        // 3. Check if it has a "query" field (required for GraphQL)
        let query = body_json.get("query")?.as_str()?;

        // 4. Extract operation name (optional)
        let operation_name = body_json
            .get("operationName")
            .and_then(|v| v.as_str())
            .map(String::from);

        // 5. Detect operation type from query string
        let query_trimmed = query.trim();
        let operation_type = if query_trimmed.starts_with("query")
            || query_trimmed.starts_with("query ")
        {
            Some("query")
        } else if query_trimmed.starts_with("mutation") || query_trimmed.starts_with("mutation ") {
            Some("mutation")
        } else if query_trimmed.starts_with("subscription")
            || query_trimmed.starts_with("subscription ")
        {
            Some("subscription")
        } else if query_trimmed.contains("__schema") || query_trimmed.contains("__type") {
            // Introspection query without explicit type
            None
        } else {
            // Assume query if no explicit type (GraphQL default)
            Some("query")
        };

        // 6. Check for introspection
        let introspection = if query.contains("__schema") {
            Some(IntrospectionMatchConfig::String("schema".to_string()))
        } else if query.contains("__type") {
            Some(IntrospectionMatchConfig::String("type".to_string()))
        } else {
            None
        };

        // 7. Extract variables (convert to FxHashMap<String, serde_json::Value>)
        let variables = if let Some(vars) = body_json.get("variables").and_then(|v| v.as_object()) {
            vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            FxHashMap::default()
        };

        // 8. Build GraphQLMatchConfig based on what we found
        if introspection.is_some() {
            // Introspection query
            Some(GraphQLMatchConfig::Structured {
                operation: None,
                query: None,
                mutation: None,
                subscription: None,
                introspection,
                variables,
            })
        } else if let Some(op_name) = operation_name {
            // Has operation name - use structured syntax
            Some(GraphQLMatchConfig::Structured {
                operation: None,
                query: if operation_type == Some("query") {
                    Some(op_name.clone())
                } else {
                    None
                },
                mutation: if operation_type == Some("mutation") {
                    Some(op_name.clone())
                } else {
                    None
                },
                subscription: if operation_type == Some("subscription") {
                    Some(op_name)
                } else {
                    None
                },
                introspection: None,
                variables,
            })
        } else {
            // No operation name - use boolean syntax (match any GraphQL of this type)
            // Any GraphQL operation type gets boolean match syntax
            Some(GraphQLMatchConfig::Boolean(true))
        }
    }

    /// Append an interaction to the recording file as a mock definition (non-blocking)
    async fn append_interaction_to_file(
        file_handle: Arc<Mutex<Option<tokio::fs::File>>>,
        interaction: &RecordedInteraction,
        format: RecordingFormat,
        recording_number: usize,
        storage_dir: PathBuf,
        strip_delay: bool,
        is_first_write: Arc<AtomicBool>,
    ) -> Result<()> {
        use crate::config::{MatchConfig, MockConfig, ReturnConfig};
        use rustc_hash::FxHashMap;

        let mut handle_guard = file_handle.lock().await;

        if let Some(file) = handle_guard.as_mut() {
            // Determine if we should use file-based body storage
            let use_file = Self::should_use_file_body(
                &interaction.response.body,
                &interaction.response.headers,
            );

            // Convert interaction to mock config
            let mock_id = format!("recorded-{recording_number}");

            // Build full URL with query parameters
            let full_url = if let Some(ref query) = interaction.request.query {
                format!("{}?{}", interaction.request.uri, query)
            } else {
                interaction.request.uri.clone()
            };

            // Prepare body - write to file if needed
            let body_str = if use_file {
                // Create bodies directory
                let bodies_dir = storage_dir.join("bodies");
                tokio::fs::create_dir_all(&bodies_dir).await?;

                // Write body to file
                let body_filename = format!("{mock_id}.body");
                let body_path = bodies_dir.join(&body_filename);
                tokio::fs::write(&body_path, interaction.response.body.as_bytes()).await?;

                format!("bodies/{body_filename}")
            } else {
                String::new()
            };

            let use_file_field = !body_str.is_empty();
            let inline_body = if use_file_field {
                None
            } else {
                Some(interaction.response.body.clone())
            };
            let file_ref = if use_file_field { Some(body_str) } else { None };

            // Detect if this is a GraphQL request
            let graphql_config = Self::detect_graphql_request(
                &interaction.request.uri,
                &interaction.request.method,
                interaction.request.body.as_deref(),
            );

            let mock_config = MockConfig {
                id: mock_id.into(),
                description: None,
                // Priority based on URL specificity + recording order
                // More specific URLs (with IDs, longer paths) get higher priority
                // Recording order used as tiebreaker (earlier = slightly higher)
                priority: Self::calculate_priority_from_url(&full_url, recording_number),
                enabled: true,
                scope: None, // Recorded mocks don't belong to a scope by default
                vars: None,
                match_config: Some(MatchConfig {
                    method: None,
                    methods: vec![interaction.request.method.clone()],
                    url: None,
                    urls: vec![full_url],
                    headers: FxHashMap::default(),
                    query: FxHashMap::default(),
                    body: FxHashMap::default(),
                    graphql: graphql_config,
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(interaction.response.status),
                    headers: interaction.response.headers.iter().cloned().collect(),
                    body: inline_body,
                    template: None,
                    file: file_ref,
                    template_file: None,
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: if strip_delay {
                    None
                } else {
                    Some(format!("{}ms", interaction.duration.as_millis()))
                },
            };

            // Determine if this is the first entry being written to the file.
            // We use an atomic swap inside the file mutex lock to guarantee that exactly
            // one writer sees `true` regardless of concurrent task scheduling order.
            let first = is_first_write.swap(false, Ordering::SeqCst);

            match format {
                RecordingFormat::Json => {
                    // Add comma if not first entry
                    let prefix = if first { "\n" } else { ",\n" };

                    // Serialize mock config with proper indentation
                    let json_str = serde_json::to_string_pretty(&mock_config)?;
                    let indented = json_str
                        .lines()
                        .map(|line| format!("    {line}"))
                        .collect::<Vec<_>>()
                        .join("\n");

                    file.write_all(prefix.as_bytes()).await?;
                    file.write_all(indented.as_bytes()).await?;
                }
                RecordingFormat::Yaml => {
                    // Write as YAML list item
                    let yaml_str = serde_yaml::to_string(&mock_config)?;
                    let indented = yaml_str
                        .lines()
                        .enumerate()
                        .map(|(i, line)| {
                            if i == 0 {
                                format!("  - {line}")
                            } else {
                                format!("    {line}")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    file.write_all(indented.as_bytes()).await?;
                    file.write_all(b"\n").await?;
                }
                RecordingFormat::Har => {
                    // Add comma if not first entry
                    let prefix = if first { "\n" } else { ",\n" };

                    // Convert interaction to HAR entry
                    let har_entry = har::to_har_entry(interaction);

                    // Serialize HAR entry with proper indentation
                    let json_str = serde_json::to_string_pretty(&har_entry)?;
                    let indented = json_str
                        .lines()
                        .map(|line| format!("      {line}"))
                        .collect::<Vec<_>>()
                        .join("\n");

                    file.write_all(prefix.as_bytes()).await?;
                    file.write_all(indented.as_bytes()).await?;
                }
            }

            // Flush to ensure data is written (but don't sync_all for performance)
            file.flush().await?;
        }

        Ok(())
    }

    /// Get the number of recorded interactions
    pub fn count(&self) -> usize {
        self.interactions.len()
    }

    /// Get all interactions
    pub fn get_all(&self) -> Vec<RecordedInteraction> {
        self.interactions
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get a specific interaction by ID
    pub fn get(&self, id: &str) -> Option<RecordedInteraction> {
        self.interactions.get(id).map(|entry| entry.value().clone())
    }

    /// Get filter options
    pub fn filter_options(&self) -> &RecordingFilterOptions {
        &self.filter_options
    }

    /// Get the recording format
    pub fn get_format(&self) -> RecordingFormat {
        self.format
    }

    /// Get the current file path (if file has been initialized)
    pub async fn get_file_path(&self) -> Option<PathBuf> {
        self.file_path.lock().await.clone()
    }

    /// Clear all recorded interactions
    pub fn clear(&self) {
        self.interactions.clear();
    }

    /// Save the recording session to disk as mock collection
    pub async fn save(&self, format: RecordingFormat) -> Result<PathBuf> {
        // Ensure storage directory exists
        tokio::fs::create_dir_all(&self.storage_dir).await?;

        let filename = format!(
            "{}-{}.{}",
            self.session_name.replace(' ', "-"),
            self.session_id
                .split('-')
                .next()
                .unwrap_or(&self.session_id),
            format.extension()
        );

        let file_path = self.storage_dir.join(&filename);

        // All formats now save as mock collections
        match format {
            RecordingFormat::Json => {
                let collection = self.create_mock_collection();
                // Write body files for file-based bodies
                self.write_body_files(&collection).await?;
                let content = serde_json::to_string_pretty(&collection)?;
                tokio::fs::write(&file_path, content).await?;
            }
            RecordingFormat::Yaml => {
                let collection = self.create_mock_collection();
                // Write body files for file-based bodies
                self.write_body_files(&collection).await?;
                let content = serde_yaml::to_string(&collection)?;
                tokio::fs::write(&file_path, content).await?;
            }
            RecordingFormat::Har => {
                // Create HAR format
                let har = self.create_har_log();
                let content = serde_json::to_string_pretty(&har)?;
                tokio::fs::write(&file_path, content).await?;
            }
        }

        Ok(file_path)
    }

    /// Write body files for mocks that use file-based body storage
    async fn write_body_files(
        &self,
        collection: &crate::config::MockCollectionConfig,
    ) -> Result<()> {
        // Create bodies directory
        let bodies_dir = self.storage_dir.join("bodies");
        tokio::fs::create_dir_all(&bodies_dir).await?;

        let interactions = self.get_all();

        for (mock, interaction) in collection.mocks.iter().zip(interactions.iter()) {
            if let Some(ref response_config) = mock.response_config
                && let Some(file_ref) = response_config.file_ref()
            {
                let filename = std::path::Path::new(file_ref)
                    .file_name()
                    .ok_or_else(|| crate::mp_err!("Invalid body file path: {file_ref}"))?;

                let body_file_path = bodies_dir.join(filename);
                tokio::fs::write(&body_file_path, &interaction.response.body).await?;
            }
        }

        Ok(())
    }

    /// Create a mock collection from recorded interactions
    fn create_mock_collection(&self) -> crate::config::MockCollectionConfig {
        use crate::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
        use rustc_hash::FxHashMap;

        let mut mocks = Vec::new();
        let interactions = self.get_all();

        for (idx, interaction) in interactions.iter().enumerate() {
            let mock_id = format!("{}-{}", self.session_name.replace(' ', "-"), idx + 1);

            // Determine if we should use file-based body storage
            let use_file = Self::should_use_file_body(
                &interaction.response.body,
                &interaction.response.headers,
            );

            // Build full URL with query parameters
            let full_url = if let Some(ref query) = interaction.request.query {
                format!("{}?{}", interaction.request.uri, query)
            } else {
                interaction.request.uri.clone()
            };

            // Prepare body - use file field for external files, body for inline
            let (inline_body, file_ref) = if use_file {
                (None, Some(format!("bodies/{mock_id}.body")))
            } else {
                (Some(interaction.response.body.clone()), None)
            };

            // Detect if this is a GraphQL request
            let graphql_config = Self::detect_graphql_request(
                &interaction.request.uri,
                &interaction.request.method,
                interaction.request.body.as_deref(),
            );

            let mock_config = MockConfig {
                id: mock_id.as_str().into(),
                description: None,
                priority: 100_u32.saturating_sub(u32::try_from(idx).unwrap_or(u32::MAX)),
                enabled: true,
                scope: None, // Recorded mocks don't belong to a scope by default
                vars: None,
                match_config: Some(MatchConfig {
                    method: None,
                    methods: vec![interaction.request.method.clone()],
                    url: None,
                    urls: vec![full_url],
                    headers: FxHashMap::default(),
                    query: FxHashMap::default(),
                    body: FxHashMap::default(),
                    graphql: graphql_config,
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(interaction.response.status),
                    headers: interaction.response.headers.iter().cloned().collect(),
                    body: inline_body,
                    template: None,
                    file: file_ref,
                    template_file: None,
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: if self.filter_options.strip_delay {
                    None
                } else {
                    Some(format!("{}ms", interaction.duration.as_millis()))
                },
            };

            mocks.push(mock_config);
        }

        MockCollectionConfig {
            name: Some(format!("Recording: {}", self.session_name)),
            description: Some(format!(
                "Auto-generated from recording session {} at {}",
                self.session_id,
                Utc::now().to_rfc3339()
            )),
            enabled: true,
            vars: None,
            mocks,
        }
    }

    /// Create a HAR log from recorded interactions
    fn create_har_log(&self) -> Har {
        let interactions = self.get_all();
        let entries: Vec<v1_2::Entries> = interactions.iter().map(har::to_har_entry).collect();

        Har {
            log: Spec::V1_2(v1_2::Log {
                creator: v1_2::Creator {
                    name: crate::core::app_name().to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    comment: None,
                },
                browser: None,
                pages: None,
                entries,
                comment: Some(format!(
                    "Recording session: {} ({})",
                    self.session_name, self.session_id
                )),
            }),
        }
    }

    /// Load a recording session from disk
    pub async fn load(path: impl AsRef<Path>) -> Result<RecordingSession> {
        session::load_session(path).await
    }

    /// Clone minimal data needed for export (used for auto-export on error)
    fn clone_for_export(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            session_name: session::create_export_session_name(&self.session_name),
            interactions: Arc::new(DashMap::new()),
            storage_dir: self.storage_dir.clone(),
            format: self.format,
            file_handle: Arc::new(Mutex::new(None)),
            file_path: Arc::new(Mutex::new(None)),
            filter_options: Arc::clone(&self.filter_options),
            error_context_buffer: Arc::new(Mutex::new(VecDeque::new())),
            pending_writes: Arc::new(AtomicUsize::new(0)),
            recording_counter: Arc::new(AtomicUsize::new(0)),
            is_first_write: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Export error context to file (includes requests before/after the error)
    async fn export_error_context(
        &self,
        context_interactions: Vec<RecordedInteraction>,
    ) -> Result<()> {
        // Add context interactions to this export instance
        for interaction in context_interactions {
            self.interactions
                .insert(interaction.id.clone(), interaction);
        }

        // Save to file
        let file_path = self.save(self.format).await?;
        tracing::debug!("Auto-exported error context to: {}", file_path.display());

        Ok(())
    }

    /// Calculate priority based on URL specificity
    /// More specific URLs get higher priority to ensure correct matching
    fn calculate_priority_from_url(url: &str, recording_order: usize) -> u32 {
        let mut priority = 500u32; // Base priority (normal tier)

        // Parse URL to extract path
        let path = url.split('?').next().unwrap_or(url);

        // Higher priority for more specific patterns:
        // 1. Path depth (more segments = more specific)
        let path_segments = path.split('/').filter(|s| !s.is_empty()).count();
        priority += u32::try_from(path_segments)
            .unwrap_or(u32::MAX)
            .saturating_mul(10);

        // 2. Presence of numeric IDs (e.g., /api/users/123)
        let has_numeric_id = path.split('/').any(|seg| seg.parse::<u64>().is_ok());
        if has_numeric_id {
            priority += 100; // Paths with IDs are very specific
        }

        // 3. Longer paths are more specific
        priority += u32::try_from(path.len()).unwrap_or(u32::MAX).min(100);

        // 4. Recording order as tiebreaker (earlier = slightly higher)
        // Limit impact to avoid overflow (max 1000 requests before it saturates)
        let order_penalty = u32::try_from(recording_order).unwrap_or(u32::MAX).min(1000);
        priority = priority.saturating_sub(order_penalty / 100);

        priority
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm,
    clippy::match_wildcard_for_single_variants,
    clippy::clone_on_ref_ptr,
    clippy::cast_sign_loss,
    clippy::option_if_let_else,
    clippy::manual_let_else
)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_json_header_serialization() {
        // Test that the JSON header includes the name field
        let session_name = "test-session";
        let session_id = "test-id";

        let header = serde_json::json!({
          "name": format!("Recording: {}", session_name),
          "description": format!("Auto-generated from recording session {}", session_id),
          "enabled": true,
          "mocks": []
        });

        let json_str = serde_json::to_string_pretty(&header).unwrap();

        println!("Generated JSON:\n{json_str}");

        // Verify name is present in the JSON string
        assert!(
            json_str.contains("\"name\""),
            "JSON should contain 'name' field"
        );
        assert!(
            json_str.contains("Recording: test-session"),
            "JSON should contain session name"
        );

        // Verify we can find the closing bracket
        assert!(
            json_str.rfind(']').is_some(),
            "JSON should contain ']' bracket"
        );
    }

    #[tokio::test]
    async fn test_init_and_finalize_file() {
        // Test that init_file and finalize_file create a valid JSON file with name
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("test-session", temp_dir.path(), RecordingFormat::Json);

        // Initialize the file
        let file_path = recorder.init_file().await.unwrap();

        // Finalize the file
        recorder.finalize_file().await.unwrap();

        // Read the file and check its contents
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        println!("Generated file content:\n{content}");

        // Parse as JSON
        let json_value: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Check if name field exists
        assert!(
            json_value.get("name").is_some(),
            "JSON should have 'name' field"
        );

        let name = json_value.get("name").unwrap();
        println!("Name field value: {name:?}");

        assert!(!name.is_null(), "Name should not be null");
        assert_eq!(
            name.as_str().unwrap(),
            "Recording: test-session",
            "Name should match session name"
        );
    }

    #[tokio::test]
    async fn test_record_interaction() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("test-session", temp_dir.path());

        let method = Method::GET;
        let uri = "/api/test";
        let query = Some("foo=bar");
        let headers = HeaderMap::new();
        let req_body = Bytes::from("test request");
        let status = StatusCode::OK;
        let resp_headers = HeaderMap::new();
        let resp_body = Bytes::from("test response");
        let duration = Duration::from_millis(100);

        let id = recorder
            .record(
                &method,
                uri,
                query,
                &headers,
                Some(&req_body),
                status,
                &resp_headers,
                &resp_body,
                duration,
            )
            .await
            .unwrap();

        assert_eq!(recorder.count(), 1);

        let interaction = recorder.get(&id).unwrap();
        assert_eq!(interaction.request.method, "GET");
        assert_eq!(interaction.request.uri, "/api/test");
        assert_eq!(interaction.request.query, Some("foo=bar".to_string()));
        assert_eq!(interaction.response.status, 200);
        assert_eq!(interaction.response.body, "test response");
        assert_eq!(interaction.duration, duration);
    }

    #[tokio::test]
    async fn test_save_and_load_json() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("test-session", temp_dir.path());

        // Record some interactions
        let req_body = Bytes::from("request");
        recorder
            .record(
                &Method::GET,
                "/api/test",
                None,
                &HeaderMap::new(),
                Some(&req_body),
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from("response 1"),
                Duration::from_millis(50),
            )
            .await
            .unwrap();

        // Save as JSON (now saves as mock collection format)
        let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
        assert!(file_path.exists());

        // Load as mock collection
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        let collection: crate::config::MockCollectionConfig =
            serde_json::from_str(&content).unwrap();

        assert!(collection.name.is_some());
        assert_eq!(collection.mocks.len(), 1);
        let first_mock_urls = &collection.mocks[0].match_config.as_ref().unwrap().urls;
        assert_eq!(first_mock_urls[0], "/api/test");
    }

    #[tokio::test]
    async fn test_save_and_load_yaml() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("test-yaml", temp_dir.path());

        recorder
            .record(
                &Method::POST,
                "/api/create",
                None,
                &HeaderMap::new(),
                Some(&Bytes::from(r#"{"name":"test"}"#)),
                StatusCode::CREATED,
                &HeaderMap::new(),
                &Bytes::from(r#"{"id":"123"}"#),
                Duration::from_millis(100),
            )
            .await
            .unwrap();

        // Save as YAML (now saves as mock collection format)
        let file_path = recorder.save(RecordingFormat::Yaml).await.unwrap();
        assert!(file_path.exists());

        // Load as mock collection
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        let collection: crate::config::MockCollectionConfig =
            serde_yaml::from_str(&content).unwrap();

        assert!(collection.name.is_some());
        assert_eq!(collection.mocks.len(), 1);
        let first_mock_methods = &collection.mocks[0].match_config.as_ref().unwrap().methods;
        assert_eq!(first_mock_methods[0], "POST");
    }

    #[tokio::test]
    async fn test_save_yaml_format() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("export-test", temp_dir.path());

        // Record multiple interactions
        recorder
            .record(
                &Method::GET,
                "/api/users",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(r#"{"users":[]}"#),
                Duration::from_millis(50),
            )
            .await
            .unwrap();

        recorder
            .record(
                &Method::GET,
                "/api/files",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(r#"{"files":[]}"#),
                Duration::from_millis(75),
            )
            .await
            .unwrap();

        // Save as YAML format
        let output_path = recorder.save(RecordingFormat::Yaml).await.unwrap();

        assert!(output_path.exists());

        // Verify the content
        let content = tokio::fs::read_to_string(&output_path).await.unwrap();
        println!("=== Saved YAML ===\n{content}\n=== End ===");

        assert!(content.contains("export-test"));
        assert!(content.contains("/api/users"));
        assert!(content.contains("/api/files"));

        // Verify it can be parsed back
        let parsed: crate::config::MockCollectionConfig =
            serde_yaml::from_str(&content).expect("YAML should be valid and parseable");
        assert_eq!(parsed.mocks.len(), 2);
    }

    #[tokio::test]
    async fn test_clear_interactions() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("clear-test", temp_dir.path());

        recorder
            .record(
                &Method::GET,
                "/api/test",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from("test"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        assert_eq!(recorder.count(), 1);

        recorder.clear();
        assert_eq!(recorder.count(), 0);
    }

    #[tokio::test]
    async fn test_har_format_save() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("har-test", temp_dir.path());

        // Record some interactions
        recorder
            .record(
                &Method::GET,
                "/api/users/123",
                Some("fields=name,email"),
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(r#"{"id":"123","name":"Test User"}"#),
                Duration::from_millis(50),
            )
            .await
            .unwrap();

        recorder
            .record(
                &Method::POST,
                "/api/files",
                None,
                &HeaderMap::new(),
                Some(&Bytes::from(r#"{"name":"test.txt"}"#)),
                StatusCode::CREATED,
                &HeaderMap::new(),
                &Bytes::from(r#"{"id":"456","name":"test.txt"}"#),
                Duration::from_millis(100),
            )
            .await
            .unwrap();

        // Save as HAR format
        let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
        assert!(file_path.exists());
        assert!(file_path.extension().unwrap() == "har");

        // Load and verify HAR structure
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        let har: Har = serde_json::from_str(&content).unwrap();

        // Extract log from Spec enum
        let log = match &har.log {
            Spec::V1_2(log) => log,
            _ => panic!("Expected V1_2 spec"),
        };

        assert_eq!(log.creator.name, crate::core::app_name());
        assert_eq!(log.entries.len(), 2);

        // Find entries by URL (order might not be preserved due to DashMap)
        let get_entry = log
            .entries
            .iter()
            .find(|e| e.request.url.contains("/api/users/123"))
            .unwrap();
        let post_entry = log
            .entries
            .iter()
            .find(|e| e.request.url.contains("/api/files"))
            .unwrap();

        // Verify GET entry
        assert_eq!(get_entry.request.method, "GET");
        assert!(get_entry.request.url.contains("/api/users/123"));
        assert!(get_entry.request.url.contains("fields=name,email"));
        assert_eq!(get_entry.response.status, 200);
        assert_eq!(get_entry.response.status_text, "OK");
        assert!(
            get_entry
                .response
                .content
                .text
                .as_ref()
                .unwrap()
                .contains("Test User")
        );

        // Verify POST entry
        assert_eq!(post_entry.request.method, "POST");
        assert!(post_entry.request.url.contains("/api/files"));
        assert_eq!(post_entry.response.status, 201);
        assert_eq!(post_entry.response.status_text, "Created");
        assert!(post_entry.request.post_data.is_some());
    }

    #[tokio::test]
    async fn test_har_streaming_format() {
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("har-stream", temp_dir.path(), RecordingFormat::Har);

        // Initialize file for streaming
        let file_path = recorder.init_file().await.unwrap();

        // Record multiple interactions
        for i in 0..3 {
            recorder
                .record(
                    &Method::GET,
                    &format!("/api/test/{i}"),
                    None,
                    &HeaderMap::new(),
                    None,
                    StatusCode::OK,
                    &HeaderMap::new(),
                    &Bytes::from(format!(r#"{{"id":{i}}}"#)),
                    Duration::from_millis(10),
                )
                .await
                .unwrap();

            // Give async task time to write
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Finalize the file
        recorder.finalize_file().await.unwrap();

        // Verify the HAR file
        assert!(file_path.exists());
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        let har: Har = serde_json::from_str(&content).unwrap();

        // Extract log from Spec enum
        let log = match &har.log {
            Spec::V1_2(log) => log,
            _ => panic!("Expected V1_2 spec"),
        };

        assert_eq!(log.entries.len(), 3);
        assert_eq!(log.entries[0].request.url, "/api/test/0");
        assert_eq!(log.entries[1].request.url, "/api/test/1");
        assert_eq!(log.entries[2].request.url, "/api/test/2");
    }

    #[tokio::test]
    async fn test_har_with_headers() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("har-headers", temp_dir.path());

        let mut req_headers = HeaderMap::new();
        req_headers.insert("authorization", "Bearer token123".parse().unwrap());
        req_headers.insert("content-type", "application/json".parse().unwrap());

        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("content-type", "application/json".parse().unwrap());
        resp_headers.insert("x-request-id", "req-123".parse().unwrap());

        recorder
            .record(
                &Method::GET,
                "/api/test",
                None,
                &req_headers,
                None,
                StatusCode::OK,
                &resp_headers,
                &Bytes::from("{}"),
                Duration::from_millis(25),
            )
            .await
            .unwrap();

        let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        let har: Har = serde_json::from_str(&content).unwrap();

        // Extract log from Spec enum
        let log = match &har.log {
            Spec::V1_2(log) => log,
            _ => panic!("Expected V1_2 spec"),
        };

        let entry = &log.entries[0];

        // Check request headers
        assert!(
            entry
                .request
                .headers
                .iter()
                .any(|h| h.name == "authorization")
        );
        assert!(
            entry
                .request
                .headers
                .iter()
                .any(|h| h.name == "content-type")
        );

        // Check response headers
        assert!(
            entry
                .response
                .headers
                .iter()
                .any(|h| h.name == "content-type")
        );
        assert!(
            entry
                .response
                .headers
                .iter()
                .any(|h| h.name == "x-request-id")
        );
    }

    #[tokio::test]
    async fn test_gzip_decompression_header_fix() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("gzip-test", temp_dir.path());

        // Create a gzipped response body
        let original_body = r#"{"message": "This is a test response that will be gzipped"}"#;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original_body.as_bytes()).unwrap();
        let gzipped_body = encoder.finish().unwrap();

        // Create response headers with content-encoding and content-length for gzipped data
        let mut resp_headers = HeaderMap::new();
        resp_headers.insert("content-encoding", "gzip".parse().unwrap());
        resp_headers.insert(
            "content-length",
            gzipped_body.len().to_string().parse().unwrap(),
        );
        resp_headers.insert("content-type", "application/json".parse().unwrap());

        // Record the interaction
        let id = recorder
            .record(
                &Method::GET,
                "/api/gzip-test",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &resp_headers,
                &Bytes::from(gzipped_body.clone()),
                Duration::from_millis(100),
            )
            .await
            .unwrap();

        // Get the recorded interaction
        let interaction = recorder.get(&id).unwrap();

        // Verify the body was decompressed
        assert_eq!(interaction.response.body, original_body);

        // Verify content-encoding header was removed
        assert!(
            !interaction
                .response
                .headers
                .iter()
                .any(|(k, _)| k.to_lowercase() == "content-encoding")
        );

        // Verify content-length was updated to match decompressed size
        let content_length_header = interaction
            .response
            .headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == "content-length");

        assert!(content_length_header.is_some());
        let (_, length_value) = content_length_header.unwrap();
        assert_eq!(
            length_value.parse::<usize>().unwrap(),
            original_body.len(),
            "Content-Length should match decompressed body size"
        );

        // Verify other headers are preserved
        assert!(
            interaction
                .response
                .headers
                .iter()
                .any(|(k, v)| k.to_lowercase() == "content-type" && v == "application/json")
        );
    }

    #[test]
    fn test_detect_graphql_query_with_operation_name() {
        let uri = "/graphql";
        let method = "POST";
        let body = r#"{"query":"query GetUser { user(id: \"123\") { name } }","operationName":"GetUser","variables":{"id":"123"}}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_some());

        if let Some(crate::config::GraphQLMatchConfig::Structured {
            query,
            mutation,
            subscription,
            variables,
            ..
        }) = result
        {
            assert_eq!(query, Some("GetUser".to_string()));
            assert_eq!(mutation, None);
            assert_eq!(subscription, None);
            assert_eq!(variables.get("id").and_then(|v| v.as_str()), Some("123"));
        } else {
            panic!("Expected Structured GraphQL config");
        }
    }

    #[test]
    fn test_detect_graphql_mutation() {
        let uri = "/api/graphql";
        let method = "POST";
        let body = r#"{"query":"mutation CreateUser { createUser(input: {name: \"John\"}) { id } }","operationName":"CreateUser"}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_some());

        if let Some(crate::config::GraphQLMatchConfig::Structured {
            query, mutation, ..
        }) = result
        {
            assert_eq!(query, None);
            assert_eq!(mutation, Some("CreateUser".to_string()));
        } else {
            panic!("Expected Structured GraphQL config with mutation");
        }
    }

    #[test]
    fn test_detect_graphql_introspection() {
        use crate::config::matcher::IntrospectionMatchConfig;

        let uri = "/graphql";
        let method = "POST";
        let body = r#"{"query":"{ __schema { types { name } } }"}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_some());

        if let Some(crate::config::GraphQLMatchConfig::Structured { introspection, .. }) = result {
            assert_eq!(
                introspection,
                Some(IntrospectionMatchConfig::String("schema".to_string()))
            );
        } else {
            panic!("Expected introspection GraphQL config");
        }
    }

    #[test]
    fn test_detect_graphql_no_operation_name() {
        let uri = "/graphql";
        let method = "POST";
        let body = r#"{"query":"{ user(id: \"123\") { name } }"}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_some());

        // Without operation name, should return Boolean(true)
        assert!(matches!(
            result,
            Some(crate::config::GraphQLMatchConfig::Boolean(true))
        ));
    }

    #[test]
    fn test_detect_non_graphql_request() {
        let uri = "/api/users";
        let method = "POST";
        let body = r#"{"name":"John"}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_none());
    }

    #[test]
    fn test_detect_graphql_get_method() {
        let uri = "/graphql";
        let method = "GET";
        let body = r#"{"query":"query GetUser { user { name } }"}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_none()); // GET method should not be detected
    }

    #[test]
    fn test_detect_graphql_with_variables() {
        let uri = "/graphql";
        let method = "POST";
        let body = r#"{"query":"mutation UpdateUser($id: ID!, $name: String!) { updateUser(id: $id, name: $name) { id } }","operationName":"UpdateUser","variables":{"id":"123","name":"Jane"}}"#;

        let result = MockRecorder::detect_graphql_request(uri, method, Some(body));
        assert!(result.is_some());

        if let Some(crate::config::GraphQLMatchConfig::Structured { variables, .. }) = result {
            assert_eq!(variables.get("id").and_then(|v| v.as_str()), Some("123"));
            assert_eq!(variables.get("name").and_then(|v| v.as_str()), Some("Jane"));
        } else {
            panic!("Expected Structured GraphQL config with variables");
        }
    }

    #[tokio::test]
    async fn test_record_graphql_query() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = MockRecorder::new("graphql-test", temp_dir.path());

        let method = Method::POST;
        let uri = "/graphql";
        let graphql_body = r#"{"query":"query GetUser { user(id: \"123\") { name email } }","operationName":"GetUser","variables":{"id":"123"}}"#;

        recorder
            .record(
                &method,
                uri,
                None,
                &HeaderMap::new(),
                Some(&Bytes::from(graphql_body)),
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(r#"{"data":{"user":{"name":"John","email":"john@example.com"}}}"#),
                Duration::from_millis(50),
            )
            .await
            .unwrap();

        // Save and verify the mock collection
        let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        let collection: crate::config::MockCollectionConfig =
            serde_json::from_str(&content).unwrap();

        assert_eq!(collection.mocks.len(), 1);
        let mock = &collection.mocks[0];

        // Verify GraphQL match config was set
        assert!(mock.match_config.is_some());
        let match_config = mock.match_config.as_ref().unwrap();
        assert!(match_config.graphql.is_some());

        if let Some(crate::config::GraphQLMatchConfig::Structured {
            query, variables, ..
        }) = &match_config.graphql
        {
            assert_eq!(query, &Some("GetUser".to_string()));
            assert_eq!(variables.get("id").and_then(|v| v.as_str()), Some("123"));
        } else {
            panic!("Expected Structured GraphQL match config");
        }
    }

    // ===========================================================================
    // Streaming write tests (init_file -> record -> finalize_file)
    // These test the actual streaming path that produces files during recording,
    // as opposed to the batch `save()` path which writes atomically.
    // ===========================================================================

    #[tokio::test]
    async fn test_streaming_single_mock_produces_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("stream-single", temp_dir.path(), RecordingFormat::Json);

        let file_path = recorder.init_file().await.unwrap();

        recorder
            .record(
                &Method::GET,
                "/api/users/1",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(r#"{"id": 1, "name": "Alice"}"#),
                Duration::from_millis(50),
            )
            .await
            .unwrap();

        // Wait for async write to complete
        tokio::time::sleep(Duration::from_millis(50)).await;
        recorder.finalize_file().await.unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        // Must be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|e| panic!("Invalid JSON: {e}\nContent:\n{content}"));

        // Must parse as MockCollectionConfig
        let collection: crate::config::MockCollectionConfig = serde_json::from_str(&content)
            .unwrap_or_else(|e| {
                panic!("Failed to parse as MockCollectionConfig: {e}\nContent:\n{content}")
            });

        assert_eq!(collection.mocks.len(), 1);
        assert!(parsed.get("name").is_some());
        assert!(parsed.get("mocks").is_some());
    }

    #[tokio::test]
    async fn test_streaming_multiple_mocks_produces_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("stream-multi", temp_dir.path(), RecordingFormat::Json);

        let file_path = recorder.init_file().await.unwrap();

        // Record 5 interactions sequentially
        for i in 1..=5 {
            recorder
                .record(
                    &Method::GET,
                    &format!("/api/items/{i}"),
                    None,
                    &HeaderMap::new(),
                    None,
                    StatusCode::OK,
                    &HeaderMap::new(),
                    &Bytes::from(format!(r#"{{"id": {i}, "value": "item-{i}"}}"#)),
                    Duration::from_millis(10 * i as u64),
                )
                .await
                .unwrap();

            // Small delay to let async writes complete
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        recorder.finalize_file().await.unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        // Must parse as MockCollectionConfig
        let collection: crate::config::MockCollectionConfig = serde_json::from_str(&content)
            .unwrap_or_else(|e| {
                panic!("Failed to parse as MockCollectionConfig: {e}\nContent:\n{content}")
            });

        assert_eq!(
            collection.mocks.len(),
            5,
            "Should have 5 mocks in the collection"
        );

        // Verify all mock IDs are unique
        let ids: rustc_hash::FxHashSet<&str> =
            collection.mocks.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), 5, "All mock IDs should be unique");
    }

    #[tokio::test]
    async fn test_streaming_concurrent_records_produce_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let recorder = Arc::new(MockRecorder::with_format(
            "stream-concurrent",
            temp_dir.path(),
            RecordingFormat::Json,
        ));

        let file_path = recorder.init_file().await.unwrap();

        // Fire off 10 concurrent record calls to stress the race condition fix
        let mut handles = Vec::new();
        for i in 0..10 {
            let rec = recorder.clone();
            handles.push(tokio::spawn(async move {
                rec.record(
                    &Method::GET,
                    &format!("/api/concurrent/{i}"),
                    None,
                    &HeaderMap::new(),
                    None,
                    StatusCode::OK,
                    &HeaderMap::new(),
                    &Bytes::from(format!(r#"{{"id": {i}}}"#)),
                    Duration::from_millis(5),
                )
                .await
                .unwrap();
            }));
        }

        // Wait for all record calls to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Wait for all async file writes
        tokio::time::sleep(Duration::from_millis(200)).await;
        recorder.finalize_file().await.unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        // Must be valid JSON - this is the key assertion that was failing before the fix
        let collection: crate::config::MockCollectionConfig = serde_json::from_str(&content)
            .unwrap_or_else(|e| {
                panic!("Invalid JSON from concurrent writes: {e}\nContent:\n{content}")
            });

        assert_eq!(
            collection.mocks.len(),
            10,
            "Should have 10 mocks from concurrent writes"
        );

        // Verify all mock IDs are unique (no collisions from race condition)
        let ids: rustc_hash::FxHashSet<&str> =
            collection.mocks.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids.len(),
            10,
            "All mock IDs should be unique (no race condition collisions)"
        );
    }

    #[tokio::test]
    async fn test_streaming_empty_recording_produces_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("stream-empty", temp_dir.path(), RecordingFormat::Json);

        let file_path = recorder.init_file().await.unwrap();

        // Finalize without recording anything
        recorder.finalize_file().await.unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        // Must be valid JSON even with no mocks
        let collection: crate::config::MockCollectionConfig = serde_json::from_str(&content)
            .unwrap_or_else(|e| {
                panic!("Empty recording should be valid JSON: {e}\nContent:\n{content}")
            });

        assert_eq!(collection.mocks.len(), 0);
        assert!(collection.enabled);
    }

    #[tokio::test]
    async fn test_streaming_with_filtered_requests_valid_json() {
        let temp_dir = TempDir::new().unwrap();
        let filter_options = RecordingFilterOptions {
            capture_success_only: true,
            ..Default::default()
        };
        let recorder = MockRecorder::with_filters(
            "stream-filtered",
            temp_dir.path(),
            RecordingFormat::Json,
            filter_options,
        );

        let file_path = recorder.init_file().await.unwrap();

        // Record a failed request (should be filtered out)
        recorder
            .record(
                &Method::GET,
                "/api/fail",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::INTERNAL_SERVER_ERROR,
                &HeaderMap::new(),
                &Bytes::from(r#"{"error": "fail"}"#),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        // Record a successful request (should be included)
        recorder
            .record(
                &Method::GET,
                "/api/success",
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(r#"{"ok": true}"#),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(50)).await;
        recorder.finalize_file().await.unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        // Must be valid JSON even when first request was filtered
        let collection: crate::config::MockCollectionConfig = serde_json::from_str(&content)
            .unwrap_or_else(|e| {
                panic!("Filtered recording should be valid JSON: {e}\nContent:\n{content}")
            });

        assert_eq!(
            collection.mocks.len(),
            1,
            "Should only have the successful request"
        );
    }

    #[tokio::test]
    async fn test_streaming_yaml_multiple_mocks() {
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("stream-yaml", temp_dir.path(), RecordingFormat::Yaml);

        let file_path = recorder.init_file().await.unwrap();

        for i in 1..=3 {
            recorder
                .record(
                    &Method::GET,
                    &format!("/api/yaml/{i}"),
                    None,
                    &HeaderMap::new(),
                    None,
                    StatusCode::OK,
                    &HeaderMap::new(),
                    &Bytes::from(format!(r#"{{"id": {i}}}"#)),
                    Duration::from_millis(10),
                )
                .await
                .unwrap();

            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        recorder.finalize_file().await.unwrap();

        let content = tokio::fs::read_to_string(&file_path).await.unwrap();

        // Must parse as valid YAML MockCollectionConfig
        let collection: crate::config::MockCollectionConfig = serde_yaml::from_str(&content)
            .unwrap_or_else(|e| panic!("Streaming YAML should be valid: {e}\nContent:\n{content}"));

        assert_eq!(collection.mocks.len(), 3);
    }

    #[tokio::test]
    async fn test_streaming_output_loadable_by_consolidator() {
        // End-to-end: record -> finalize -> load for consolidation
        let temp_dir = TempDir::new().unwrap();
        let recorder =
            MockRecorder::with_format("stream-e2e", temp_dir.path(), RecordingFormat::Json);

        let file_path = recorder.init_file().await.unwrap();

        // Record similar mocks that should be consolidatable
        for i in 1..=4 {
            recorder
                .record(
                    &Method::GET,
                    &format!("/api/users/{}", i * 100),
                    None,
                    &HeaderMap::new(),
                    None,
                    StatusCode::OK,
                    &HeaderMap::new(),
                    &Bytes::from(format!(
                        r#"{{"id": {}, "name": "User {}", "email": "user{}@test.com"}}"#,
                        i * 100,
                        i,
                        i
                    )),
                    Duration::from_millis(20),
                )
                .await
                .unwrap();

            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        recorder.finalize_file().await.unwrap();

        // Verify the file can be loaded by MockCollectionConfig::from_file
        let collection = crate::config::MockCollectionConfig::from_file(&file_path)
            .await
            .unwrap_or_else(|e| panic!("Recording should be loadable by config parser: {e}"));

        assert_eq!(collection.mocks.len(), 4);
        assert!(collection.enabled);
        assert!(collection.name.is_some());
    }
}
