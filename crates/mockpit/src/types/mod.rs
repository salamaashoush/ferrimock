//! Core types for the mock engine

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

/// Type alias for async handler functions that receive request context and produce dynamic responses.
///
/// This is the function signature used by programmatic mock handlers (MSW-style API).
/// Handlers receive the full [`RequestContext`] (method, path, captures, headers, body, etc.)
/// and return a [`DynamicResponse`] which can set status, headers, and body.
pub type HandlerFn = Arc<
    dyn Fn(
            RequestContext,
        ) -> Pin<Box<dyn Future<Output = Result<DynamicResponse, anyhow::Error>> + Send>>
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

    /// Create request context from HTTP components
    pub fn from_request(
        method: &str,
        uri: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
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

        // Extract headers
        let header_map: FxHashMap<_, _> = headers
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.to_string(), val.to_string())))
            .collect();

        // Validate UTF-8 in-place (no copy), then allocate String only if valid
        let body_str = body.and_then(|b| std::str::from_utf8(b).ok().map(String::from));

        // Parse body as JSON if possible
        let body_json = body_str.as_ref().and_then(|s| serde_json::from_str(s).ok());

        Self {
            method: method.to_string(),
            uri: uri.to_string(),
            path,
            query: query_params,
            captures: FxHashMap::default(), // Will be populated by matcher
            headers: header_map,
            body: body_str,
            body_json,
            vars: None, // Will be populated from mock definition's cascaded vars
        }
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
                    captures.insert(name.to_string(), value.as_str().to_string());
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
    /// Glob pattern match
    Glob(GlobMatcher),
}

impl UrlPattern {
    /// Check if the pattern matches the given path
    pub fn matches(&self, path: &str) -> bool {
        match self {
            UrlPattern::Exact(s) => path == s,
            UrlPattern::Prefix(s) => path.starts_with(s),
            UrlPattern::Suffix(s) => path.ends_with(s),
            UrlPattern::Regex(re) => re.is_match(path),
            UrlPattern::Glob(g) => g.is_match(path),
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
                            captures.insert(name.to_string(), value.as_str().to_string());
                        }
                    }

                    if !captures.is_empty() {
                        return Some(captures);
                    }
                }
                None
            }
            // Other pattern types don't support captures
            UrlPattern::Exact(_)
            | UrlPattern::Prefix(_)
            | UrlPattern::Suffix(_)
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
    /// - `*` as a full segment — wildcard (matches everything)
    /// - Literal segments — escaped for regex safety
    ///
    /// # Examples
    /// ```
    /// # use mockpit::types::UrlPattern;
    /// let pattern = UrlPattern::path_pattern("/users/:id").unwrap();
    /// assert!(pattern.matches("/users/123"));
    /// assert!(!pattern.matches("/users/123/extra"));
    /// ```
    pub fn path_pattern(pattern: &str) -> Result<Self, regex::Error> {
        let regex_str = pattern
            .split('/')
            .map(|segment| {
                if let Some(param) = segment.strip_prefix(':') {
                    format!("(?P<{param}>[^/]+)")
                } else if segment == "*" {
                    ".*".to_string()
                } else {
                    regex::escape(segment)
                }
            })
            .collect::<Vec<_>>()
            .join("/");

        UrlPattern::regex(&format!("^{regex_str}$"))
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

    /// Check if the matcher matches the given body
    pub fn matches(&self, body: &[u8]) -> bool {
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
            BodyMatcher::JsonPath { path, value } => {
                // Parse body as JSON
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
                    // Use simple JSONPath-like matching
                    Self::json_path_match(&json, path, value)
                } else {
                    false
                }
            }
            BodyMatcher::JsonEquals(expected) => {
                if let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) {
                    &json == expected
                } else {
                    false
                }
            }
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
        // Remove leading $. or $ if present
        let path = path.strip_prefix("$.").unwrap_or(path);
        let path = path.strip_prefix('$').unwrap_or(path);

        // Parse path segments (handles both dots and array indices)
        let segments = Self::parse_jsonpath_segments(path);

        // Traverse JSON following the path
        let mut current = json;
        for segment in segments {
            match segment {
                PathSegment::Key(key) => {
                    // Object property access
                    if let Some(value) = current.get(key) {
                        current = value;
                    } else {
                        return false;
                    }
                }
                PathSegment::Index(idx) => {
                    // Array index access
                    if let Some(array) = current.as_array() {
                        if let Some(value) = array.get(idx) {
                            current = value;
                        } else {
                            return false; // Index out of bounds
                        }
                    } else {
                        return false; // Not an array
                    }
                }
            }
        }

        current == expected
    }

    /// Parse JSONPath segments handling both object keys and array indices
    ///
    /// Examples:
    /// - `"user.name"` -> `[Key("user"), Key("name")]`
    /// - `"users[0].name"` -> `[Key("users"), Index(0), Key("name")]`
    /// - `"data[1][2].value"` -> `[Key("data"), Index(1), Index(2), Key("value")]`
    fn parse_jsonpath_segments(path: &str) -> Vec<PathSegment> {
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
enum PathSegment {
    /// Object key access
    Key(String),
    /// Array index access
    Index(usize),
}

/// GraphQL operation matcher
#[derive(Debug, Clone)]
pub struct GraphQLMatcher {
    /// Operation name (e.g., "GetUser")
    pub operation_name: Option<String>,
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
#[derive(Debug, Clone)]
pub struct DynamicResponse {
    /// HTTP status code (overrides mock definition if present)
    pub status: Option<StatusCode>,
    /// Additional response headers (merged with mock definition headers)
    pub headers: Option<FxHashMap<String, String>>,
    /// Response body bytes
    pub body: bytes::Bytes,
}

impl DynamicResponse {
    /// Create a new dynamic response with just a body (uses mock defaults for status/headers)
    pub fn body_only(body: bytes::Bytes) -> Self {
        Self {
            status: None,
            headers: None,
            body,
        }
    }

    /// Parse a JSON value into a DynamicResponse
    ///
    /// This is the unified parsing logic used by both templates and scripts.
    /// It checks for structured response format: { status?, headers?, body }
    /// - If the response has status/headers/body fields, parse them
    /// - If not, use the entire JSON as the body
    pub fn from_json(json: &serde_json::Value) -> Result<Self, anyhow::Error> {
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
}

impl ResponseGenerator {
    /// Create a new response generator with the given status and body
    pub fn new(status: StatusCode, body: BodySource) -> Self {
        let structured_response = body.may_produce_structured_response();
        Self {
            status,
            headers: FxHashMap::default(),
            body,
            delay: None,
            mode: ResponseMode::default(),
            structured_response,
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
        self.body = body;
    }

    /// Generate the response body for non-template sources only
    ///
    /// Note: Template rendering is not available in bdg-mock-types.
    /// Use bdg-mock-template or bdg-mock-engine for template support.
    pub async fn generate_static(&self) -> Result<bytes::Bytes, anyhow::Error> {
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
            BodySource::Template { .. } => Err(anyhow::anyhow!(
                "Template rendering not available in bdg-mock-types. Use bdg-mock-template or bdg-mock-engine."
            )),
            BodySource::Handler(_) => Err(anyhow::anyhow!(
                "Handler-based responses require generate_dynamic(). Use the engine's ResponseGeneratorExt."
            )),
        }
    }

    /// Build header map from the configured headers
    pub fn build_headers(&self) -> Result<HeaderMap, anyhow::Error> {
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
        let hash = Self::compute_hash(&source);
        BodySource::Template { source, hash }
    }

    /// Create a handler body source from a function
    pub fn handler(f: HandlerFn) -> Self {
        BodySource::Handler(f)
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

    /// Compute FxHash for a template string
    fn compute_hash(template: &str) -> u64 {
        use rustc_hash::FxHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = FxHasher::default();
        template.hash(&mut hasher);
        hasher.finish()
    }
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
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        let matcher = BodyMatcher::json_path("$.users[1].role", serde_json::json!("user"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        // Wrong value
        let matcher = BodyMatcher::json_path("$.users[0].name", serde_json::json!("Bob"));
        assert!(!matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        // Out of bounds
        let matcher = BodyMatcher::json_path("$.users[2].name", serde_json::json!("Charlie"));
        assert!(!matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));
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
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        let matcher = BodyMatcher::json_path("$.data.items[1].tags[1]", serde_json::json!("web"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        // Access nested object in array
        let matcher = BodyMatcher::json_path("$.data.items[0].id", serde_json::json!(1));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));
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
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        let matcher = BodyMatcher::json_path(
            "$.response.users[0].addresses[1].zip",
            serde_json::json!("02101"),
        );
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));
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
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        // Without $. prefix
        let matcher = BodyMatcher::json_path("users[0].name", serde_json::json!("Alice"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));
    }

    #[test]
    fn test_jsonpath_array_of_primitives() {
        let json = serde_json::json!({
          "tags": ["rust", "mock", "testing"]
        });

        // Direct array access
        let matcher = BodyMatcher::json_path("$.tags[0]", serde_json::json!("rust"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        let matcher = BodyMatcher::json_path("$.tags[2]", serde_json::json!("testing"));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));
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
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        let matcher = BodyMatcher::json_path("$.matrix[1][2]", serde_json::json!(6));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));

        let matcher = BodyMatcher::json_path("$.matrix[2][1]", serde_json::json!(8));
        assert!(matcher.matches(serde_json::to_string(&json).unwrap().as_bytes()));
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
