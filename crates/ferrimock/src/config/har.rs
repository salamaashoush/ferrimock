//! HAR (HTTP Archive) file loading and conversion to mock configurations
//!
//! Produces clean, replay-ready mock collections from HAR files.
//! By default, normalizes absolute URLs to relative paths, strips sensitive
//! and infrastructure headers, and optionally extracts large response bodies
//! to separate files.
//!
//! Use the consolidator for further smart pattern detection and optimization.

use crate::Result;
use crate::error::Context;
use har::{Har, Spec, v1_2};
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::{Url, form_urlencoded};

use super::{GraphQLMatchConfig, MatchConfig, MockConfig, ResponseConfig};

/// Parse HAR text robustly under `serde_json/arbitrary_precision`.
///
/// The `har` crate's `Spec` is `#[serde(untagged)]`; when a transitive
/// dependency (the rolldown bundler behind the `scripting` feature)
/// force-enables `arbitrary_precision` workspace-wide, serde's untagged
/// buffering re-emits floats (`time`, `timings.*`) as private-Number
/// maps and direct `from_str::<Har>` fails with
/// "invalid type: map, expected f64". Picking the version manually and
/// deserializing the plain (non-untagged) `Log` struct avoids the
/// buffering entirely.
pub fn parse_har(content: &str) -> Result<Har> {
    if let Ok(har) = serde_json::from_str::<Har>(content) {
        return Ok(har);
    }
    let value: serde_json::Value = serde_json::from_str(content)?;
    let log = value
        .get("log")
        .cloned()
        .ok_or_else(|| crate::mp_err!("HAR file has no `log` object"))?;
    let log: v1_2::Log =
        serde_json::from_value(log).map_err(|e| crate::mp_err!("Failed to parse HAR log: {e}"))?;
    Ok(Har {
        log: Spec::V1_2(log),
    })
}

/// Default body size threshold for extraction (100 KB)
const DEFAULT_BODY_SIZE_THRESHOLD: usize = 100 * 1024;

/// Priority of a mock that pins the whole request line, query included. Ties
/// among these fall back to recording order, which is what replays a repeated
/// request in sequence.
const QUERY_MATCH_PRIORITY: u32 = 200;

/// Priority of a mock recorded from a bare path, which also answers requests
/// that carry a query no recording pinned.
const PATH_MATCH_PRIORITY: u32 = 100;

/// The GraphQL operation a recorded request carries, if it is one.
///
/// Every operation POSTs to the same endpoint, so a mock matched on URL alone
/// answers whichever operation happens to win — silently returning another
/// query's data rather than missing, which is the harder failure to notice.
fn graphql_operation(entry: &v1_2::Entries) -> Option<String> {
    let text = entry.request.post_data.as_ref()?.text.as_ref()?;
    let body: serde_json::Value = serde_json::from_str(text).ok()?;
    // A batch posts an array; those route as a whole and are left to match on
    // URL, since no single operation name describes the request.
    let object = body.as_object()?;
    object.get("query")?.as_str()?;
    let name = object.get("operationName")?.as_str()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Narrow a recorded status to a real HTTP one.
///
/// Returns `None` for anything outside the HTTP range, which is how browser
/// tools spell "this request never completed" — Playwright writes `-1`, and an
/// `as` cast would turn that into 65535.
fn http_status(recorded: i64) -> Option<u16> {
    u16::try_from(recorded)
        .ok()
        .filter(|s| (100..=599).contains(s))
}

/// Determines whether a hostname should be included when loading HAR files.
///
/// Embedders provide their own domain filtering logic (e.g., only allow
/// their API domains). When no filter is set, all domains are included.
///
/// Closures with signature `Fn(&str) -> bool` automatically implement this trait.
pub trait DomainFilter: Send + Sync {
    /// Returns true if the given hostname should be included.
    fn is_allowed(&self, host: &str) -> bool;
}

impl<F> DomainFilter for F
where
    F: Fn(&str) -> bool + Send + Sync,
{
    fn is_allowed(&self, host: &str) -> bool {
        self(host)
    }
}

/// Check if a URL points to a static asset based on file extension
fn is_static_asset(raw_url: &str) -> bool {
    // Strip query string and fragment
    let path = raw_url.split('?').next().unwrap_or(raw_url);
    let path = path.split('#').next().unwrap_or(path);

    // Extract extension from the last path segment
    let last_segment = path.rsplit('/').next().unwrap_or("");
    let ext = match last_segment.rsplit('.').next() {
        Some(e) if e != last_segment => e.to_lowercase(),
        _ => return false,
    };

    matches!(
        ext.as_str(),
        // Scripts & styles
        "js" | "mjs" | "cjs" | "css" | "map" |
    // Images
    "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "avif" | "bmp" |
    // Fonts
    "woff" | "woff2" | "ttf" | "otf" | "eot" |
    // Media
    "mp3" | "mp4" | "webm" | "ogg" | "wav" | "avi" |
    // Documents
    "pdf" |
    // Archives
    "zip" | "gz" | "tar" | "br" |
    // Manifests & metadata
    "json" | "xml" | "manifest" | "webmanifest"
    )
}

/// Check if a query parameter name is sensitive and should be stripped
fn is_sensitive_query_param(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "access_token" | "token" | "api_key" | "apikey" | "secret" | "password" | "session_id"
    )
}

/// A recorded request line reduced to what a mock matches on.
struct UrlMatch {
    /// Everything before the query string.
    base: String,
    /// The query to match as part of the URL, as recorded.
    query: Option<String>,
    /// Query parameters to match one by one, used when the recorded query
    /// cannot be matched whole because a credential was removed from it.
    pinned: FxHashMap<String, String>,
}

impl UrlMatch {
    fn into_request_line(self) -> String {
        match self.query {
            Some(query) => format!("{}?{}", self.base, query),
            None => self.base,
        }
    }
}

fn split_request_line(raw_url: &str) -> (String, Option<String>) {
    let without_fragment = raw_url.split('#').next().unwrap_or(raw_url);
    match without_fragment.split_once('?') {
        Some((base, query)) => (base.to_string(), Some(query.to_string())),
        None => (without_fragment.to_string(), None),
    }
}

/// Restate a query string as one matcher per parameter.
///
/// Returns None when doing so would change what the query means:
/// [`QueryMatcher`](crate::types::QueryMatcher) compares decoded values, holds
/// one value per name, and never sees a parameter written without `=`, so a
/// query with a repeat, a valueless parameter, or a value that is not valid
/// UTF-8 once decoded has no faithful matcher form.
fn pin_query(query: &str) -> Option<FxHashMap<String, String>> {
    let mut pinned = FxHashMap::default();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (name, value) = pair.split_once('=')?;
        let name = urlencoding::decode(name).ok()?.into_owned();
        let value = urlencoding::decode(value).ok()?.into_owned();
        if pinned.insert(name, value).is_some() {
            return None;
        }
    }
    Some(pinned)
}

/// What a mock matches on, as one comparable value.
///
/// Two recordings with the same key are indistinguishable to the matcher: no
/// request can select one over the other.
type MatchKey = (
    Vec<String>,
    Vec<String>,
    Option<String>,
    Vec<(String, String)>,
);

/// The match key of a converted recording, or None when the mock says something
/// this cannot compare — in which case it is left out of every group and never
/// sequenced, which is the safe way to be unsure.
fn match_key(mock: &MockConfig) -> Option<MatchKey> {
    let m = mock.match_config.as_ref()?;
    let graphql = match m.graphql.as_ref() {
        None => None,
        Some(GraphQLMatchConfig::Simple(operation)) => Some(operation.clone()),
        Some(_) => return None,
    };
    let mut pinned: Vec<(String, String)> = m
        .query
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    pinned.sort_unstable();
    Some((m.methods.clone(), m.urls.clone(), graphql, pinned))
}

/// Whether two recordings of the same request were answered the same way.
fn same_answer(one: &MockConfig, other: &MockConfig) -> bool {
    match (one.response_config.as_ref(), other.response_config.as_ref()) {
        (
            Some(ResponseConfig::Structured {
                status: one_status,
                body: one_body,
                file: one_file,
                ..
            }),
            Some(ResponseConfig::Structured {
                status: other_status,
                body: other_body,
                file: other_file,
                ..
            }),
        ) => one_status == other_status && one_body == other_body && one_file == other_file,
        (None, None) => true,
        _ => false,
    }
}

/// Replay a request that was recorded several times in the order it was
/// answered, by retiring each mock as it is used.
///
/// A trace records a conversation, not a table: the same call can be answered
/// differently as the session moves on — a list before and after an upload, a
/// job that is pending and then done. Left alone, every one of those requests
/// is answered by whichever recording came first and the rest are dead weight.
/// Chaining them with `once` hands them back in the order they were recorded,
/// and the final one keeps answering afterwards.
///
/// Recordings that were answered identically are left alone: sequencing them
/// would retire mocks to no visible effect.
fn sequence_repeated_requests(mocks: &mut [MockConfig]) {
    let mut groups: std::collections::HashMap<MatchKey, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, mock) in mocks.iter().enumerate() {
        if let Some(key) = match_key(mock) {
            groups.entry(key).or_default().push(index);
        }
    }

    for indices in groups.values() {
        let Some((&last, rest)) = indices.split_last() else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let answered_alike = {
            let Some(final_answer) = mocks.get(last) else {
                continue;
            };
            rest.iter()
                .filter_map(|&i| mocks.get(i))
                .all(|earlier| same_answer(earlier, final_answer))
        };
        if answered_alike {
            continue;
        }
        for &i in rest {
            if let Some(mock) = mocks.get_mut(i) {
                mock.once = true;
            }
        }
    }
}

/// Whether a raw `name=value` pair names a sensitive parameter.
///
/// The name is decoded before the test so an encoded spelling cannot smuggle a
/// credential past it; the pair itself stays untouched.
fn is_sensitive_pair(pair: &str) -> bool {
    let raw_name = pair.split('=').next().unwrap_or(pair);
    match form_urlencoded::parse(raw_name.as_bytes()).next() {
        Some((name, _)) => is_sensitive_query_param(&name),
        None => false,
    }
}

/// Check if a response body should be extracted to a file rather than inlined
fn should_use_file_body(body: &str, content_type: Option<&str>, threshold: usize) -> bool {
    if body.len() > threshold {
        return true;
    }

    if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.starts_with("image/")
            || ct_lower.starts_with("video/")
            || ct_lower.starts_with("audio/")
            || ct_lower.starts_with("font/")
            || ct_lower.contains("application/pdf")
            || ct_lower.contains("application/zip")
            || ct_lower.contains("application/octet-stream")
            || ct_lower.contains("text/html")
            || ct_lower.contains("text/css")
        {
            return true;
        }
    }

    false
}

/// Options for loading HAR files
#[derive(Clone)]
pub struct HarLoadOptions {
    /// Exclude OPTIONS preflight requests
    pub exclude_preflight: bool,
    /// Exclude redirect responses (3xx)
    pub exclude_redirects: bool,
    /// Strip browser-specific headers
    pub strip_browser_headers: bool,
    /// Convert absolute URLs to relative paths (default: true)
    pub normalize_urls: bool,
    /// Domain filter: only include entries from allowed domains.
    /// When None, all domains are included.
    pub domain_filter: Option<Arc<dyn DomainFilter>>,
    /// Skip static asset entries like .js, .css, .png (default: true)
    pub exclude_static_assets: bool,
    /// Remove Authorization, Cookie, Set-Cookie headers (default: true)
    pub strip_sensitive_headers: bool,
    /// Remove date, server, x-envoy-*, alt-svc, etc. (default: true)
    pub strip_infrastructure_headers: bool,
    /// Remove access_token, api_key from query strings (default: true)
    pub strip_sensitive_query_params: bool,
    /// Replay a request recorded several times with differing answers in the
    /// order it was answered, rather than always giving back the first
    /// (default: true)
    pub sequence_repeated_requests: bool,
    /// Directory for extracted body files (None = inline all bodies)
    pub body_output_dir: Option<PathBuf>,
    /// Size threshold for body extraction (default: 100KB)
    pub body_size_threshold: usize,
}

impl Default for HarLoadOptions {
    fn default() -> Self {
        Self {
            exclude_preflight: true,
            exclude_redirects: true,
            strip_browser_headers: true,
            normalize_urls: true,
            domain_filter: None,
            exclude_static_assets: true,
            strip_sensitive_headers: true,
            strip_infrastructure_headers: true,
            strip_sensitive_query_params: true,
            sequence_repeated_requests: true,
            body_output_dir: None,
            body_size_threshold: DEFAULT_BODY_SIZE_THRESHOLD,
        }
    }
}

impl std::fmt::Debug for HarLoadOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarLoadOptions")
            .field("exclude_preflight", &self.exclude_preflight)
            .field("exclude_redirects", &self.exclude_redirects)
            .field("strip_browser_headers", &self.strip_browser_headers)
            .field("normalize_urls", &self.normalize_urls)
            .field("domain_filter", &self.domain_filter.is_some())
            .field("exclude_static_assets", &self.exclude_static_assets)
            .field("strip_sensitive_headers", &self.strip_sensitive_headers)
            .field(
                "strip_infrastructure_headers",
                &self.strip_infrastructure_headers,
            )
            .field(
                "strip_sensitive_query_params",
                &self.strip_sensitive_query_params,
            )
            .field(
                "sequence_repeated_requests",
                &self.sequence_repeated_requests,
            )
            .field("body_output_dir", &self.body_output_dir)
            .field("body_size_threshold", &self.body_size_threshold)
            .finish()
    }
}

/// HAR file loader
pub struct HarLoader {
    options: HarLoadOptions,
}

impl HarLoader {
    /// Create a new HAR loader with default options
    pub fn new() -> Self {
        Self {
            options: HarLoadOptions::default(),
        }
    }

    /// Create a new HAR loader with custom options
    pub fn with_options(options: HarLoadOptions) -> Self {
        Self { options }
    }

    /// Load HAR file and convert to mock definitions
    pub async fn load_from_file(&self, path: impl AsRef<Path>) -> Result<Vec<MockConfig>> {
        let content = tokio::fs::read_to_string(path.as_ref()).await?;
        let har = parse_har(&content)?;

        let mut mocks = self.convert_har_to_mocks(har).await?;
        // Chrome's `_webSocketMessages` extension is not in the har
        // crate's schema — recover it from the raw JSON.
        mocks.extend(self.convert_websocket_entries(&content)?);
        Ok(mocks)
    }

    /// Convert HAR structure to mock definitions (simple 1:1 conversion)
    pub async fn convert_har_to_mocks(&self, har: Har) -> Result<Vec<MockConfig>> {
        let entries = match &har.log {
            Spec::V1_2(log) => &log.entries,
            Spec::V1_3(_) => return Err(crate::mp_err!("Unsupported HAR version")),
        };

        // Create bodies directory if body extraction is enabled
        if let Some(ref output_dir) = self.options.body_output_dir {
            let bodies_dir = output_dir.join("bodies");
            tokio::fs::create_dir_all(&bodies_dir)
                .await
                .context("Failed to create bodies directory")?;
        }

        let mut mocks = Vec::new();

        for (idx, entry) in entries.iter().enumerate() {
            // Apply filtering options
            if self.should_skip_entry(entry) {
                continue;
            }

            // Convert entry to mock - returns None if domain filtered
            if let Some(mock) = self.convert_entry_to_mock(entry, idx).await? {
                mocks.push(mock);
            }
        }

        if self.options.sequence_repeated_requests {
            sequence_repeated_requests(&mut mocks);
        }

        Ok(mocks)
    }

    /// Check if an entry should be skipped based on filtering options
    fn should_skip_entry(&self, entry: &v1_2::Entries) -> bool {
        // Skip OPTIONS preflight requests
        if self.options.exclude_preflight && entry.request.method == "OPTIONS" {
            return true;
        }

        // An entry that never received a response cannot be replayed. Browser
        // tools record an aborted or cancelled request with a status outside
        // the HTTP range (Playwright writes `-1`), and carrying one through
        // produces a mock no server would accept — which, being a load-time
        // error, takes every other mock in the file down with it.
        let Some(status) = http_status(entry.response.status) else {
            return true;
        };

        // Skip redirects
        if self.options.exclude_redirects && (300..400).contains(&status) {
            return true;
        }

        // Skip static assets
        if self.options.exclude_static_assets && is_static_asset(&entry.request.url) {
            return true;
        }

        false
    }

    /// Normalize a URL: convert absolute to relative, strip sensitive query params.
    /// Returns None if the domain is filtered out.
    fn normalize_url(&self, raw_url: &str) -> Option<String> {
        Some(self.url_match(raw_url)?.into_request_line())
    }

    /// Reduce a recorded request line to what the mock should match on.
    ///
    /// Returns None if the domain is filtered out.
    fn url_match(&self, raw_url: &str) -> Option<UrlMatch> {
        let (base, raw_query) = if let Ok(parsed) = Url::parse(raw_url)
            && let Some(host) = parsed.host_str()
        {
            if let Some(ref filter) = self.options.domain_filter
                && !filter.is_allowed(host)
            {
                return None;
            }
            if self.options.normalize_urls {
                // `Url::path` and `Url::query` both hand back the string as it
                // was written, which is what keeps the pattern comparable with
                // the request line a server later receives.
                (
                    parsed.path().to_string(),
                    parsed.query().map(str::to_string),
                )
            } else {
                split_request_line(raw_url)
            }
        } else {
            split_request_line(raw_url)
        };

        let Some(raw_query) = raw_query else {
            return Some(UrlMatch {
                base,
                query: None,
                pinned: FxHashMap::default(),
            });
        };

        let kept = self.filter_query_string(&raw_query);
        // Nothing was dropped, so the recorded request line still describes
        // itself and matching it whole is exact.
        if kept == raw_query {
            return Some(UrlMatch {
                base,
                query: (!raw_query.is_empty()).then_some(raw_query),
                pinned: FxHashMap::default(),
            });
        }

        // A credential was dropped from the middle of the query. Keeping the
        // remainder in the pattern would demand a request whose query is
        // exactly the redacted one — which the recorded request, still
        // carrying its token, is not. Pinning the survivors individually says
        // what the recording actually established and ignores the credential.
        match pin_query(&kept) {
            Some(pinned) => Some(UrlMatch {
                base,
                query: None,
                pinned,
            }),
            // The remainder cannot be stated as matchers without changing its
            // meaning, so the redacted request line stays — narrower than the
            // recording, never wider.
            None => Some(UrlMatch {
                base,
                query: (!kept.is_empty()).then(|| kept.into_owned()),
                pinned: FxHashMap::default(),
            }),
        }
    }

    /// Drop sensitive parameters from a raw query string, leaving every
    /// surviving pair byte-for-byte as it was recorded.
    ///
    /// Reassembling the query from a decoded pair list corrupts it: a recorded
    /// `cursor=eyJwIjoxfQ%3D%3D` comes back as `cursor=eyJwIjoxfQ==`, a `%2C`
    /// separator becomes a literal comma, and a valueless `marker` grows an
    /// `=`. The mock then fails to match the very request it was recorded from.
    /// Whole pairs are removed here, never rewritten.
    fn filter_query_string<'a>(&self, raw_query: &'a str) -> Cow<'a, str> {
        if !self.options.strip_sensitive_query_params {
            return Cow::Borrowed(raw_query);
        }
        if !raw_query.split('&').any(is_sensitive_pair) {
            return Cow::Borrowed(raw_query);
        }
        let kept: Vec<&str> = raw_query
            .split('&')
            .filter(|pair| !is_sensitive_pair(pair))
            .collect();
        Cow::Owned(kept.join("&"))
    }

    /// Convert a HAR entry to a mock definition. Returns None if the entry's
    /// domain is filtered out.
    async fn convert_entry_to_mock(
        &self,
        entry: &v1_2::Entries,
        index: usize,
    ) -> Result<Option<MockConfig>> {
        let mock_id = format!("har-entry-{}", index + 1);

        // WebSocket entries (ws:// or wss:// URLs, Chrome's recording
        // scheme) convert through the `_webSocketMessages` pass instead.
        if entry.request.url.starts_with("ws://") || entry.request.url.starts_with("wss://") {
            return Ok(None);
        }

        // Reduce the request line to a matcher (None if domain is filtered)
        let Some(url_match) = self.url_match(&entry.request.url) else {
            return Ok(None);
        };
        let pinned_query = url_match.pinned.clone();
        // An exact pattern that names no query still matches a request that
        // carries one, so a recording of the bare endpoint would answer every
        // parameterised call to it — and, being recorded earlier, would win the
        // tie against the mocks recorded for those calls. Ranking by how much
        // of the request line a mock pins puts the specific ones first.
        let priority = if url_match.query.is_some() || !url_match.pinned.is_empty() {
            QUERY_MATCH_PRIORITY
        } else {
            PATH_MATCH_PRIORITY
        };
        let url_pattern = format!("exact:{}", url_match.into_request_line());

        // Convert headers, stripping based on options
        let headers: FxHashMap<String, String> = entry
            .response
            .headers
            .iter()
            .filter(|h| !self.should_strip_header(&h.name))
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect();

        // Extract response body
        let body = entry.response.content.text.clone().unwrap_or_default();
        let content_type = entry.response.content.mime_type.as_deref();

        // Determine if body should be extracted to a file
        let (body_value, file_value) = if self.options.body_output_dir.is_some()
            && should_use_file_body(&body, content_type, self.options.body_size_threshold)
        {
            let file_path = format!("bodies/{mock_id}.body");
            if let Some(ref output_dir) = self.options.body_output_dir {
                let full_path = output_dir.join(&file_path);
                tokio::fs::write(&full_path, &body).await.with_context(|| {
                    format!("Failed to write body file: {}", full_path.display())
                })?;
            }
            (None, Some(file_path))
        } else {
            (Some(body), None)
        };

        // Calculate delay from timings (use wait time)
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let delay_ms = entry.timings.wait.max(0.0) as u64;

        Ok(Some(MockConfig {
            id: mock_id.into(),
            description: None,
            priority,
            enabled: true,
            once: false,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                method: None,
                methods: vec![entry.request.method.clone()],
                url: None,
                urls: vec![url_pattern],
                headers: FxHashMap::default(),
                query: pinned_query,
                body: FxHashMap::default(),
                graphql: graphql_operation(entry).map(GraphQLMatchConfig::Simple),
            }),
            request: None,
            response_config: Some(ResponseConfig::Structured {
                // Entries without a usable status are filtered out by
                // `should_skip_entry`, so this only ever narrows a real one.
                status: http_status(entry.response.status),
                headers,
                body: body_value,
                template: None,
                file: file_value,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            patch: None,
            delay: if delay_ms > 0 {
                Some(format!("{delay_ms}ms"))
            } else {
                None
            },
            network_error: None,
            sse: None,
            ws: None,
        }))
    }

    /// Convert Chrome DevTools `_webSocketMessages` entries into
    /// declarative `ws` mocks.
    ///
    /// Frames the recorded server sent (DevTools direction `receive`)
    /// become the mock's sends; frames the client sent (`send`) become
    /// `on_message` exact-match rules replying with the server frames
    /// that followed them — when that pairing is unambiguous (no client
    /// payload recurs with a different reply). Ambiguous recordings fold
    /// every server frame into the `on_connect` sequence with the
    /// recorded inter-frame delays instead.
    fn convert_websocket_entries(&self, content: &str) -> Result<Vec<MockConfig>> {
        let value: serde_json::Value = serde_json::from_str(content)?;
        let Some(entries) = value
            .get("log")
            .and_then(|log| log.get("entries"))
            .and_then(serde_json::Value::as_array)
        else {
            return Ok(Vec::new());
        };

        let mut mocks = Vec::new();
        for entry in entries {
            let Some(messages) = entry
                .get("_webSocketMessages")
                .and_then(serde_json::Value::as_array)
            else {
                continue;
            };
            let Some(raw_url) = entry
                .get("request")
                .and_then(|r| r.get("url"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            // normalize_url handles the domain filter and relative-path
            // conversion; ws schemes parse like http ones.
            let Some(normalized_url) = self.normalize_url(raw_url) else {
                continue;
            };

            let frames = parse_ws_messages(messages);
            if frames.is_empty() {
                continue;
            }

            let ws = build_ws_config(&frames);
            let index = mocks.len() + 1;
            mocks.push(MockConfig {
                id: format!("har-ws-{index}").into(),
                description: None,
                priority: 100,
                enabled: true,
                once: false,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    method: None,
                    methods: vec!["GET".to_string()],
                    url: None,
                    urls: vec![format!("exact:{normalized_url}")],
                    headers: FxHashMap::default(),
                    query: FxHashMap::default(),
                    body: FxHashMap::default(),
                    graphql: None,
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
                network_error: None,
                sse: None,
                ws: Some(ws),
            });
        }
        Ok(mocks)
    }

    /// Check if a header should be stripped
    fn should_strip_header(&self, name: &str) -> bool {
        let lower = name.to_lowercase();

        // Sensitive headers (auth, cookies)
        if self.options.strip_sensitive_headers
            && matches!(
                lower.as_str(),
                "authorization"
                    | "cookie"
                    | "set-cookie"
                    | "x-auth-token"
                    | "x-csrf-token"
                    | "proxy-authorization"
            )
        {
            return true;
        }

        // Browser-specific headers
        if self.options.strip_browser_headers
            && matches!(
                lower.as_str(),
                "user-agent"
                    | "accept-language"
                    | "accept-encoding"
                    | "cache-control"
                    | "connection"
                    | "upgrade-insecure-requests"
                    | "sec-fetch-site"
                    | "sec-fetch-mode"
                    | "sec-fetch-dest"
                    | "sec-ch-ua"
                    | "sec-ch-ua-mobile"
                    | "sec-ch-ua-platform"
                    | "referer"
                    | "origin"
            )
        {
            return true;
        }

        // Infrastructure headers (server, proxy, CDN)
        if self.options.strip_infrastructure_headers {
            if matches!(
                lower.as_str(),
                "date"
                    | "age"
                    | "server"
                    | "via"
                    | "server-timing"
                    | "alt-svc"
                    | "x-cache"
                    | "strict-transport-security"
                    | "expect-ct"
                    | "report-to"
                    | "nel"
            ) {
                return true;
            }

            // Prefix-based infrastructure headers
            if lower.starts_with("x-envoy-")
                || lower.starts_with("x-gateway-")
                || lower.starts_with("x-amz-")
                || lower.starts_with("x-cdn-")
                || lower.starts_with("x-forwarded-")
            {
                return true;
            }
        }

        false
    }
}

impl Default for HarLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// One recorded WebSocket frame, mock-perspective: `outbound` frames are
/// the ones the mock replays (the recorded server sent them).
struct HarWsFrame {
    outbound: bool,
    /// Epoch seconds (Chrome's `_webSocketMessages[].time`).
    time: f64,
    payload: HarWsPayload,
}

#[derive(PartialEq, Eq, Clone)]
enum HarWsPayload {
    Text(String),
    /// Kept base64-encoded, as recorded (opcode 2).
    Binary(String),
}

fn parse_ws_messages(messages: &[serde_json::Value]) -> Vec<HarWsFrame> {
    let mut frames = Vec::new();
    for message in messages {
        let Some(direction) = message.get("type").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let outbound = match direction {
            "receive" => true,
            "send" => false,
            _ => continue,
        };
        let time = message
            .get("time")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let data = message
            .get("data")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let payload = match message.get("opcode").and_then(serde_json::Value::as_i64) {
            Some(2) => HarWsPayload::Binary(data),
            _ => HarWsPayload::Text(data),
        };
        frames.push(HarWsFrame {
            outbound,
            time,
            payload,
        });
    }
    frames.sort_by(|a, b| a.time.total_cmp(&b.time));
    frames
}

fn ws_delay_ms(from: f64, to: f64) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let ms = ((to - from) * 1000.0).round().max(0.0) as u64;
    ms
}

fn send_action(payload: &HarWsPayload) -> super::streaming::WsActionConfig {
    match payload {
        HarWsPayload::Text(text) => super::streaming::WsActionConfig::Send {
            send: serde_json::Value::String(text.clone()),
        },
        HarWsPayload::Binary(encoded) => super::streaming::WsActionConfig::SendBinary {
            send_binary: encoded.clone(),
        },
    }
}

/// Replay a run of outbound frames as send actions with the recorded
/// inter-frame delays (the first frame plays immediately).
fn replay_actions(frames: &[&HarWsFrame]) -> Vec<super::streaming::WsActionConfig> {
    let mut actions = Vec::new();
    let mut previous: Option<f64> = None;
    for frame in frames {
        if let Some(prev) = previous {
            let delay = ws_delay_ms(prev, frame.time);
            if delay > 0 {
                actions.push(super::streaming::WsActionConfig::Delay {
                    delay: format!("{delay}ms"),
                });
            }
        }
        actions.push(send_action(&frame.payload));
        previous = Some(frame.time);
    }
    actions
}

fn rule_match(payload: &HarWsPayload) -> super::streaming::WsMatchConfig {
    let (exact, binary) = match payload {
        HarWsPayload::Text(text) => (Some(text.clone()), None),
        HarWsPayload::Binary(encoded) => (None, Some(encoded.clone())),
    };
    super::streaming::WsMatchConfig {
        exact,
        regex: None,
        json_path: None,
        equals: None,
        binary_base64: binary,
        binary_prefix_base64: None,
        any: None,
    }
}

fn build_ws_config(frames: &[HarWsFrame]) -> super::streaming::WsConfig {
    use super::streaming::{WsActionConfig, WsConfig, WsRuleConfig};

    let client_indexes: Vec<usize> = frames
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.outbound)
        .map(|(i, _)| i)
        .collect();

    let fold_everything = |frames: &[HarWsFrame]| -> WsConfig {
        let outbound: Vec<&HarWsFrame> = frames.iter().filter(|f| f.outbound).collect();
        WsConfig {
            subprotocol: None,
            echo: None,
            upstream: None,
            on_connect: replay_actions(&outbound),
            on_message: Vec::new(),
        }
    };

    let Some(&first_client) = client_indexes.first() else {
        return fold_everything(frames);
    };

    // Server frames before the first client message replay on connect;
    // each client message maps to the server frames that followed it.
    let preamble: Vec<&HarWsFrame> = frames.iter().take(first_client).collect();

    let mut pairs: Vec<(&HarWsFrame, Vec<&HarWsFrame>)> = Vec::new();
    for (position, &index) in client_indexes.iter().enumerate() {
        let end = client_indexes
            .get(position + 1)
            .copied()
            .unwrap_or(frames.len());
        let Some(client) = frames.get(index) else {
            continue;
        };
        let replies: Vec<&HarWsFrame> = frames
            .iter()
            .take(end)
            .skip(index + 1)
            .filter(|f| f.outbound)
            .collect();
        pairs.push((client, replies));
    }

    // Ambiguity check: the same client payload recurring with a
    // different reply sequence cannot become an exact-match rule.
    for (i, (frame_a, replies_a)) in pairs.iter().enumerate() {
        for (frame_b, replies_b) in pairs.iter().skip(i + 1) {
            if frame_a.payload == frame_b.payload {
                let payloads_a: Vec<&HarWsPayload> = replies_a.iter().map(|f| &f.payload).collect();
                let payloads_b: Vec<&HarWsPayload> = replies_b.iter().map(|f| &f.payload).collect();
                if payloads_a != payloads_b {
                    return fold_everything(frames);
                }
            }
        }
    }

    let mut on_message: Vec<WsRuleConfig> = Vec::new();
    let mut seen: Vec<&HarWsPayload> = Vec::new();
    for (client, replies) in &pairs {
        if replies.is_empty() || seen.contains(&&client.payload) {
            continue;
        }
        seen.push(&client.payload);

        let mut actions: Vec<WsActionConfig> = Vec::new();
        let mut previous = client.time;
        for reply in replies {
            let delay = ws_delay_ms(previous, reply.time);
            if delay > 0 {
                actions.push(WsActionConfig::Delay {
                    delay: format!("{delay}ms"),
                });
            }
            actions.push(send_action(&reply.payload));
            previous = reply.time;
        }
        on_message.push(WsRuleConfig {
            match_config: rule_match(&client.payload),
            actions,
        });
    }

    WsConfig {
        subprotocol: None,
        echo: None,
        upstream: None,
        on_connect: replay_actions(&preamble),
        on_message,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- Helper --

    fn create_test_entry(method: &str, url: &str, status: i64) -> v1_2::Entries {
        create_test_entry_with_headers(method, url, status, vec![], None)
    }

    fn create_test_entry_with_headers(
        method: &str,
        url: &str,
        status: i64,
        response_headers: Vec<(&str, &str)>,
        body: Option<&str>,
    ) -> v1_2::Entries {
        v1_2::Entries {
            pageref: None,
            started_date_time: "2025-10-07T12:00:00.000Z".to_string(),
            time: 50.0,
            request: v1_2::Request {
                method: method.to_string(),
                url: url.to_string(),
                http_version: "HTTP/1.1".to_string(),
                cookies: vec![],
                headers: vec![],
                query_string: vec![],
                post_data: None,
                headers_size: -1,
                body_size: 0,
                comment: None,
            },
            response: v1_2::Response {
                status,
                status_text: "OK".to_string(),
                http_version: "HTTP/1.1".to_string(),
                cookies: vec![],
                headers: response_headers
                    .into_iter()
                    .map(|(n, v)| v1_2::Headers {
                        name: n.to_string(),
                        value: v.to_string(),
                        comment: None,
                    })
                    .collect(),
                content: v1_2::Content {
                    #[allow(clippy::cast_possible_wrap)]
                    size: body.map_or(0, |b| b.len() as i64),
                    compression: None,
                    mime_type: Some("application/json".to_string()),
                    text: Some(body.unwrap_or("{}").to_string()),
                    encoding: None,
                    comment: None,
                },
                redirect_url: Some(String::new()),
                headers_size: -1,
                body_size: 0,
                comment: None,
            },
            cache: v1_2::Cache {
                before_request: None,
                after_request: None,
            },
            timings: v1_2::Timings {
                blocked: None,
                dns: None,
                connect: None,
                send: 0.0,
                wait: 50.0,
                receive: 0.0,
                ssl: None,
                comment: None,
            },
            server_ip_address: None,
            connection: None,
            comment: None,
        }
    }

    fn make_har(entries: Vec<v1_2::Entries>) -> Har {
        Har {
            log: Spec::V1_2(v1_2::Log {
                creator: v1_2::Creator {
                    name: "test".to_string(),
                    version: "1.0".to_string(),
                    comment: None,
                },
                browser: None,
                pages: None,
                entries,
                comment: None,
            }),
        }
    }

    // -- is_static_asset --

    #[test]
    fn test_is_static_asset() {
        assert!(is_static_asset("https://cdn.example.com/app.js"));
        assert!(is_static_asset("https://cdn.example.com/style.css"));
        assert!(is_static_asset("https://cdn.example.com/logo.png"));
        assert!(is_static_asset("https://cdn.example.com/font.woff2"));
        assert!(is_static_asset("/assets/bundle.js?v=123"));
        assert!(is_static_asset("/images/icon.svg#fragment"));
    }

    #[test]
    fn test_is_not_static_asset() {
        assert!(!is_static_asset("https://api.example.com/v2/users/me"));
        assert!(!is_static_asset("/v2/files/123"));
        assert!(!is_static_asset(
            "https://api.example.com/v2/folders/0/items"
        ));
    }

    // -- is_sensitive_query_param --

    #[test]
    fn test_is_sensitive_query_param() {
        assert!(is_sensitive_query_param("access_token"));
        assert!(is_sensitive_query_param("token"));
        assert!(is_sensitive_query_param("api_key"));
        assert!(is_sensitive_query_param("ACCESS_TOKEN"));
        assert!(!is_sensitive_query_param("fields"));
        assert!(!is_sensitive_query_param("limit"));
        assert!(!is_sensitive_query_param("offset"));
    }

    // -- should_use_file_body --

    #[test]
    fn test_should_use_file_body_large() {
        let large_body = "x".repeat(200 * 1024);
        assert!(should_use_file_body(
            &large_body,
            Some("application/json"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
    }

    #[test]
    fn test_should_use_file_body_small() {
        assert!(!should_use_file_body(
            "{}",
            Some("application/json"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
    }

    #[test]
    fn test_should_use_file_body_binary_content_type() {
        assert!(should_use_file_body(
            "small",
            Some("image/png"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
        assert!(should_use_file_body(
            "small",
            Some("application/pdf"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
        assert!(should_use_file_body(
            "small",
            Some("text/html"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
    }

    // -- URL normalization --

    #[test]
    fn test_normalize_url_absolute() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("https://api.example.com/v2/users/me"),
            Some("/v2/users/me".to_string())
        );
    }

    #[test]
    fn test_normalize_url_preserves_query() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("https://api.example.com/v2/items?fields=name,id&limit=100"),
            Some("/v2/items?fields=name,id&limit=100".to_string())
        );
    }

    /// Redacting a credential leaves a query the recorded request does not
    /// have, so the survivors move out of the URL and into matchers, which
    /// ignore the token instead of demanding its absence.
    #[test]
    fn test_normalize_url_strips_access_token() {
        let loader = HarLoader::new();
        let matched = loader
            .url_match("https://api.example.com/v2/users/me?access_token=SECRET&fields=name")
            .expect("not filtered");

        assert_eq!(
            matched.pinned.get("fields").map(String::as_str),
            Some("name")
        );
        assert!(!matched.pinned.contains_key("access_token"));
        assert_eq!(matched.into_request_line(), "/v2/users/me");
    }

    #[test]
    fn test_normalize_url_domain_filter() {
        let loader = HarLoader::with_options(HarLoadOptions {
            domain_filter: Some(Arc::new(|host: &str| host.ends_with(".example.com"))),
            ..Default::default()
        });
        assert_eq!(
            loader.normalize_url("https://api.example.com/v2/users"),
            Some("/v2/users".to_string())
        );
        assert_eq!(
            loader.normalize_url("https://www.google.com/analytics"),
            None
        );
    }

    #[test]
    fn test_normalize_url_already_relative() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("/v2/users/me"),
            Some("/v2/users/me".to_string())
        );
    }

    #[test]
    fn test_normalize_url_disabled() {
        let loader = HarLoader::with_options(HarLoadOptions {
            normalize_urls: false,
            ..Default::default()
        });
        let result =
            loader.normalize_url("https://api.example.com/v2/users/me?access_token=SECRET");
        assert_eq!(
            result,
            Some("https://api.example.com/v2/users/me".to_string())
        );
    }

    // -- should_skip_entry --

    #[test]
    fn test_skip_static_assets() {
        let loader = HarLoader::new();
        let entry = create_test_entry("GET", "https://cdn.example.com/app.js", 200);
        assert!(loader.should_skip_entry(&entry));
    }

    #[test]
    fn test_keep_api_calls() {
        let loader = HarLoader::new();
        let entry = create_test_entry("GET", "https://api.example.com/v2/users/me", 200);
        assert!(!loader.should_skip_entry(&entry));
    }

    // -- Header stripping --

    #[test]
    fn test_strip_sensitive_headers() {
        let loader = HarLoader::new();
        assert!(loader.should_strip_header("Authorization"));
        assert!(loader.should_strip_header("Cookie"));
        assert!(loader.should_strip_header("Set-Cookie"));
        assert!(loader.should_strip_header("x-csrf-token"));
    }

    #[test]
    fn test_strip_infrastructure_headers() {
        let loader = HarLoader::new();
        assert!(loader.should_strip_header("date"));
        assert!(loader.should_strip_header("server"));
        assert!(loader.should_strip_header("x-envoy-upstream-service-time"));
        assert!(loader.should_strip_header("alt-svc"));
        assert!(loader.should_strip_header("x-amz-request-id"));
        assert!(loader.should_strip_header("x-forwarded-for"));
    }

    #[test]
    fn test_keep_content_headers() {
        let loader = HarLoader::new();
        assert!(!loader.should_strip_header("content-type"));
        assert!(!loader.should_strip_header("content-length"));
        assert!(!loader.should_strip_header("x-request-id"));
        assert!(!loader.should_strip_header("x-correlation-id"));
    }

    #[test]
    fn test_keep_sensitive_headers_when_disabled() {
        let loader = HarLoader::with_options(HarLoadOptions {
            strip_sensitive_headers: false,
            ..Default::default()
        });
        assert!(!loader.should_strip_header("Authorization"));
        assert!(!loader.should_strip_header("Cookie"));
    }

    // -- Domain filtering integration --

    #[tokio::test]
    async fn test_filter_domains_with_filter() {
        let har = make_har(vec![
            create_test_entry("GET", "https://api.example.com/v2/users/me", 200),
            create_test_entry("GET", "https://www.google.com/analytics", 200),
            create_test_entry("POST", "https://upload.example.com/v2/files/content", 201),
            create_test_entry("GET", "https://cdn.jsdelivr.net/npm/react", 200),
        ]);

        let loader = HarLoader::with_options(HarLoadOptions {
            domain_filter: Some(Arc::new(|host: &str| host.ends_with(".example.com"))),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        // Only the 2 example.com entries should remain
        assert_eq!(mocks.len(), 2);
    }

    #[tokio::test]
    async fn test_no_domain_filter() {
        let har = make_har(vec![
            create_test_entry("GET", "https://api.example.com/v2/users/me", 200),
            create_test_entry("GET", "https://www.google.com/analytics", 200),
        ]);

        // Default: no domain filter, all domains included
        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 2);
    }

    // -- URL normalization integration --

    #[tokio::test]
    async fn test_urls_normalized_to_relative() {
        let har = make_har(vec![create_test_entry(
            "GET",
            "https://api.example.com/v2/users/me",
            200,
        )]);

        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let mc = mocks[0].match_config.as_ref().unwrap();
        assert_eq!(mc.urls[0], "exact:/v2/users/me");
    }

    #[tokio::test]
    async fn test_absolute_urls_preserved_when_disabled() {
        let har = make_har(vec![create_test_entry(
            "GET",
            "https://api.example.com/v2/users/me",
            200,
        )]);

        let loader = HarLoader::with_options(HarLoadOptions {
            normalize_urls: false,
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let mc = mocks[0].match_config.as_ref().unwrap();
        assert_eq!(mc.urls[0], "exact:https://api.example.com/v2/users/me");
    }

    // -- Body extraction --

    #[tokio::test]
    async fn test_body_extraction_large() {
        let temp_dir = TempDir::new().unwrap();
        let large_body = "x".repeat(200 * 1024);

        let mut entry = create_test_entry("GET", "https://api.example.com/v2/files/123", 200);
        entry.response.content.text = Some(large_body.clone());

        let har = make_har(vec![entry]);

        let loader = HarLoader::with_options(HarLoadOptions {
            body_output_dir: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let rc = mocks[0].response_config.as_ref().unwrap();
        // Body should be in file, not inline
        assert_eq!(rc.file_ref(), Some(&"bodies/har-entry-1.body".to_string()));
        assert!(rc.body().is_none());

        // Verify file was written
        let file_content =
            tokio::fs::read_to_string(temp_dir.path().join("bodies/har-entry-1.body"))
                .await
                .unwrap();
        assert_eq!(file_content, large_body);
    }

    #[tokio::test]
    async fn test_body_inline_small() {
        let temp_dir = TempDir::new().unwrap();

        let har = make_har(vec![create_test_entry(
            "GET",
            "https://api.example.com/v2/users/me",
            200,
        )]);

        let loader = HarLoader::with_options(HarLoadOptions {
            body_output_dir: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let rc = mocks[0].response_config.as_ref().unwrap();
        // Small body should be inline
        assert!(rc.body().is_some());
        assert!(rc.file_ref().is_none());
    }

    // -- End-to-end --

    #[tokio::test]
    async fn test_end_to_end_clean_conversion() {
        let har = make_har(vec![
            // API call - should be kept with relative URL
            create_test_entry_with_headers(
                "GET",
                "https://api.example.com/v2/users/me?access_token=SECRET_TOKEN&fields=name",
                200,
                vec![
                    ("content-type", "application/json"),
                    ("Authorization", "Bearer tok_123"),
                    ("x-envoy-upstream-service-time", "42"),
                    ("date", "Mon, 01 Jan 2024 00:00:00 GMT"),
                    ("x-request-id", "abc123"),
                ],
                Some(r#"{"id":"123","name":"Test"}"#),
            ),
            // Static asset - should be filtered
            create_test_entry("GET", "https://cdn.example.com/static/app.js", 200),
            // Non-allowed domain - should be filtered
            create_test_entry("GET", "https://www.google-analytics.com/collect", 200),
            // OPTIONS preflight - should be filtered
            create_test_entry("OPTIONS", "https://api.example.com/v2/files", 204),
        ]);

        let loader = HarLoader::with_options(HarLoadOptions {
            domain_filter: Some(Arc::new(|host: &str| host.ends_with(".example.com"))),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        // Only the first entry should survive
        assert_eq!(mocks.len(), 1);
        let mock = &mocks[0];

        // URL relative, access_token gone, and the parameter it was recorded
        // alongside pinned separately so the recorded request still matches.
        let mc = mock.match_config.as_ref().unwrap();
        assert_eq!(mc.urls[0], "exact:/v2/users/me");
        assert_eq!(mc.query.get("fields").map(String::as_str), Some("name"));
        assert!(!mc.query.contains_key("access_token"));

        // Sensitive and infrastructure headers should be stripped
        let rc = mock.response_config.as_ref().unwrap();
        let headers = rc.headers().unwrap();
        assert!(headers.contains_key("content-type"));
        assert!(headers.contains_key("x-request-id"));
        assert!(!headers.contains_key("Authorization"));
        assert!(!headers.contains_key("x-envoy-upstream-service-time"));
        assert!(!headers.contains_key("date"));
    }

    // -- File loading --

    #[tokio::test]
    async fn test_load_har_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let har_path = temp_dir.path().join("test.har");

        let har_content = r#"{
      "log": {
        "version": "1.2",
        "creator": {
          "name": "test",
          "version": "1.0"
        },
        "entries": [
          {
            "startedDateTime": "2025-10-07T12:00:00.000Z",
            "time": 50,
            "request": {
              "method": "GET",
              "url": "https://api.example.com/v2/users/me",
              "httpVersion": "HTTP/1.1",
              "headers": [],
              "queryString": [],
              "cookies": [],
              "headersSize": -1,
              "bodySize": 0
            },
            "response": {
              "status": 200,
              "statusText": "OK",
              "httpVersion": "HTTP/1.1",
              "headers": [
                {
                  "name": "content-type",
                  "value": "application/json"
                }
              ],
              "cookies": [],
              "content": {
                "size": 100,
                "mimeType": "application/json",
                "text": "{\"id\":\"123\",\"name\":\"Test User\"}"
              },
              "redirectURL": "",
              "headersSize": -1,
              "bodySize": 100
            },
            "cache": {},
            "timings": {
              "send": 0,
              "wait": 50,
              "receive": 0
            }
          }
        ]
      }
    }"#;

        tokio::fs::write(&har_path, har_content)
            .await
            .expect("Failed to write HAR file");

        let loader = HarLoader::new();
        let mocks = loader
            .load_from_file(&har_path)
            .await
            .expect("Failed to load HAR file");

        assert_eq!(mocks.len(), 1);
        let match_config = mocks[0]
            .match_config
            .as_ref()
            .expect("match_config should exist");
        let response_config = mocks[0]
            .response_config
            .as_ref()
            .expect("response_config should exist");
        assert_eq!(match_config.methods[0], "GET");
        assert_eq!(response_config.status().expect("status should exist"), 200);
        // Should now be a relative path
        assert_eq!(match_config.urls[0], "exact:/v2/users/me");
    }

    #[tokio::test]
    async fn test_exclude_preflight() {
        let har = make_har(vec![
            create_test_entry("OPTIONS", "https://api.example.com/test", 204),
            create_test_entry("GET", "https://api.example.com/test", 200),
        ]);

        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let match_config = mocks[0]
            .match_config
            .as_ref()
            .expect("match_config should exist");
        assert_eq!(match_config.methods[0], "GET");
    }

    #[tokio::test]
    async fn test_domain_filter() {
        let har = make_har(vec![
            create_test_entry("GET", "https://api.example.com/v2/users/me", 200),
            create_test_entry("GET", "https://internal.mycompany.com/api/data", 200),
            create_test_entry("GET", "https://www.google.com/analytics", 200),
        ]);

        let loader = HarLoader::with_options(HarLoadOptions {
            domain_filter: Some(Arc::new(|host: &str| {
                host.ends_with(".example.com") || host.ends_with(".mycompany.com")
            })),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        // google.com filtered out
        assert_eq!(mocks.len(), 2);
    }
}
