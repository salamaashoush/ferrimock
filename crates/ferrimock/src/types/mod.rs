//! Core types for the mock engine

pub mod streaming;

pub use streaming::*;

use bytes::Bytes;
use globset::{Glob, GlobMatcher};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
pub use lean_string::LeanString;
use regex::Regex;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub use smallvec::SmallVec;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

/// Marker header set by `HttpResponse.error()`: the transport layer
/// (interceptor or mock server) simulates a network failure instead of
/// delivering the response.
pub const NETWORK_ERROR_HEADER: &str = "x-ferrimock-network-error";

/// Marker header set by `passthrough()` in scripted handlers: the
/// request should be treated as unhandled (interceptor forwards to the
/// real network; the standalone server falls through to unmatched).
pub const PASSTHROUGH_HEADER: &str = "x-ferrimock-passthrough";

/// Marker header for a handler that returned `undefined`/`null`: the
/// request falls through to the next matching mock (MSW semantics),
/// unlike `passthrough()` which skips all remaining handlers.
pub const FALLTHROUGH_HEADER: &str = "x-ferrimock-fallthrough";

/// Type alias for async handler functions that receive request context and produce dynamic responses.
///
/// This is the function signature used by programmatic mock handlers (MSW-style API).
/// Handlers receive the full [`RequestContext`] (method, path, captures, headers, body, etc.)
/// and return a [`DynamicResponse`] which can set status, headers, and body.
pub type HandlerFn = Arc<
    dyn Fn(
            RequestContext,
        )
            -> Pin<Box<dyn Future<Output = Result<DynamicResponse, crate::FerrimockError>> + Send>>
        + Send
        + Sync,
>;

/// Unified request context for both scripts and templates
/// This context contains all HTTP request information plus custom variables
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Full URI including query string
    pub uri: String,
    /// Request path (without query string)
    pub path: String,
    /// Query parameters
    pub query: FxHashMap<String, String>,
    /// URL regex captures from pattern matching
    pub captures: FxHashMap<String, String>,
    /// Request headers
    pub headers: FxHashMap<String, String>,
    /// Request body as string
    pub body: Option<String>,
    /// Raw body bytes, stored only when the body is not valid UTF-8
    /// (`body` is None then); use [`RequestContext::body_as_bytes`]
    pub body_bytes: Option<Bytes>,
    /// Request body parsed as JSON
    pub body_json: Option<Value>,
    /// Cascading variables (merged from global -> collection -> mock levels)
    pub vars: Option<serde_json::Map<String, Value>>,
}

impl RequestContext {
    /// Create a new empty request context
    pub fn new() -> Self {
        Self::default()
    }

    /// Create request context from HTTP components (materializes all fields).
    pub fn from_request(
        method: &str,
        uri: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) -> Self {
        Self::from_request_selective(method, uri, query, headers, body, true, true)
    }

    /// Create request context for a handler invocation: full headers and
    /// body string, but no eager JSON parse — both handler runtimes
    /// (V8 and QuickJS) parse the body lazily on first `bodyJson` access.
    pub fn from_request_for_handler(
        method: &str,
        uri: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) -> Self {
        let mut ctx = Self::from_request_selective(method, uri, query, headers, body, true, false);
        match body.map(std::str::from_utf8) {
            Some(Ok(s)) => ctx.body = Some(s.to_string()),
            Some(Err(_)) => ctx.body_bytes = body.map(Bytes::copy_from_slice),
            None => {}
        }
        ctx
    }

    /// Create request context, materializing only the fields the consumer needs.
    ///
    /// `want_headers` / `want_body` are computed once at load time from the
    /// template source (a template that never references `headers`/`body` does not
    /// pay for building the header map or parsing the body JSON per request).
    /// Handlers and non-template paths pass `true` for both (full context).
    pub fn from_request_selective(
        method: &str,
        uri: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
        want_headers: bool,
        want_body: bool,
    ) -> Self {
        // Parse query string
        let query_params = if let Some(q) = query {
            q.split('&')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    let key = parts.next()?.to_string();
                    let value = parts.next().unwrap_or_default().to_string();
                    Some((key, value))
                })
                .collect()
        } else {
            FxHashMap::default()
        };

        // Extract path (uri without query string)
        let path = uri.split('?').next().unwrap_or_default().to_string();

        // Extract headers only when the consumer references them.
        let header_map: FxHashMap<_, _> = if want_headers {
            headers
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
                .collect()
        } else {
            FxHashMap::default()
        };

        // Validate UTF-8 in-place (no copy) and parse JSON only when referenced.
        let (body_str, body_bytes, body_json) = if want_body {
            match body.map(std::str::from_utf8) {
                Some(Ok(s)) => {
                    let json = serde_json::from_str(s).ok();
                    (Some(s.to_string()), None, json)
                }
                Some(Err(_)) => (None, body.map(Bytes::copy_from_slice), None),
                None => (None, None, None),
            }
        } else {
            (None, None, None)
        };

        Self {
            method: method.to_string(),
            uri: uri.to_string(),
            path,
            query: query_params,
            captures: FxHashMap::default(), // Will be populated by matcher
            headers: header_map,
            body: body_str,
            body_bytes,
            body_json,
            vars: None, // Will be populated from mock definition's cascaded vars
        }
    }

    /// Raw body bytes regardless of UTF-8 validity.
    pub fn body_as_bytes(&self) -> Option<&[u8]> {
        self.body_bytes
            .as_deref()
            .or_else(|| self.body.as_deref().map(str::as_bytes))
    }

    /// Add a capture variable (builder pattern)
    #[must_use]
    pub fn with_capture(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.captures.insert(key.into(), value.into());
        self
    }

    /// Add a query parameter (builder pattern)
    #[must_use]
    pub fn with_query(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.query.insert(key.into(), value.into());
        self
    }

    /// Add a header (builder pattern)
    #[must_use]
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Extract URL captures from a regex match
    pub fn extract_captures_from_path(
        pattern: &regex::Regex,
        path: &str,
    ) -> FxHashMap<String, String> {
        let mut captures = FxHashMap::default();

        if let Some(caps) = pattern.captures(path) {
            // Get named captures
            for name in pattern.capture_names().flatten() {
                if let Some(value) = caps.name(name) {
                    // A zero-segment `:name*` participates with an empty
                    // match — MSW omits the param entirely.
                    if value.as_str().is_empty() && repeat_capture_name(name).is_some() {
                        continue;
                    }
                    captures.insert(
                        capture_param_key(name).to_string(),
                        value.as_str().to_string(),
                    );
                }
            }
        }

        captures
    }

    /// Extract query parameters from a query string
    pub fn extract_query_params(query: Option<&str>) -> FxHashMap<String, String> {
        let mut params = FxHashMap::default();

        if let Some(q) = query {
            for pair in q.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    params.insert(
                        urlencoding::decode(key).unwrap_or_default().to_string(),
                        urlencoding::decode(value).unwrap_or_default().to_string(),
                    );
                }
            }
        }

        params
    }
}

/// Context available to template expressions in patch operations.
///
/// Includes both request data (captures, vars, headers, body) and upstream
/// response data (status, headers, body_json). This enables dynamic patch
/// values like `{{ captures.id }}` or `{{ response.body_json.name }}`.
#[derive(Debug, Clone, Default)]
pub struct PatchContext {
    /// Request context (method, path, query, captures, headers, body, vars)
    pub request: RequestContext,
    /// Upstream response status code
    pub response_status: u16,
    /// Upstream response headers
    pub response_headers: FxHashMap<String, String>,
    /// Upstream response body parsed as JSON (None if not valid JSON)
    pub response_body_json: Option<serde_json::Value>,
}

/// A complete mock definition with request matching and response generation
#[derive(Debug, Clone)]
pub struct MockDefinition {
    /// Unique identifier for this mock
    pub id: LeanString,
    /// Priority for matching (higher = matched first)
    pub priority: u32,
    /// Request matching criteria
    pub request: RequestMatcher,
    /// Response generation configuration
    pub response: ResponseGenerator,
    /// Enabled flag
    pub enabled: bool,
    /// One-time handler: auto-disables after first match (MSW's `{ once: true }`)
    pub once: bool,
    /// Optional scope for test isolation (mocks in a scope can be deleted together)
    pub scope: Option<LeanString>,
    /// Source file path (for hot reload tracking)
    pub source_file: Option<String>,
    /// Request transforms for passthrough mode (None for full mocks)
    pub request_transforms: Option<ResolvedRequestTransforms>,
    /// Cascading variables (merged from global -> collection -> mock levels)
    /// These are injected into the template context as {{ vars.key }}
    pub vars: Option<serde_json::Map<String, Value>>,
    /// Long-lived connection behavior (WebSocket/SSE); None for every
    /// plain HTTP mock
    pub streaming: Option<StreamingResponse>,
}

/// Request matching criteria.
///
/// Uses `SmallVec` with per-field inline capacities tuned to real usage data:
/// - `methods`: capacity 2 — ~95% of mocks have 1 method, ~4% have 2 (e.g. GET+POST)
/// - `url_patterns`: capacity 1 — ~99% of mocks have exactly 1 URL pattern (UrlPattern is 96 bytes,
///   so [UrlPattern; 2] would waste 96 bytes in the common case)
/// - `header_matchers`: capacity 2 — ~70% have 0, ~25% have 1, ~4% have 2
/// - `query_matchers`: capacity 2 — ~85% have 0, ~10% have 1, ~4% have 2
#[derive(Debug, Clone, Default)]
pub struct RequestMatcher {
    /// HTTP methods to match (empty = match all)
    pub methods: SmallVec<[Method; 2]>,
    /// URL patterns to match (almost always exactly 1)
    pub url_patterns: SmallVec<[UrlPattern; 1]>,
    /// Header matchers
    pub header_matchers: SmallVec<[HeaderMatcher; 2]>,
    /// Query parameter matchers
    pub query_matchers: SmallVec<[QueryMatcher; 2]>,
    /// Body matcher
    pub body_matcher: Option<BodyMatcher>,
    /// GraphQL operation matcher (first-class!)
    pub graphql_matcher: Option<GraphQLMatcher>,
}

/// URL pattern matching strategies
#[derive(Debug, Clone)]
pub enum UrlPattern {
    /// Exact string match
    Exact(String),
    /// Prefix match
    Prefix(String),
    /// Suffix match
    Suffix(String),
    /// Regular expression match
    Regex(Regex),
    /// Regular expression tested against the bare path AND against
    /// `ws://host/path` / `wss://host/path` reconstructions of the
    /// request. Backs `ws.link(RegExp)`, whose MSW idiom matches the
    /// full connection href; the matcher supplies the host from the
    /// handshake's Host header.
    HrefRegex(Regex),
    /// Glob pattern match
    Glob(GlobMatcher),
}

/// Regex group names must be identifiers, so positional `*` wildcards are
/// compiled as `__wcN` and mapped back to MSW's numeric params keys
/// (`"0"`, `"1"`, …) at extraction time.
pub(crate) const WILDCARD_CAPTURE_PREFIX: &str = "__wc";

/// Repeatable params (`:name+` / `:name*`) are compiled as `__rp{name}`.
/// The marker survives into the captures map so the JS-facing lanes can
/// return them as arrays (MSW splits the matched segments on `/`), while
/// the template context sees the plain name with the joined value.
pub(crate) const REPEAT_CAPTURE_PREFIX: &str = "__rp";

pub(crate) fn capture_param_key(name: &str) -> &str {
    name.strip_prefix(WILDCARD_CAPTURE_PREFIX)
        .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
        .unwrap_or(name)
}

/// The param name behind a repeatable-capture key (`__rp{name}`), or
/// `None` for ordinary captures. MSW returns repeatable params as
/// `string[]` — consumers split the value on `/`.
pub fn repeat_capture_name(name: &str) -> Option<&str> {
    name.strip_prefix(REPEAT_CAPTURE_PREFIX)
        .filter(|rest| !rest.is_empty())
}

/// An MSW-shaped path param value: repeatable captures surface as the
/// matched segments (`string[]`), everything percent-decoded (MSW runs
/// `decodeURIComponent` on every captured value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MswParamValue {
    Single(String),
    List(Vec<String>),
}

fn decode_param(value: &str) -> String {
    urlencoding::decode(value).map_or_else(|_| value.to_string(), std::borrow::Cow::into_owned)
}

/// Convert raw URL captures to MSW-shaped params for the JS lanes:
/// `__rp` marker keys become `name -> [segments]`, other keys stay
/// single-valued; all values percent-decoded.
#[allow(clippy::implicit_hasher)]
pub fn msw_params(captures: &FxHashMap<String, String>) -> Vec<(String, MswParamValue)> {
    captures
        .iter()
        .map(|(key, value)| match repeat_capture_name(key) {
            Some(name) => (
                name.to_string(),
                MswParamValue::List(value.split('/').map(decode_param).collect()),
            ),
            None => (key.clone(), MswParamValue::Single(decode_param(value))),
        })
        .collect()
}

impl UrlPattern {
    /// Check if the pattern matches the given path
    pub fn matches(&self, path: &str) -> bool {
        match self {
            UrlPattern::Exact(s) => path == s,
            UrlPattern::Prefix(s) => path.starts_with(s),
            UrlPattern::Suffix(s) => path.ends_with(s),
            UrlPattern::Regex(re) | UrlPattern::HrefRegex(re) => re.is_match(path),
            UrlPattern::Glob(g) => g.is_match(path),
        }
    }

    /// Test an [`UrlPattern::HrefRegex`] against reconstructed hrefs.
    /// When the caller knows the connection's real scheme (the Node
    /// interceptor lane sees the full `ws://`/`wss://` URL) it passes it
    /// and only that variant is tested. On the TCP lane the scheme is
    /// unknowable server-side (plain-HTTP servers can sit behind TLS
    /// termination), so both `ws://` and `wss://` variants are tried.
    pub fn matches_href(
        &self,
        scheme: Option<&str>,
        host: &str,
        path: &str,
        query: Option<&str>,
    ) -> bool {
        let UrlPattern::HrefRegex(re) = self else {
            return false;
        };
        let suffix = match query {
            Some(q) => format!("{path}?{q}"),
            None => path.to_string(),
        };
        match scheme {
            Some(scheme) => re.is_match(&format!("{scheme}://{host}{suffix}")),
            None => {
                re.is_match(&format!("ws://{host}{suffix}"))
                    || re.is_match(&format!("wss://{host}{suffix}"))
            }
        }
    }

    /// Extract named captures from a regex pattern match
    /// Returns None if the pattern doesn't match or doesn't have named captures
    pub fn extract_captures(&self, path: &str) -> Option<FxHashMap<String, String>> {
        match self {
            UrlPattern::Regex(re) => {
                if let Some(caps) = re.captures(path) {
                    let mut captures = FxHashMap::default();

                    // Get named captures
                    for name in re.capture_names().flatten() {
                        if let Some(value) = caps.name(name) {
                            // A zero-segment `:name*` participates with an
                            // empty match — MSW omits the param entirely.
                            if value.as_str().is_empty() && repeat_capture_name(name).is_some() {
                                continue;
                            }
                            captures.insert(
                                capture_param_key(name).to_string(),
                                value.as_str().to_string(),
                            );
                        }
                    }

                    if !captures.is_empty() {
                        return Some(captures);
                    }
                }
                None
            }
            // Other pattern types don't support captures (HrefRegex
            // mirrors MSW's RegExp handlers, whose params are empty)
            UrlPattern::Exact(_)
            | UrlPattern::Prefix(_)
            | UrlPattern::Suffix(_)
            | UrlPattern::HrefRegex(_)
            | UrlPattern::Glob(_) => None,
        }
    }

    /// Create an exact match pattern
    pub fn exact(s: impl Into<String>) -> Self {
        UrlPattern::Exact(s.into())
    }

    /// Create a prefix match pattern
    pub fn prefix(s: impl Into<String>) -> Self {
        UrlPattern::Prefix(s.into())
    }

    /// Create a suffix match pattern
    pub fn suffix(s: impl Into<String>) -> Self {
        UrlPattern::Suffix(s.into())
    }

    /// Create a regex match pattern
    pub fn regex(pattern: &str) -> Result<Self, regex::Error> {
        Ok(UrlPattern::Regex(Regex::new(pattern)?))
    }

    /// Create a glob match pattern
    pub fn glob(pattern: &str) -> Result<Self, globset::Error> {
        let glob = Glob::new(pattern)?;
        Ok(UrlPattern::Glob(glob.compile_matcher()))
    }

    /// Compile an MSW-style path pattern with `:param` placeholders to a regex.
    ///
    /// Converts patterns like `/users/:id/posts/:postId` into
    /// `^/users/(?P<id>[^/]+)/posts/(?P<postId>[^/]+)$` with named captures.
    ///
    /// Supports:
    /// - `:param` — named path parameter (matches one segment)
    /// - `:param+` / `:param*` — repeatable parameter (one-or-more /
    ///   zero-or-more segments, path-to-regexp modifiers); captured as
    ///   `__rp{param}` so the JS lanes return the segments as `string[]`
    /// - `*` as a full segment — greedy wildcard (crosses `/`, like
    ///   path-to-regexp); captured positionally as params `"0"`, `"1"`, …
    /// - Literal segments — escaped for regex safety
    ///
    /// # Examples
    /// ```
    /// # use ferrimock::types::UrlPattern;
    /// let pattern = UrlPattern::path_pattern("/users/:id").unwrap();
    /// assert!(pattern.matches("/users/123"));
    /// assert!(!pattern.matches("/users/123/extra"));
    ///
    /// let repeat = UrlPattern::path_pattern("/files/:path+").unwrap();
    /// assert!(repeat.matches("/files/a/b/c"));
    /// assert!(!repeat.matches("/files"));
    ///
    /// let optional = UrlPattern::path_pattern("/files/:path*").unwrap();
    /// assert!(optional.matches("/files/a/b"));
    /// assert!(optional.matches("/files"));
    /// ```
    pub fn path_pattern(pattern: &str) -> Result<Self, regex::Error> {
        use std::fmt::Write;

        let mut wildcard_index = 0usize;
        let mut regex_str = String::from("^");
        for (index, segment) in pattern.split('/').enumerate() {
            let sep = if index == 0 { "" } else { "/" };
            if let Some(param) = segment.strip_prefix(':') {
                if let Some(name) = param.strip_suffix('+') {
                    let _ = write!(regex_str, "{sep}(?P<{REPEAT_CAPTURE_PREFIX}{name}>.+)");
                } else if let Some(name) = param.strip_suffix('*') {
                    // Zero segments must also match the path without the
                    // separator, so the group swallows its own slash.
                    let _ = write!(regex_str, "(?:{sep}(?P<{REPEAT_CAPTURE_PREFIX}{name}>.*))?");
                } else {
                    let _ = write!(regex_str, "{sep}(?P<{param}>[^/]+)");
                }
            } else if segment == "*" {
                let _ = write!(
                    regex_str,
                    "{sep}(?P<{WILDCARD_CAPTURE_PREFIX}{wildcard_index}>.*)"
                );
                wildcard_index += 1;
            } else {
                regex_str.push_str(sep);
                regex_str.push_str(&regex::escape(segment));
            }
        }
        regex_str.push('$');

        UrlPattern::regex(&regex_str)
    }

    /// Split an absolute-URL predicate (`https://api.example.com/users/:id`)
    /// into its host (`api.example.com`, including any port) and path
    /// (`/users/:id`). Returns None for path-only predicates. Matching is
    /// scheme-agnostic: the host becomes a Host-header matcher and the path
    /// goes through [`UrlPattern::path_pattern`].
    pub fn split_absolute_url(pattern: &str) -> Option<(&str, &str)> {
        let rest = pattern
            .strip_prefix("https://")
            .or_else(|| pattern.strip_prefix("http://"))?;
        match rest.find('/') {
            Some(idx) => Some(rest.split_at(idx)),
            None => Some((rest, "/")),
        }
    }
}

/// Header matching criteria
#[derive(Debug, Clone)]
pub struct HeaderMatcher {
    /// Header name to match
    pub name: HeaderName,
    /// Expected value (can be a regex pattern)
    pub pattern: HeaderMatchPattern,
}

impl HeaderMatcher {
    /// Create a new header matcher with exact value match
    pub fn exact(name: HeaderName, value: impl Into<String>) -> Self {
        Self {
            name,
            pattern: HeaderMatchPattern::Exact(value.into()),
        }
    }

    /// Create a new header matcher with regex pattern
    pub fn regex(name: HeaderName, pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            name,
            pattern: HeaderMatchPattern::Regex(Regex::new(pattern)?),
        })
    }

    /// Create a matcher that checks for header presence
    pub fn present(name: HeaderName) -> Self {
        Self {
            name,
            pattern: HeaderMatchPattern::Present,
        }
    }

    /// Create a matcher that checks for header absence
    pub fn absent(name: HeaderName) -> Self {
        Self {
            name,
            pattern: HeaderMatchPattern::Absent,
        }
    }

    /// Check if the matcher matches the given headers
    pub fn matches(&self, headers: &HeaderMap) -> bool {
        match &self.pattern {
            HeaderMatchPattern::Exact(expected) => headers
                .get(&self.name)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v == expected),
            HeaderMatchPattern::Regex(re) => headers
                .get(&self.name)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| re.is_match(v)),
            HeaderMatchPattern::Present => headers.contains_key(&self.name),
            HeaderMatchPattern::Absent => !headers.contains_key(&self.name),
        }
    }
}

/// Header matching pattern
#[derive(Debug, Clone)]
pub enum HeaderMatchPattern {
    /// Exact value match
    Exact(String),
    /// Regex pattern match
    Regex(Regex),
    /// Header must be present
    Present,
    /// Header must be absent
    Absent,
}

/// Query parameter matching criteria
#[derive(Debug, Clone)]
pub struct QueryMatcher {
    /// Query parameter name to match
    pub name: String,
    /// Expected value pattern
    pub pattern: QueryMatchPattern,
}

impl QueryMatcher {
    /// Create a new query matcher with exact value match
    pub fn exact(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pattern: QueryMatchPattern::Exact(value.into()),
        }
    }

    /// Create a new query matcher with regex pattern
    pub fn regex(name: impl Into<String>, pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            name: name.into(),
            pattern: QueryMatchPattern::Regex(Regex::new(pattern)?),
        })
    }

    /// Create a matcher that checks for query parameter presence
    pub fn present(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pattern: QueryMatchPattern::Present,
        }
    }

    /// Create a matcher that checks for query parameter absence
    pub fn absent(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pattern: QueryMatchPattern::Absent,
        }
    }

    /// Check if the matcher matches the given query string
    pub fn matches(&self, query: Option<&str>) -> bool {
        // Parse query parameters with URL decoding
        let query_params = Self::parse_query(query);
        self.matches_parsed(&query_params)
    }

    /// Check if the matcher matches pre-parsed query parameters
    /// Use this method when you have multiple matchers to check against the same query string
    /// to avoid re-parsing the query string for each matcher
    #[inline]
    pub fn matches_parsed(&self, query_params: &FxHashMap<String, String>) -> bool {
        match &self.pattern {
            QueryMatchPattern::Exact(expected) => {
                query_params.get(&self.name).is_some_and(|v| v == expected)
            }
            QueryMatchPattern::Regex(re) => {
                query_params.get(&self.name).is_some_and(|v| re.is_match(v))
            }
            QueryMatchPattern::Present => query_params.contains_key(&self.name),
            QueryMatchPattern::Absent => !query_params.contains_key(&self.name),
        }
    }

    /// Parse a query string into a hashmap
    /// Returns an empty map if query is None or empty
    pub fn parse_query(query: Option<&str>) -> FxHashMap<String, String> {
        if let Some(q) = query {
            let mut params = FxHashMap::default();
            for pair in q.split('&') {
                if let Some((key, value)) = pair.split_once('=') {
                    let decoded_key = urlencoding::decode(key).unwrap_or_default().to_string();
                    let decoded_value = urlencoding::decode(value).unwrap_or_default().to_string();
                    params.insert(decoded_key, decoded_value);
                }
            }
            params
        } else {
            FxHashMap::default()
        }
    }
}

/// Query parameter matching pattern
#[derive(Debug, Clone)]
pub enum QueryMatchPattern {
    /// Exact value match
    Exact(String),
    /// Regex pattern match
    Regex(Regex),
    /// Parameter must be present (any value)
    Present,
    /// Parameter must be absent
    Absent,
}

/// Body matching strategies
#[derive(Debug, Clone)]
pub enum BodyMatcher {
    /// Body contains substring
    Contains(String),
    /// Body matches regex
    Regex(Regex),
    /// JSON body matches using JSONPath
    JsonPath {
        path: String,
        value: serde_json::Value,
    },
    /// Exact JSON match
    JsonEquals(serde_json::Value),
}

impl BodyMatcher {
    /// Create a contains matcher
    pub fn contains(s: impl Into<String>) -> Self {
        BodyMatcher::Contains(s.into())
    }

    /// Create a regex matcher
    pub fn regex(pattern: &str) -> Result<Self, regex::Error> {
        Ok(BodyMatcher::Regex(Regex::new(pattern)?))
    }

    /// Create a JSON path matcher
    pub fn json_path(path: impl Into<String>, value: serde_json::Value) -> Self {
        BodyMatcher::JsonPath {
            path: path.into(),
            value,
        }
    }

    /// Create a JSON equals matcher
    pub fn json_equals(value: serde_json::Value) -> Self {
        BodyMatcher::JsonEquals(value)
    }

    /// Check if the matcher matches the given body.
    ///
    /// `parsed_json` is an optional request-body JSON parsed once per request and
    /// shared across all matchers; JSON matchers reuse it instead of re-parsing.
    /// Pass `None` to have JSON matchers parse the body themselves.
    pub fn matches(&self, body: &[u8], parsed_json: Option<&serde_json::Value>) -> bool {
        match self {
            BodyMatcher::Contains(substr) => {
                if let Ok(body_str) = std::str::from_utf8(body) {
                    body_str.contains(substr)
                } else {
                    false
                }
            }
            BodyMatcher::Regex(re) => {
                if let Ok(body_str) = std::str::from_utf8(body) {
                    re.is_match(body_str)
                } else {
                    false
                }
            }
            BodyMatcher::JsonPath { path, value } => Self::with_json(body, parsed_json, |json| {
                Self::json_path_match(json, path, value)
            }),
            BodyMatcher::JsonEquals(expected) => {
                Self::with_json(body, parsed_json, |json| json == expected)
            }
        }
    }

    /// Resolve request-body JSON: use the shared pre-parsed value when present,
    /// otherwise parse from bytes. Returns false if no valid JSON is available.
    #[inline]
    fn with_json<F: FnOnce(&serde_json::Value) -> bool>(
        body: &[u8],
        parsed_json: Option<&serde_json::Value>,
        f: F,
    ) -> bool {
        match parsed_json {
            Some(json) => f(json),
            None => serde_json::from_slice::<serde_json::Value>(body).is_ok_and(|json| f(&json)),
        }
    }

    /// JSONPath matching with array index support
    ///
    /// Supports:
    /// - Object access: `$.user.name` or `user.name`
    /// - Array indexing: `$.users[0].name`
    /// - Nested arrays: `$.data.items[1].subitems[0].id`
    /// - Mixed: `$.response.users[2].addresses[0].city`
    fn json_path_match(json: &serde_json::Value, path: &str, expected: &serde_json::Value) -> bool {
        json_path_lookup(json, path).is_some_and(|found| found == expected)
    }

    /// Parse JSONPath segments handling both object keys and array indices
    ///
    /// Examples:
    /// - `"user.name"` -> `[Key("user"), Key("name")]`
    /// - `"users[0].name"` -> `[Key("users"), Index(0), Key("name")]`
    /// - `"data[1][2].value"` -> `[Key("data"), Index(1), Index(2), Key("value")]`
    pub(crate) fn parse_jsonpath_segments(path: &str) -> Vec<PathSegment> {
        let mut segments = Vec::new();
        let mut current_key = String::new();
        let mut chars = path.chars();

        while let Some(ch) = chars.next() {
            match ch {
                '.' => {
                    // Dot separator - finish current key if any
                    if !current_key.is_empty() {
                        segments.push(PathSegment::Key(current_key.clone()));
                        current_key.clear();
                    }
                }
                '[' => {
                    // Array index start - finish current key if any
                    if !current_key.is_empty() {
                        segments.push(PathSegment::Key(current_key.clone()));
                        current_key.clear();
                    }

                    // Parse array index
                    let mut index_str = String::new();
                    for next_ch in chars.by_ref() {
                        if next_ch == ']' {
                            break;
                        }
                        index_str.push(next_ch);
                    }

                    // Parse index as number
                    if let Ok(index) = index_str.parse::<usize>() {
                        segments.push(PathSegment::Index(index));
                    }
                    // If parsing fails, we just skip this segment
                }
                _ => {
                    // Regular character - add to current key
                    current_key.push(ch);
                }
            }
        }

        // Add final key if any
        if !current_key.is_empty() {
            segments.push(PathSegment::Key(current_key));
        }

        segments
    }
}

/// Path segment for JSONPath traversal
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PathSegment {
    /// Object key access
    Key(String),
    /// Array index access
    Index(usize),
}

/// Resolve a simple JSONPath (`$.a.b[0].c`) against a JSON value.
///
/// Shared by body matchers and WebSocket message rules.
pub fn json_path_lookup<'a>(
    json: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let path = path.strip_prefix("$.").unwrap_or(path);
    let path = path.strip_prefix('$').unwrap_or(path);

    let mut current = json;
    for segment in BodyMatcher::parse_jsonpath_segments(path) {
        current = match segment {
            PathSegment::Key(key) => current.get(key)?,
            PathSegment::Index(idx) => current.as_array()?.get(idx)?,
        };
    }
    Some(current)
}

/// GraphQL operation matcher
#[derive(Debug, Clone, Default)]
pub struct GraphQLMatcher {
    /// Operation name (e.g., "GetUser")
    pub operation_name: Option<String>,
    /// Operation name as a regex (MSW accepts RegExp operation predicates)
    pub operation_name_regex: Option<Regex>,
    /// Operation type (query, mutation, subscription)
    pub operation_type: Option<GraphQLOperationType>,
    /// Match any operation (wildcard)
    pub match_any: bool,
    /// Variable constraints (e.g., id = "123", input.role = "admin")
    pub variable_matchers: FxHashMap<String, serde_json::Value>,
    /// Introspection matcher
    pub introspection_matcher: Option<IntrospectionMatcher>,
}

/// GraphQL operation type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphQLOperationType {
    Query,
    Mutation,
    Subscription,
}

/// Introspection query matcher
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntrospectionMatcher {
    /// Match any introspection query (__schema, __type, __typename)
    Any,
    /// Match only __schema queries (full schema introspection)
    Schema,
    /// Match only __type queries (specific type introspection)
    Type,
    /// Match only __typename queries (typename resolution)
    TypeName,
}

/// Response generation mode
#[derive(Debug, Clone, Default)]
pub enum ResponseMode {
    /// Static response (default)
    #[default]
    Static,
    /// Template-based response
    Template,
    /// Patch an upstream response
    Patch { operations: Vec<PatchOperation> },
}

/// JSON patch operations
#[derive(Debug, Clone)]
pub enum PatchOperation {
    /// RFC 6902 JSON Patch
    JsonPatch(json_patch::Patch),
    /// JSONPath-style patch
    JsonPath {
        path: String,
        value: serde_json::Value,
    },
    /// Regex replace in body
    RegexReplace {
        pattern: regex::Regex,
        replacement: String,
    },
    /// Add header
    HeaderAdd { name: String, value: String },
    /// Remove header
    HeaderRemove { name: String },
}

impl PatchOperation {}

/// Request modification operations (applied before forwarding to upstream)
#[derive(Debug, Clone)]
pub enum RequestPatch {
    /// Add or replace a request header
    HeaderAdd { name: String, value: String },
    /// Remove a request header
    HeaderRemove { name: String },
    /// Add or replace a query parameter
    QueryAdd { name: String, value: String },
    /// Remove a query parameter
    QueryRemove { name: String },
    /// JSONPath-style body patch
    JsonPath {
        path: String,
        value: serde_json::Value,
    },
    /// RFC 6902 JSON Patch for body
    JsonPatch(json_patch::Patch),
    /// Regex replacement in body
    RegexReplace {
        pattern: regex::Regex,
        replacement: String,
    },
}

/// Optional overrides for upstream request behavior
#[derive(Debug, Clone, Default)]
pub struct UpstreamOptions {
    /// Custom timeout for this specific upstream request
    pub timeout: Option<std::time::Duration>,
    /// Override the upstream host/URL (e.g., forward to staging)
    pub forward_to: Option<String>,
}

/// Resolved request transforms (parsed from config, ready for engine use)
#[derive(Debug, Clone)]
pub struct ResolvedRequestTransforms {
    /// Request patch operations (headers, query, body)
    pub patches: Vec<RequestPatch>,
    /// Delay before forwarding to upstream
    pub pre_delay: Option<std::time::Duration>,
    /// Upstream request options (timeout, forward_to)
    pub upstream_options: UpstreamOptions,
    /// Rewrite path template string (rendered at match time with RequestContext)
    pub rewrite_path: Option<String>,
}

/// Dynamic response metadata that can be returned by scripts/templates
/// This allows runtime control of status codes and headers
#[derive(Debug, Clone, Default)]
pub struct DynamicResponse {
    /// HTTP status code (overrides mock definition if present)
    pub status: Option<StatusCode>,
    /// Custom status text. HTTP/2 dropped reason phrases, so this never
    /// reaches the wire on the standalone server; the Node interceptor
    /// applies it when reconstructing the JS `Response`.
    pub status_text: Option<String>,
    /// Additional response headers (merged with mock definition headers)
    pub headers: Option<FxHashMap<String, String>>,
    /// Response body bytes
    pub body: bytes::Bytes,
}

impl DynamicResponse {
    /// Create a new dynamic response with just a body (uses mock defaults for status/headers)
    pub fn body_only(body: bytes::Bytes) -> Self {
        Self {
            body,
            ..Self::default()
        }
    }

    /// A response that signals MSW-style fall-through: the caller should
    /// retry matching with this mock excluded.
    pub fn fallthrough() -> Self {
        let mut headers = FxHashMap::default();
        headers.insert(FALLTHROUGH_HEADER.to_string(), "1".to_string());
        Self {
            headers: Some(headers),
            ..Self::default()
        }
    }

    /// Whether this response carries the fall-through marker.
    pub fn is_fallthrough(&self) -> bool {
        self.headers
            .as_ref()
            .is_some_and(|h| h.get(FALLTHROUGH_HEADER).is_some_and(|v| v == "1"))
    }

    /// Parse a JSON value into a DynamicResponse
    ///
    /// This is the unified parsing logic used by both templates and scripts.
    /// It checks for structured response format: { status?, headers?, body }
    /// - If the response has status/headers/body fields, parse them
    /// - If not, use the entire JSON as the body
    pub fn from_json(json: &serde_json::Value) -> Result<Self, crate::FerrimockError> {
        if let Some(obj) = json.as_object() {
            // Check for structured response format: { status, headers, body }
            let has_status = obj.contains_key("status");
            let has_headers = obj.contains_key("headers");
            let has_body = obj.contains_key("body");

            if has_status || has_headers || has_body {
                // Parse structured response
                let status = obj
                    .get("status")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|code| u16::try_from(code).ok())
                    .and_then(|code| StatusCode::from_u16(code).ok());

                let headers = obj
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|header_obj| {
                        header_obj
                            .iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect::<FxHashMap<String, String>>()
                    });

                // Use "body" field if present, otherwise use the whole result
                let body_value = if let Some(body_val) = obj.get("body") {
                    body_val
                } else {
                    json
                };

                let body_str = serde_json::to_string(body_value)?;
                let body_bytes = bytes::Bytes::from(body_str);

                return Ok(DynamicResponse {
                    status,
                    headers,
                    body: body_bytes,
                    ..Self::default()
                });
            }
        }

        // Not a structured response - just return body
        let body_str = serde_json::to_string(json)?;
        Ok(DynamicResponse::body_only(bytes::Bytes::from(body_str)))
    }

    /// Parse a rendered string (potentially JSON) into a DynamicResponse.
    ///
    /// Tries to parse as JSON first. If successful and has structured format
    /// (status/headers/body fields), extracts them. Otherwise returns the
    /// original string as-is without a wasteful parse→re-serialize round-trip.
    pub fn from_rendered_string(rendered: String) -> Self {
        // Quick check: only attempt JSON parse if it looks like a JSON object
        if rendered.starts_with('{')
            && let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&rendered)
        {
            if let Some(obj) = json_value.as_object() {
                let has_status = obj.contains_key("status");
                let has_headers = obj.contains_key("headers");
                let has_body = obj.contains_key("body");

                if has_status || has_headers || has_body {
                    // Structured response -- extract fields
                    if let Ok(dynamic_response) = Self::from_json(&json_value) {
                        return dynamic_response;
                    }
                }
            }
            // Valid JSON but not structured -- return original string directly
            // instead of re-serializing (avoids wasteful Value -> String round-trip)
            return Self::body_only(bytes::Bytes::from(rendered));
        }

        // Not JSON — return as body
        Self::body_only(bytes::Bytes::from(rendered))
    }
}

/// Response generation configuration
#[derive(Debug, Clone)]
pub struct ResponseGenerator {
    /// HTTP status code (default, can be overridden by scripts/templates)
    pub status: StatusCode,
    /// Response headers (default, can be extended/overridden by scripts/templates)
    pub headers: FxHashMap<String, String>,
    /// Response body source
    pub body: BodySource,
    /// Optional delay before responding
    pub delay: Option<Duration>,
    /// Response generation mode
    pub mode: ResponseMode,
    /// Whether the template may produce a structured response (`{ "status": ..., "headers": ..., "body": ... }`).
    /// When false, the rendered output is used directly as the body (skips JSON parse).
    pub structured_response: bool,
    /// Whether the template body references request headers (computed at load time).
    /// When false, the per-request context skips building the header map.
    pub context_uses_headers: bool,
    /// Whether the template body references the request body (computed at load time).
    /// When false, the per-request context skips the body string + JSON parse.
    pub context_uses_body: bool,
}

impl ResponseGenerator {
    /// Create a new response generator with the given status and body
    pub fn new(status: StatusCode, body: BodySource) -> Self {
        let structured_response = body.may_produce_structured_response();
        let (context_uses_headers, context_uses_body) = body.context_needs();
        Self {
            status,
            headers: FxHashMap::default(),
            body,
            delay: None,
            mode: ResponseMode::default(),
            structured_response,
            context_uses_headers,
            context_uses_body,
        }
    }

    /// Add a header to the response
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Set the response delay
    #[must_use]
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }

    /// Set the response mode
    #[must_use]
    pub fn with_mode(mut self, mode: ResponseMode) -> Self {
        self.mode = mode;
        self
    }

    /// Replace the body source and recalculate the structured response flag
    pub fn set_body(&mut self, body: BodySource) {
        self.structured_response = body.may_produce_structured_response();
        let (uses_headers, uses_body) = body.context_needs();
        self.context_uses_headers = uses_headers;
        self.context_uses_body = uses_body;
        self.body = body;
    }

    /// Generate the response body for non-template sources only
    ///
    /// Note: Template rendering is not available in bdg-mock-types.
    /// Use bdg-mock-template or bdg-mock-engine for template support.
    pub async fn generate_static(&self) -> Result<bytes::Bytes, crate::FerrimockError> {
        // Apply delay if configured
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }

        match &self.body {
            BodySource::Inline(cached_bytes) => {
                // Zero-copy inline content via Arc
                Ok((**cached_bytes).clone())
            }
            BodySource::File(path) => {
                let content = tokio::fs::read(path).await?;
                Ok(bytes::Bytes::from(content))
            }
            BodySource::FileCached(cached_bytes) => {
                // Zero-copy cached file content
                Ok((**cached_bytes).clone())
            }
            BodySource::Template { .. } => Err(crate::mp_err!(
                "Template rendering not available in bdg-mock-types. Use bdg-mock-template or bdg-mock-engine."
            )),
            BodySource::Handler(_) => Err(crate::mp_err!(
                "Handler-based responses require generate_dynamic(). Use the engine's ResponseGeneratorExt."
            )),
        }
    }

    /// Build header map from the configured headers
    pub fn build_headers(&self) -> Result<HeaderMap, crate::FerrimockError> {
        let mut header_map = HeaderMap::new();
        for (name, value) in &self.headers {
            let header_name = HeaderName::try_from(name.as_str())?;
            let header_value = HeaderValue::try_from(value.as_str())?;
            header_map.insert(header_name, header_value);
        }
        Ok(header_map)
    }
}

/// Source for response body content
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodySource {
    /// Inline string content - now uses `Arc<Bytes>` for zero-copy
    #[serde(
        serialize_with = "serialize_inline",
        deserialize_with = "deserialize_inline"
    )]
    Inline(Arc<bytes::Bytes>),
    /// Content from a file (path - will be read on demand, not cached)
    File(PathBuf),
    /// Cached file content (pre-loaded into memory for performance)
    #[serde(skip)]
    FileCached(Arc<bytes::Bytes>),
    /// Template string with pre-computed hash for cache lookup
    Template {
        source: String,
        /// Pre-computed FxHash of the template source (avoids re-hashing on every render)
        #[serde(skip)]
        hash: u64,
    },
    /// Function-based response handler (programmatic MSW-style API).
    ///
    /// Receives the full [`RequestContext`] and returns a [`DynamicResponse`].
    /// Only created programmatically via the handler builder API, never from config files.
    #[serde(skip)]
    Handler(HandlerFn),
}

impl std::fmt::Debug for BodySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodySource::Inline(b) => f.debug_tuple("Inline").field(b).finish(),
            BodySource::File(p) => f.debug_tuple("File").field(p).finish(),
            BodySource::FileCached(b) => f.debug_tuple("FileCached").field(b).finish(),
            BodySource::Template { source, hash } => f
                .debug_struct("Template")
                .field("source", source)
                .field("hash", hash)
                .finish(),
            BodySource::Handler(_) => f.debug_tuple("Handler").field(&"<fn>").finish(),
        }
    }
}

// Custom serialization for Inline to maintain serde compatibility
fn serialize_inline<S>(bytes: &Arc<bytes::Bytes>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = std::str::from_utf8(bytes).map_err(serde::ser::Error::custom)?;
    serializer.serialize_str(s)
}

fn deserialize_inline<'de, D>(deserializer: D) -> Result<Arc<bytes::Bytes>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(Arc::new(bytes::Bytes::from(s)))
}

impl BodySource {
    /// Create an inline body source (zero-copy via Arc)
    pub fn inline(content: impl Into<String>) -> Self {
        BodySource::Inline(Arc::new(bytes::Bytes::from(content.into())))
    }

    /// Create a file body source
    pub fn file(path: impl Into<PathBuf>) -> Self {
        BodySource::File(path.into())
    }

    /// Create a template body source with pre-computed hash
    pub fn template(content: impl Into<String>) -> Self {
        let source = content.into();
        let hash = template_hash(&source);
        BodySource::Template { source, hash }
    }

    /// Create a handler body source from a function
    pub fn handler(f: HandlerFn) -> Self {
        BodySource::Handler(f)
    }

    /// Which request-context fields this body may reference: `(headers, body)`.
    ///
    /// For templates this is a substring scan of the source — template *functions*
    /// (fake/store) never read the request context, so the only access path to
    /// headers/body is the `{{ headers }}` / `{{ body }}` / `{{ body_json }}`
    /// variables, both of which contain "header"/"body". A false positive only
    /// costs a little extra work; there is no false-negative path. Non-template
    /// bodies (handlers, etc.) get the full context.
    fn context_needs(&self) -> (bool, bool) {
        match self {
            BodySource::Template { source, .. } => {
                (source.contains("header"), source.contains("body"))
            }
            _ => (true, true),
        }
    }

    /// Whether this body source may produce a structured response with status/headers/body fields.
    /// Used to skip expensive JSON parsing when the template just returns a plain body.
    pub fn may_produce_structured_response(&self) -> bool {
        match self {
            BodySource::Template { source, .. } => {
                source.contains("\"status\"")
                    || source.contains("\"headers\"")
                    || source.contains("\"body\"")
            }
            // Handlers return DynamicResponse directly, which already has structured fields
            BodySource::Handler(_) => true,
            _ => false,
        }
    }
}

/// FxHash of a template source — the key into the shared render cache
/// (used by response bodies and streaming SSE/WS payload templates).
pub fn template_hash(template: &str) -> u64 {
    use rustc_hash::FxHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    template.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::get_unwrap
)]
mod tests {
    use super::*;

    #[test]
    fn test_url_pattern_exact_match() {
        let pattern = UrlPattern::exact("/api/users");
        assert!(pattern.matches("/api/users"));
        assert!(!pattern.matches("/api/users/123"));
        assert!(!pattern.matches("/api/user"));
    }

    #[test]
    fn test_url_pattern_prefix_match() {
        let pattern = UrlPattern::prefix("/api/");
        assert!(pattern.matches("/api/users"));
        assert!(pattern.matches("/api/files"));
        assert!(!pattern.matches("/v2/api/users"));
    }

    #[test]
    fn test_url_pattern_suffix_match() {
        let pattern = UrlPattern::suffix(".json");
        assert!(pattern.matches("/api/users.json"));
        assert!(pattern.matches("/data.json"));
        assert!(!pattern.matches("/api/users.xml"));
    }

    #[test]
    fn test_url_pattern_regex_match() {
        let pattern = UrlPattern::regex(r"^/api/users/\d+$").unwrap();
        assert!(pattern.matches("/api/users/123"));
        assert!(pattern.matches("/api/users/456"));
        assert!(!pattern.matches("/api/users/abc"));
        assert!(!pattern.matches("/api/users/123/posts"));
    }

    #[test]
    fn test_url_pattern_glob_match() {
        let pattern = UrlPattern::glob("/api/**/*.json").unwrap();
        assert!(pattern.matches("/api/users/123.json"));
        assert!(pattern.matches("/api/v2/files/data.json"));
        assert!(!pattern.matches("/api/users/123.xml"));
    }

    #[test]
    fn test_header_matcher_exact() {
        let matcher =
            HeaderMatcher::exact(HeaderName::from_static("content-type"), "application/json");

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        assert!(matcher.matches(&headers));

        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("text/plain"),
        );

        assert!(!matcher.matches(&headers));
    }

    #[test]
    fn test_header_matcher_regex() {
        let matcher = HeaderMatcher::regex(
            HeaderName::from_static("content-type"),
            r"^application/(json|xml)$",
        )
        .unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        assert!(matcher.matches(&headers));

        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/xml"),
        );
        assert!(matcher.matches(&headers));

        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("text/plain"),
        );
        assert!(!matcher.matches(&headers));
    }

    #[test]
    fn test_header_matcher_present() {
        let matcher = HeaderMatcher::present(HeaderName::from_static("authorization"));

        let mut headers = HeaderMap::new();
        assert!(!matcher.matches(&headers));

        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer token"),
        );
        assert!(matcher.matches(&headers));
    }

    #[test]
    fn test_header_matcher_absent() {
        let matcher = HeaderMatcher::absent(HeaderName::from_static("x-api-key"));

        let mut headers = HeaderMap::new();
        assert!(matcher.matches(&headers));

        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );
        assert!(!matcher.matches(&headers));
    }

    #[test]
    fn test_body_source_inline() {
        let body = BodySource::inline("test content");
        match body {
            BodySource::Inline(content) => assert_eq!(content.as_ref().as_ref(), b"test content"),
            _ => panic!("Expected inline body source"),
        }
    }

    #[test]
    fn test_body_source_file() {
        let body = BodySource::file("/path/to/file.json");
        match body {
            BodySource::File(path) => assert_eq!(path.to_str().unwrap(), "/path/to/file.json"),
            _ => panic!("Expected file body source"),
        }
    }

    #[test]
    fn test_response_generator_with_header() {
        let response = ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}"))
            .with_header("content-type", "application/json");

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(
            response.headers.get("content-type").unwrap(),
            "application/json"
        );
    }

    #[test]
    fn test_response_generator_with_delay() {
        let response = ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}"))
            .with_delay(Duration::from_millis(100));

        assert_eq!(response.delay, Some(Duration::from_millis(100)));
    }

    #[tokio::test]
    async fn test_response_generator_generate_inline() {
        let response = ResponseGenerator::new(StatusCode::OK, BodySource::inline("test response"));

        let body = response.generate_static().await.unwrap();
        assert_eq!(body.as_ref(), b"test response");
    }

    #[test]
    fn test_response_generator_build_headers() {
        let response = ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}"))
            .with_header("content-type", "application/json")
            .with_header("x-custom", "value");

        let headers = response.build_headers().unwrap();
        assert_eq!(headers.len(), 2);
        assert_eq!(
            headers.get("content-type").unwrap(),
            HeaderValue::from_static("application/json")
        );
        assert_eq!(
            headers.get("x-custom").unwrap(),
            HeaderValue::from_static("value")
        );
    }

    #[test]
    fn test_request_matcher_default() {
        let matcher = RequestMatcher::default();
        assert!(matcher.methods.is_empty());
        assert!(matcher.url_patterns.is_empty());
        assert!(matcher.header_matchers.is_empty());
    }

    #[test]
    fn test_mock_definition_creation() {
        let mock = MockDefinition {
            id: "test-mock".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            request: RequestMatcher {
                methods: SmallVec::from_vec(vec![Method::GET]),
                url_patterns: SmallVec::from_vec(vec![UrlPattern::exact("/test")]),
                header_matchers: SmallVec::new(),
                query_matchers: SmallVec::new(),
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
            vars: None,
            streaming: None,
        };

        assert_eq!(mock.id, "test-mock");
        assert_eq!(mock.priority, 100);
        assert!(mock.enabled);
        assert_eq!(mock.request.methods.len(), 1);
    }

    #[test]
    fn test_jsonpath_array_index() {
        let json = serde_json::json!({
          "users": [
            {"name": "Alice", "role": "admin"},
            {"name": "Bob", "role": "user"}
          ]
        });

        // Array index access
        let matcher = BodyMatcher::json_path("$.users[0].name", serde_json::json!("Alice"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        let matcher = BodyMatcher::json_path("$.users[1].role", serde_json::json!("user"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        // Wrong value
        let matcher = BodyMatcher::json_path("$.users[0].name", serde_json::json!("Bob"));
        assert!(!matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        // Out of bounds
        let matcher = BodyMatcher::json_path("$.users[2].name", serde_json::json!("Charlie"));
        assert!(!matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));
    }

    #[test]
    fn test_jsonpath_nested_arrays() {
        let json = serde_json::json!({
          "data": {
            "items": [
              {
                "id": 1,
                "tags": ["rust", "programming"]
              },
              {
                "id": 2,
                "tags": ["javascript", "web"]
              }
            ]
          }
        });

        // Nested array access
        let matcher = BodyMatcher::json_path("$.data.items[0].tags[0]", serde_json::json!("rust"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        let matcher = BodyMatcher::json_path("$.data.items[1].tags[1]", serde_json::json!("web"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        // Access nested object in array
        let matcher = BodyMatcher::json_path("$.data.items[0].id", serde_json::json!(1));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));
    }

    #[test]
    fn test_jsonpath_deep_nesting() {
        let json = serde_json::json!({
          "response": {
            "users": [
              {
                "name": "Alice",
                "addresses": [
                  {"city": "New York", "zip": "10001"},
                  {"city": "Boston", "zip": "02101"}
                ]
              }
            ]
          }
        });

        // Deep nested path with multiple array indices
        let matcher = BodyMatcher::json_path(
            "$.response.users[0].addresses[0].city",
            serde_json::json!("New York"),
        );
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        let matcher = BodyMatcher::json_path(
            "$.response.users[0].addresses[1].zip",
            serde_json::json!("02101"),
        );
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));
    }

    #[test]
    fn test_jsonpath_without_dollar_prefix() {
        let json = serde_json::json!({
          "users": [
            {"name": "Alice"}
          ]
        });

        // Without $ prefix
        let matcher = BodyMatcher::json_path("users[0].name", serde_json::json!("Alice"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        // Without $. prefix
        let matcher = BodyMatcher::json_path("users[0].name", serde_json::json!("Alice"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));
    }

    #[test]
    fn test_jsonpath_array_of_primitives() {
        let json = serde_json::json!({
          "tags": ["rust", "mock", "testing"]
        });

        // Direct array access
        let matcher = BodyMatcher::json_path("$.tags[0]", serde_json::json!("rust"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        let matcher = BodyMatcher::json_path("$.tags[2]", serde_json::json!("testing"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));
    }

    #[test]
    fn test_jsonpath_consecutive_array_indices() {
        let json = serde_json::json!({
          "matrix": [
            [1, 2, 3],
            [4, 5, 6],
            [7, 8, 9]
          ]
        });

        // Consecutive array indices (2D array)
        let matcher = BodyMatcher::json_path("$.matrix[0][0]", serde_json::json!(1));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        let matcher = BodyMatcher::json_path("$.matrix[1][2]", serde_json::json!(6));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));

        let matcher = BodyMatcher::json_path("$.matrix[2][1]", serde_json::json!(8));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes(), None));
    }

    #[test]
    fn test_jsonpath_parse_segments() {
        // Test the segment parsing directly
        let segments = BodyMatcher::parse_jsonpath_segments("user.name");
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("user".to_string()),
                PathSegment::Key("name".to_string())
            ]
        );

        let segments = BodyMatcher::parse_jsonpath_segments("users[0].name");
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("users".to_string()),
                PathSegment::Index(0),
                PathSegment::Key("name".to_string())
            ]
        );

        let segments = BodyMatcher::parse_jsonpath_segments("data[1][2].value");
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("data".to_string()),
                PathSegment::Index(1),
                PathSegment::Index(2),
                PathSegment::Key("value".to_string())
            ]
        );

        let segments = BodyMatcher::parse_jsonpath_segments("items[0]");
        assert_eq!(
            segments,
            vec![PathSegment::Key("items".to_string()), PathSegment::Index(0)]
        );
    }
}
