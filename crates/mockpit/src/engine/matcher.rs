//! Mock matching engine for finding the appropriate mock for a request

use super::registry::MockRegistry;
use crate::engine::types::LeanString;
use crate::engine::types::{
    MockDefinition, PatchOperation, RequestPatch, ResponseGeneratorExt, UpstreamOptions,
};
use http::{HeaderMap, Method, StatusCode};
use lru::LruCache;
use nohash_hasher::BuildNoHashHasher;
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHasher};
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Result of a mock match, including the mock and any URL captures
#[derive(Debug, Clone)]
pub struct MockMatch {
    /// The matched mock definition (Arc'd for efficient cloning)
    pub mock: Arc<MockDefinition>,
    /// URL regex captures (name -> value)
    pub captures: FxHashMap<String, String>,
}

/// Action to take after mock matching
#[derive(Debug)]
pub enum MockAction {
    /// Return a complete mock response (no upstream call needed)
    FullMock(http::Response<bytes::Bytes>),
    /// Fetch from upstream with optional transforms, then patch response
    PatchUpstream {
        /// Response patches (jsonpath, regex, json_patch, headers)
        response_patches: Vec<PatchOperation>,
        /// Request patches (headers, query, body)
        request_patches: Vec<RequestPatch>,
        /// Delay before forwarding to upstream
        pre_delay: Option<std::time::Duration>,
        /// Delay after receiving upstream response
        post_delay: Option<std::time::Duration>,
        /// Override upstream status code (from response.status in patch mode)
        status_override: Option<http::StatusCode>,
        /// Upstream request options (timeout, forward_to)
        upstream_options: UpstreamOptions,
        /// Rewrite the request path (already template-rendered)
        rewrite_path: Option<String>,
        /// Mock ID for tracking
        mock_id: LeanString,
        /// URL captures from pattern matching (for template rendering in patches)
        captures: FxHashMap<String, String>,
        /// Mock-level variables (for template rendering in patches)
        vars: Option<serde_json::Map<String, serde_json::Value>>,
    },
}

/// Calculate cache hash directly from method and path bytes.
/// Avoids String allocation by hashing the borrowed data directly.
#[inline]
fn cache_hash(method: &Method, path: &str) -> u64 {
    let mut hasher = FxHasher::default();
    method.as_str().hash(&mut hasher);
    0u8.hash(&mut hasher);
    path.hash(&mut hasher);
    hasher.finish()
}

/// Mock matcher for finding matching mocks based on request criteria
#[derive(Clone)]
pub struct MockMatcher {
    registry: MockRegistry,
    /// LRU cache for matched mocks (method+path -> mock_id)
    /// Cache stores mock IDs rather than full MockDefinitions to save memory.
    /// Uses nohash-hasher since keys are pre-hashed u64 values.
    cache: Arc<Mutex<LruCache<u64, LeanString, BuildNoHashHasher<u64>>>>,
}

impl MockMatcher {
    /// Create a new mock matcher with the given registry
    /// Cache size defaults to 1000 entries
    pub fn new(registry: MockRegistry) -> Self {
        Self::with_cache_size(registry, 1000)
    }

    /// Create a new mock matcher with a specific cache size
    pub fn with_cache_size(registry: MockRegistry, cache_size: usize) -> Self {
        let cache = LruCache::with_hasher(
            NonZeroUsize::new(cache_size).unwrap_or(NonZeroUsize::MIN),
            BuildNoHashHasher::default(),
        );
        Self {
            registry,
            cache: Arc::new(Mutex::new(cache)),
        }
    }

    /// Clear the cache (useful when mocks are added/removed/modified)
    pub fn clear_cache(&self) {
        self.cache.lock().clear();
    }

    /// Find a matching mock for the given request
    ///
    /// This implements the full matching flow:
    /// 1. Try the exact-match index for O(1) lookup (simple mocks, no conditionals)
    /// 2. Check LRU cache (method+path -> mock_id)
    /// 3. Fall back to linear scan through all mocks
    /// 4. Cache the result for future lookups
    pub fn find_match(
        &self,
        method: &Method,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) -> Option<MockMatch> {
        // Return None if the mock system is disabled
        if !self.registry.is_enabled() {
            return None;
        }

        // FAST PATH 1: Exact-match index for simple requests (no query/body/conditional mocks)
        // This is O(1) and avoids all lock contention on the LRU cache.
        if query.is_none()
            && body.is_none()
            && !self.registry.has_conditional_mocks()
            && let Some(mock) = self.registry.try_exact_match(method, path)
            && mock.enabled
        {
            self.record_call_if_needed(&mock, method, path, query, headers, body);
            let captures = self.extract_url_captures(&mock, path);
            return Some(MockMatch { mock, captures });
        }

        // Cache eligibility: we can cache results for requests without query/body.
        // The mock-level cacheability check ensures we only cache mocks without
        // header/body/query matchers, so it's safe to cache even when headers are present.
        let can_use_cache = query.is_none() && body.is_none();

        if can_use_cache {
            let hash = cache_hash(method, path);

            // Try to get from cache (parking_lot::Mutex - no poisoning, faster)
            {
                let mut cache = self.cache.lock();
                if let Some(mock_id) = cache.get(&hash) {
                    // Cache hit! Retrieve the mock by ID and verify it's still cacheable
                    if let Some(mock) = self.registry.get_mock(mock_id.as_str()) {
                        let is_cacheable = mock.request.query_matchers.is_empty()
                            && mock.request.header_matchers.is_empty()
                            && mock.request.body_matcher.is_none();

                        if is_cacheable {
                            self.record_call_if_needed(&mock, method, path, query, headers, body);
                            let captures = self.extract_url_captures(&mock, path);
                            return Some(MockMatch { mock, captures });
                        }
                        // Mock is no longer cacheable (was modified), invalidate
                        cache.pop(&hash);
                    } else {
                        cache.pop(&hash);
                    }
                }
            } // Mutex dropped here
        }

        // SLOW PATH: Linear scan through all enabled mocks
        let all_mocks = self.registry.get_enabled_mocks();

        // Pre-parse query params once for all matchers (optimization)
        let parsed_query = if query.is_some()
            && all_mocks
                .iter()
                .any(|m| !m.request.query_matchers.is_empty())
        {
            Some(crate::types::QueryMatcher::parse_query(query))
        } else {
            None
        };

        // Find the first mock that matches all criteria (cheapest checks first)
        let matched_mock = all_mocks.into_iter().find(|mock| {
            self.matches_method(mock, method)
                && self.matches_url(mock, path, query)
                && self.matches_headers(mock, headers)
                && self.matches_query(mock, query, parsed_query.as_ref())
                && self.matches_graphql(mock, body)
                && self.matches_body(mock, body)
        });

        if let Some(mock) = matched_mock {
            self.record_call_if_needed(&mock, method, path, query, headers, body);
            let captures = self.extract_url_captures(&mock, path);

            // Cache the result if eligible
            if can_use_cache {
                let is_cacheable = mock.request.query_matchers.is_empty()
                    && mock.request.header_matchers.is_empty()
                    && mock.request.body_matcher.is_none();

                if is_cacheable {
                    let hash = cache_hash(method, path);
                    self.cache.lock().put(hash, mock.id.clone());
                }
            }

            Some(MockMatch { mock, captures })
        } else {
            None
        }
    }

    /// Record a call to the mock if call tracking is enabled.
    /// Extracted to avoid duplicating tracking code across fast/slow paths.
    #[inline]
    fn record_call_if_needed(
        &self,
        mock: &Arc<MockDefinition>,
        method: &Method,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) {
        if self.registry.is_call_tracking_enabled(mock.id.as_str()) {
            const MAX_TRACKED_HEADERS: usize = 10;
            let headers_map: FxHashMap<String, String> = headers
                .iter()
                .take(MAX_TRACKED_HEADERS)
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v_str| (k.as_str().to_string(), v_str.to_string()))
                })
                .collect();

            let call = super::registry::MockCall::new(
                method.to_string(),
                path.to_string(),
                query.map(std::string::ToString::to_string),
                headers_map,
                body,
            );

            self.registry.record_call(mock.id.as_str(), call);
        }
    }

    /// Extract URL captures from the mock's URL patterns
    #[allow(clippy::unused_self)]
    fn extract_url_captures(&self, mock: &MockDefinition, path: &str) -> FxHashMap<String, String> {
        // Try each URL pattern until we find one with captures
        for pattern in &mock.request.url_patterns {
            if let Some(captures) = pattern.extract_captures(path) {
                return captures;
            }
        }
        FxHashMap::default()
    }

    /// Check if the mock's method criteria matches the request method
    #[allow(clippy::unused_self)]
    fn matches_method(&self, mock: &MockDefinition, method: &Method) -> bool {
        // Empty methods list means match all methods
        if mock.request.methods.is_empty() {
            return true;
        }

        mock.request.methods.contains(method)
    }

    /// Check if the mock's URL patterns match the request path
    #[allow(clippy::unused_self)]
    fn matches_url(&self, mock: &MockDefinition, path: &str, query: Option<&str>) -> bool {
        // Empty URL patterns means match all URLs
        if mock.request.url_patterns.is_empty() {
            return true;
        }

        // Fast path: try matching just the path first (avoids allocation)
        // Most patterns match against path only.
        if mock
            .request
            .url_patterns
            .iter()
            .any(|pattern| pattern.matches(path))
        {
            return true;
        }

        // Slow path: if there's a query string, try matching full URL (path?query)
        // This is needed for exact-match patterns from recordings that include query params.
        // Only allocate the format string if the fast path didn't match.
        if let Some(q) = query {
            let full_url = format!("{path}?{q}");
            return mock
                .request
                .url_patterns
                .iter()
                .any(|pattern| pattern.matches(&full_url));
        }

        false
    }

    /// Check if the mock's header matchers match the request headers
    #[allow(clippy::unused_self)]
    fn matches_headers(&self, mock: &MockDefinition, headers: &HeaderMap) -> bool {
        // Empty header matchers means match all
        if mock.request.header_matchers.is_empty() {
            return true;
        }

        // All header matchers must match
        mock.request
            .header_matchers
            .iter()
            .all(|matcher| matcher.matches(headers))
    }

    /// Check if the mock's GraphQL matcher matches the request
    fn matches_graphql(&self, mock: &MockDefinition, body: Option<&[u8]>) -> bool {
        // If no GraphQL matcher is specified, match all
        let Some(graphql_matcher) = &mock.request.graphql_matcher else {
            return true;
        };

        // If GraphQL matcher is specified but no body provided, no match
        let Some(body_bytes) = body else {
            return false;
        };

        // Parse body as JSON
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(body_bytes) else {
            return false;
        };

        // Wildcard match - any GraphQL operation
        if graphql_matcher.match_any {
            return json.get("query").is_some() || json.get("operationName").is_some();
        }

        // Check introspection matcher if specified
        if let Some(introspection_matcher) = &graphql_matcher.introspection_matcher
            && !self.matches_introspection(introspection_matcher, &json)
        {
            return false;
        }

        // Check operation name if specified
        if let Some(expected_name) = &graphql_matcher.operation_name {
            let actual_name = json.get("operationName").and_then(|v| v.as_str());

            if actual_name != Some(expected_name.as_str()) {
                return false;
            }
        }

        // Check operation type if specified
        if let Some(expected_type) = graphql_matcher.operation_type {
            let query = json.get("query").and_then(|v| v.as_str());

            if let Some(q) = query {
                let detected_type = Self::detect_operation_type(q);
                match detected_type {
                    Some(detected) if detected == expected_type => {
                        // Type matches, continue
                    }
                    _ => return false,
                }
            } else {
                // No query string, can't verify type
                return false;
            }
        }

        // Check variable matchers if specified
        if !graphql_matcher.variable_matchers.is_empty() {
            let variables = json.get("variables");

            for (path, expected_value) in &graphql_matcher.variable_matchers {
                // Parse path (supports nested like "input.role")
                let actual_value = Self::get_nested_json_value(variables, path);

                if actual_value != Some(expected_value) {
                    return false;
                }
            }
        }

        true
    }

    /// Match introspection queries
    #[allow(clippy::unused_self)]
    fn matches_introspection(
        &self,
        matcher: &crate::types::IntrospectionMatcher,
        json: &serde_json::Value,
    ) -> bool {
        // Get the query string
        let Some(query) = json.get("query").and_then(|v| v.as_str()) else {
            return false;
        };

        // Check if query contains introspection fields
        match matcher {
            crate::types::IntrospectionMatcher::Any => {
                // Match any introspection query (__schema, __type, __typename)
                query.contains("__schema")
                    || query.contains("__type")
                    || query.contains("__typename")
            }
            crate::types::IntrospectionMatcher::Schema => {
                // Match only __schema queries
                query.contains("__schema")
            }
            crate::types::IntrospectionMatcher::Type => {
                // Match only __type queries (not __typename)
                query.contains("__type") && !query.contains("__typename")
            }
            crate::types::IntrospectionMatcher::TypeName => {
                // Match only __typename queries
                query.contains("__typename")
            }
        }
    }

    /// Helper to get nested JSON value by path
    /// Supports paths like "id", "input.role", "user.address.city"
    fn get_nested_json_value<'a>(
        json: Option<&'a serde_json::Value>,
        path: &str,
    ) -> Option<&'a serde_json::Value> {
        let mut current = json?;

        for segment in path.split('.') {
            current = current.get(segment)?;
        }

        Some(current)
    }

    /// Simple operation type detection without full parsing
    /// Uses string prefix matching for fast detection
    fn detect_operation_type(query: &str) -> Option<crate::types::GraphQLOperationType> {
        let trimmed = query.trim();

        // Check for explicit type keywords
        if trimmed.starts_with("query ") || trimmed.starts_with("query{") {
            return Some(crate::types::GraphQLOperationType::Query);
        }

        if trimmed.starts_with("mutation ") || trimmed.starts_with("mutation{") {
            return Some(crate::types::GraphQLOperationType::Mutation);
        }

        if trimmed.starts_with("subscription ") || trimmed.starts_with("subscription{") {
            return Some(crate::types::GraphQLOperationType::Subscription);
        }

        // Unnamed operation starting with '{' is assumed to be a query
        if trimmed.starts_with('{') {
            return Some(crate::types::GraphQLOperationType::Query);
        }

        None
    }

    /// Check if the mock's body matcher matches the request body
    #[allow(clippy::unused_self)]
    fn matches_body(&self, mock: &MockDefinition, body: Option<&[u8]>) -> bool {
        // If no body matcher is specified, match all
        let Some(body_matcher) = &mock.request.body_matcher else {
            return true;
        };

        // If body matcher is specified but no body provided, no match
        let Some(body_bytes) = body else {
            return false;
        };

        // Check if body matches
        body_matcher.matches(body_bytes)
    }

    /// Check if the mock's query matchers match
    /// Uses pre-parsed query params to avoid re-parsing for each matcher
    #[allow(clippy::unused_self)]
    fn matches_query(
        &self,
        mock: &MockDefinition,
        query: Option<&str>,
        parsed_query: Option<&rustc_hash::FxHashMap<String, String>>,
    ) -> bool {
        // Empty query matchers means match all
        if mock.request.query_matchers.is_empty() {
            return true;
        }

        // Parse query once if not already parsed
        let owned_params;
        let query_params = if let Some(params) = parsed_query {
            params
        } else {
            owned_params = crate::types::QueryMatcher::parse_query(query);
            &owned_params
        };

        // All query matchers must match using pre-parsed params
        mock.request
            .query_matchers
            .iter()
            .all(|matcher| matcher.matches_parsed(query_params))
    }

    /// Get a reference to the registry
    pub fn registry(&self) -> &MockRegistry {
        &self.registry
    }

    /// Try to match a request and determine the appropriate action (optimized version)
    ///
    /// This is an optimized entry point that takes references instead of ownership,
    /// avoiding unnecessary clones in the proxy hot path. Use this when you already
    /// have the request parts decomposed.
    ///
    /// Returns None if no mock matches, or Some(MockAction) indicating what to do:
    /// - FullMock: Complete response ready to return (no upstream call needed)
    /// - PatchUpstream: Instructions to fetch from upstream and apply patches
    ///
    /// Automatically adds X-Mock-Id header
    pub async fn try_match_parts(
        &self,
        method: &Method,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) -> Option<MockAction> {
        use bytes::Bytes;
        use http::Response;
        use http::header::HeaderValue;

        // Return None if the mock system is disabled
        if !self.registry.is_enabled() {
            return None;
        }

        // Find a matching mock using the existing find_match logic
        let mock_match = self.find_match(method, path, query, headers, body)?;

        let mock_def = &mock_match.mock;
        let captures = mock_match.captures;

        // Check if this is a passthrough mock (has request transforms or response patches)
        let has_request_transforms = mock_def.request_transforms.is_some();
        let has_response_patches = matches!(
            &mock_def.response.mode,
            crate::types::ResponseMode::Patch { .. }
        );

        if has_request_transforms || has_response_patches {
            let response_patches = match &mock_def.response.mode {
                crate::types::ResponseMode::Patch { operations } => operations.clone(),
                _ => vec![],
            };

            let (request_patches, pre_delay, upstream_options, rewrite_path) =
                if let Some(rt) = &mock_def.request_transforms {
                    // TODO: Template rendering for rewrite_path and header/query values
                    // will be added later. For now, pass through as-is.
                    (
                        rt.patches.clone(),
                        rt.pre_delay,
                        rt.upstream_options.clone(),
                        rt.rewrite_path.clone(),
                    )
                } else {
                    (vec![], None, UpstreamOptions::default(), None)
                };

            // post_delay comes from response.delay in PatchUpstream mode
            let post_delay = mock_def.response.delay;

            return Some(MockAction::PatchUpstream {
                response_patches,
                request_patches,
                pre_delay,
                post_delay,
                status_override: None, // TODO: will be set from config in future
                upstream_options,
                rewrite_path,
                mock_id: mock_def.id.clone(),
                captures,
                vars: mock_def.vars.clone(),
            });
        }

        // For non-patch mocks, generate a full response
        let response_generator = &mock_def.response;

        // Generate the response (may include dynamic status/headers from templates)
        let dynamic_result = if matches!(
            response_generator.body,
            crate::types::BodySource::Template { .. }
        ) {
            // For templates, use generate_dynamic which returns DynamicResponse
            response_generator
                .generate_dynamic(
                    method.as_str(),
                    path,
                    query,
                    headers,
                    body,
                    captures,
                    mock_def.vars.as_ref(),
                )
                .await
        } else {
            // For static content, wrap in DynamicResponse with no overrides
            response_generator
                .generate()
                .await
                .map(crate::types::DynamicResponse::body_only)
        };

        // Handle response generation errors
        let dynamic_response = match dynamic_result {
            Ok(resp) => resp,
            Err(e) => {
                // Template rendering error - return 500 with error message
                let error_message =
                    format!("Mock '{}' failed to generate response: {}", mock_def.id, e);
                tracing::error!("{}", error_message);

                let error_body = serde_json::json!({
                  "error": "Mock Response Generation Failed",
                  "mock_id": mock_def.id,
                  "details": format!("{}", e)
                });

                let error_response = match Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("Content-Type", "application/json")
                    .header("X-Mock-Id", mock_def.id.as_str())
                    .header("X-Mock-Error", "true")
                    .body(Bytes::from(error_body.to_string()))
                {
                    Ok(resp) => resp,
                    Err(build_err) => {
                        tracing::error!("Failed to build error response: {build_err}");
                        return None;
                    }
                };

                return Some(MockAction::FullMock(error_response));
            }
        };

        // Use dynamic status if provided, otherwise use mock definition's status
        let final_status = dynamic_response.status.unwrap_or(response_generator.status);

        // Build final response with dynamic status
        let mut response = Response::builder().status(final_status);

        // Add mock definition headers first (as defaults)
        for (key, value) in &response_generator.headers {
            if let Ok(header_value) = HeaderValue::from_str(value) {
                response = response.header(key.as_str(), header_value);
            }
        }

        // Then add/override with dynamic headers from script/template
        if let Some(dynamic_headers) = &dynamic_response.headers {
            for (key, value) in dynamic_headers {
                if let Ok(header_value) = HeaderValue::from_str(value) {
                    response = response.header(key.as_str(), header_value);
                }
            }
        }

        // Always add X-Mock-Id header so the proxy can log it if needed
        response = response.header("X-Mock-Id", mock_def.id.as_str());

        let final_response = response.body(dynamic_response.body).ok()?;
        Some(MockAction::FullMock(final_response))
    }

    /// Apply patches to an upstream response
    ///
    /// This is a convenience method that takes an upstream response and applies
    /// the provided patch operations, returning the patched response with the mock ID header.
    ///
    /// If `request_context` is provided, template expressions in patch values (e.g.
    /// `{{ captures.id }}`, `{{ response.body_json.name }}`) will be rendered with
    /// both request and upstream response data.
    pub fn apply_patches(
        patches: Vec<PatchOperation>,
        mock_id: impl AsRef<str>,
        upstream_response: http::Response<bytes::Bytes>,
        request_context: Option<crate::types::RequestContext>,
    ) -> Result<http::Response<bytes::Bytes>, anyhow::Error> {
        use http::Response;

        let (upstream_parts, upstream_bytes) = upstream_response.into_parts();

        // Build PatchContext if request_context is provided (for template rendering)
        let patch_context = request_context.map(|req_ctx| {
            let response_headers: FxHashMap<String, String> = upstream_parts
                .headers
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str()
                        .ok()
                        .map(|v_str| (k.to_string(), v_str.to_string()))
                })
                .collect();
            let response_body_json = serde_json::from_slice(&upstream_bytes).ok();
            crate::types::PatchContext {
                request: req_ctx,
                response_status: upstream_parts.status.as_u16(),
                response_headers,
                response_body_json,
            }
        });

        let patchable_response = Response::from_parts(upstream_parts, upstream_bytes);

        // Apply patches to the upstream response
        let patcher = super::patcher::ResponsePatcher::new(patches);
        let patched_response = patcher.apply(patchable_response, patch_context.as_ref())?;

        // Extract patched components and add mock ID header
        let (patched_parts, patched_body) = patched_response.into_parts();

        let mut final_builder = Response::builder().status(patched_parts.status);
        // Copy headers but skip Content-Length (we'll set it correctly below)
        for (key, value) in &patched_parts.headers {
            if key != http::header::CONTENT_LENGTH {
                final_builder = final_builder.header(key, value);
            }
        }

        // Set correct Content-Length based on patched body
        final_builder = final_builder
            .header("X-Mock-Id", mock_id.as_ref())
            .header(http::header::CONTENT_LENGTH, patched_body.len());

        Ok(final_builder.body(patched_body)?)
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
    use crate::engine::types::{
        BodySource, HeaderMatcher, RequestMatcher, ResponseGenerator, UrlPattern,
    };
    use http::StatusCode;
    use http::header::{HeaderName, HeaderValue};
    use smallvec::smallvec;

    fn create_mock(
        id: &str,
        priority: u32,
        methods: Vec<Method>,
        url_patterns: Vec<UrlPattern>,
        header_matchers: Vec<HeaderMatcher>,
    ) -> MockDefinition {
        MockDefinition {
            id: id.into(),
            priority,
            enabled: true,
            scope: None,
            source_file: None,
            request_transforms: None,
            request: RequestMatcher {
                methods: methods.into(),
                url_patterns: url_patterns.into(),
                header_matchers: header_matchers.into(),
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
            vars: None,
        }
    }

    #[test]
    fn test_matcher_creation() {
        let registry = MockRegistry::new();
        let matcher = MockMatcher::new(registry);
        assert!(matcher.registry().is_enabled());
    }

    #[test]
    fn test_no_match_when_disabled() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::exact("/test")],
            vec![],
        ));

        registry.disable();

        let matcher = MockMatcher::new(registry);
        let result = matcher.find_match(&Method::GET, "/test", None, &HeaderMap::new(), None);

        assert!(result.is_none());
    }

    #[test]
    fn test_match_any_method() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![], // Empty = match all methods
            vec![UrlPattern::exact("/test")],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        // Should match any method
        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::POST, "/test", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::PUT, "/test", None, &HeaderMap::new(), None)
                .is_some()
        );
    }

    #[test]
    fn test_match_specific_method() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![Method::GET, Method::POST],
            vec![UrlPattern::exact("/test")],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        // Should match GET and POST
        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::POST, "/test", None, &HeaderMap::new(), None)
                .is_some()
        );

        // Should not match PUT
        assert!(
            matcher
                .find_match(&Method::PUT, "/test", None, &HeaderMap::new(), None)
                .is_none()
        );
    }

    #[test]
    fn test_match_exact_url() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::exact("/api/users")],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        assert!(
            matcher
                .find_match(&Method::GET, "/api/users", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/users/123",
                    None,
                    &HeaderMap::new(),
                    None
                )
                .is_none()
        );
    }

    #[test]
    fn test_match_prefix_url() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::prefix("/api/")],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        assert!(
            matcher
                .find_match(&Method::GET, "/api/users", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::GET, "/api/files", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::GET, "/v2/api/users", None, &HeaderMap::new(), None)
                .is_none()
        );
    }

    #[test]
    fn test_match_regex_url() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::regex(r"^/api/users/\d+$").unwrap()],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/users/123",
                    None,
                    &HeaderMap::new(),
                    None
                )
                .is_some()
        );
        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/users/456",
                    None,
                    &HeaderMap::new(),
                    None
                )
                .is_some()
        );
        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/users/abc",
                    None,
                    &HeaderMap::new(),
                    None
                )
                .is_none()
        );
    }

    #[test]
    fn test_match_multiple_url_patterns() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![
                UrlPattern::exact("/api/users"),
                UrlPattern::exact("/api/files"),
            ],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        assert!(
            matcher
                .find_match(&Method::GET, "/api/users", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::GET, "/api/files", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::GET, "/api/other", None, &HeaderMap::new(), None)
                .is_none()
        );
    }

    #[test]
    fn test_match_headers() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::exact("/test")],
            vec![HeaderMatcher::exact(
                HeaderName::from_static("content-type"),
                "application/json",
            )],
        ));

        let matcher = MockMatcher::new(registry);

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &headers, None)
                .is_some()
        );

        // Different content-type should not match
        let mut headers2 = HeaderMap::new();
        headers2.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("text/plain"),
        );

        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &headers2, None)
                .is_none()
        );
    }

    #[test]
    fn test_match_header_presence() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::exact("/test")],
            vec![HeaderMatcher::present(HeaderName::from_static(
                "authorization",
            ))],
        ));

        let matcher = MockMatcher::new(registry);

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer token"),
        );

        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &headers, None)
                .is_some()
        );

        // Without authorization header should not match
        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &HeaderMap::new(), None)
                .is_none()
        );
    }

    #[test]
    fn test_priority_ordering() {
        let registry = MockRegistry::new();

        // Add mocks with different priorities
        registry.add_mock(create_mock(
            "low",
            10,
            vec![],
            vec![UrlPattern::prefix("/api/")],
            vec![],
        ));

        registry.add_mock(create_mock(
            "high",
            100,
            vec![],
            vec![UrlPattern::prefix("/api/")],
            vec![],
        ));

        registry.add_mock(create_mock(
            "medium",
            50,
            vec![],
            vec![UrlPattern::prefix("/api/")],
            vec![],
        ));

        let matcher = MockMatcher::new(registry);

        // Should return the highest priority match
        let result = matcher
            .find_match(&Method::GET, "/api/test", None, &HeaderMap::new(), None)
            .unwrap();
        assert_eq!(result.mock.id, "high");
        assert_eq!(result.mock.priority, 100);
    }

    #[test]
    fn test_combined_matching() {
        let registry = MockRegistry::new();

        // Add a mock with all criteria
        registry.add_mock(create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/api/users")],
            vec![HeaderMatcher::exact(
                HeaderName::from_static("content-type"),
                "application/json",
            )],
        ));

        let matcher = MockMatcher::new(registry);

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        // Should match when all criteria are met
        assert!(
            matcher
                .find_match(&Method::POST, "/api/users", None, &headers, None)
                .is_some()
        );

        // Should not match with wrong method
        assert!(
            matcher
                .find_match(&Method::GET, "/api/users", None, &headers, None)
                .is_none()
        );

        // Should not match with wrong path
        assert!(
            matcher
                .find_match(&Method::POST, "/api/files", None, &headers, None)
                .is_none()
        );

        // Should not match with wrong headers
        assert!(
            matcher
                .find_match(&Method::POST, "/api/users", None, &HeaderMap::new(), None)
                .is_none()
        );
    }

    #[test]
    fn test_empty_patterns_match_all() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock("test", 100, vec![], vec![], vec![]));

        let matcher = MockMatcher::new(registry);

        // Should match any request
        assert!(
            matcher
                .find_match(&Method::GET, "/any/path", None, &HeaderMap::new(), None)
                .is_some()
        );
        assert!(
            matcher
                .find_match(&Method::POST, "/other/path", None, &HeaderMap::new(), None)
                .is_some()
        );
    }

    #[test]
    fn test_disabled_mock_not_matched() {
        let registry = MockRegistry::new();
        let mut mock = create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::exact("/test")],
            vec![],
        );
        mock.enabled = false;
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &HeaderMap::new(), None)
                .is_none()
        );
    }

    #[test]
    fn test_multiple_header_matchers() {
        let registry = MockRegistry::new();
        registry.add_mock(create_mock(
            "test",
            100,
            vec![],
            vec![UrlPattern::exact("/test")],
            vec![
                HeaderMatcher::present(HeaderName::from_static("authorization")),
                HeaderMatcher::exact(HeaderName::from_static("content-type"), "application/json"),
            ],
        ));

        let matcher = MockMatcher::new(registry);

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer token"),
        );
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        // Should match when all headers match
        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &headers, None)
                .is_some()
        );

        // Should not match when one header is missing
        let mut partial_headers = HeaderMap::new();
        partial_headers.insert(
            HeaderName::from_static("authorization"),
            HeaderValue::from_static("Bearer token"),
        );

        assert!(
            matcher
                .find_match(&Method::GET, "/test", None, &partial_headers, None)
                .is_none()
        );
    }

    // GraphQL matching tests
    #[test]
    fn test_graphql_query_by_operation_name() {
        let registry = MockRegistry::new();

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: Some("GetUser".to_string()),
            operation_type: None,
            match_any: false,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: None,
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // GraphQL request matching operation name
        let graphql_body = r#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","operationName":"GetUser","variables":{"id":"123"}}"#;

        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(graphql_body.as_bytes())
                )
                .is_some()
        );

        // Different operation name should not match
        let other_body = r#"{"query":"query GetPosts { posts { title } }","operationName":"GetPosts","variables":{}}"#;

        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(other_body.as_bytes())
                )
                .is_none()
        );
    }

    #[test]
    fn test_graphql_query_by_type() {
        let registry = MockRegistry::new();

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: None,
            operation_type: Some(crate::types::GraphQLOperationType::Query),
            match_any: false,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: None,
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // Query should match
        let query_body = r#"{"query":"query GetUser { user { name } }","operationName":"GetUser"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(query_body.as_bytes())
                )
                .is_some()
        );

        // Mutation should not match
        let mutation_body = r#"{"query":"mutation CreateUser { createUser(name: \"John\") { id } }","operationName":"CreateUser"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(mutation_body.as_bytes())
                )
                .is_none()
        );
    }

    #[test]
    fn test_graphql_mutation_matching() {
        let registry = MockRegistry::new();

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: Some("CreateUser".to_string()),
            operation_type: Some(crate::types::GraphQLOperationType::Mutation),
            match_any: false,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: None,
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        let mutation_body = r#"{"query":"mutation CreateUser($name: String!) { createUser(name: $name) { id } }","operationName":"CreateUser","variables":{"name":"John"}}"#;

        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(mutation_body.as_bytes())
                )
                .is_some()
        );
    }

    #[test]
    fn test_graphql_variable_matching() {
        let registry = MockRegistry::new();

        let mut variables = FxHashMap::default();
        variables.insert("id".to_string(), serde_json::json!("123"));

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: Some("GetUser".to_string()),
            operation_type: None,
            match_any: false,
            variable_matchers: variables,
            introspection_matcher: None,
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // Matching variable value
        let body_match = r#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","operationName":"GetUser","variables":{"id":"123"}}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(body_match.as_bytes())
                )
                .is_some()
        );

        // Non-matching variable value
        let body_no_match = r#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","operationName":"GetUser","variables":{"id":"456"}}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(body_no_match.as_bytes())
                )
                .is_none()
        );
    }

    #[test]
    fn test_graphql_nested_variable_matching() {
        let registry = MockRegistry::new();

        let mut variables = FxHashMap::default();
        variables.insert("input.role".to_string(), serde_json::json!("admin"));

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: Some("CreateUser".to_string()),
            operation_type: Some(crate::types::GraphQLOperationType::Mutation),
            match_any: false,
            variable_matchers: variables,
            introspection_matcher: None,
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // Matching nested variable
        let body_match = r#"{"query":"mutation CreateUser($input: UserInput!) { createUser(input: $input) { id } }","operationName":"CreateUser","variables":{"input":{"name":"John","role":"admin"}}}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(body_match.as_bytes())
                )
                .is_some()
        );

        // Non-matching nested variable
        let body_no_match = r#"{"query":"mutation CreateUser($input: UserInput!) { createUser(input: $input) { id } }","operationName":"CreateUser","variables":{"input":{"name":"John","role":"user"}}}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(body_no_match.as_bytes())
                )
                .is_none()
        );
    }

    #[test]
    fn test_graphql_introspection_any() {
        let registry = MockRegistry::new();

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: None,
            operation_type: None,
            match_any: false,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: Some(crate::types::IntrospectionMatcher::Any),
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // __schema introspection
        let schema_body = r#"{"query":"query IntrospectionQuery { __schema { types { name } } }","operationName":"IntrospectionQuery"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(schema_body.as_bytes())
                )
                .is_some()
        );

        // __type introspection
        let type_body = r#"{"query":"query { __type(name: \"User\") { name fields { name } } }"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(type_body.as_bytes())
                )
                .is_some()
        );

        // Regular query should not match
        let regular_body =
            r#"{"query":"query GetUser { user { name } }","operationName":"GetUser"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(regular_body.as_bytes())
                )
                .is_none()
        );
    }

    #[test]
    fn test_graphql_introspection_schema_only() {
        let registry = MockRegistry::new();

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: None,
            operation_type: None,
            match_any: false,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: Some(crate::types::IntrospectionMatcher::Schema),
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // __schema should match
        let schema_body = r#"{"query":"query { __schema { types { name } } }"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(schema_body.as_bytes())
                )
                .is_some()
        );

        // __type should not match
        let type_body = r#"{"query":"query { __type(name: \"User\") { name } }"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(type_body.as_bytes())
                )
                .is_none()
        );
    }

    #[test]
    fn test_graphql_wildcard_match() {
        let registry = MockRegistry::new();

        let mut mock = create_mock(
            "test",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        mock.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: None,
            operation_type: None,
            match_any: true,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: None,
        });
        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // Any GraphQL query should match
        let query_body = r#"{"query":"query GetUser { user { name } }","operationName":"GetUser"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(query_body.as_bytes())
                )
                .is_some()
        );

        let mutation_body =
            r#"{"query":"mutation CreateUser { createUser { id } }","operationName":"CreateUser"}"#;
        assert!(
            matcher
                .find_match(
                    &Method::POST,
                    "/graphql",
                    None,
                    &HeaderMap::new(),
                    Some(mutation_body.as_bytes())
                )
                .is_some()
        );
    }

    #[test]
    fn test_graphql_priority_matching() {
        let registry = MockRegistry::new();

        // High priority mock for specific user ID
        let mut high_priority = create_mock(
            "high",
            200,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        let mut variables_high = FxHashMap::default();
        variables_high.insert("id".to_string(), serde_json::json!("999"));
        high_priority.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: Some("GetUser".to_string()),
            operation_type: None,
            match_any: false,
            variable_matchers: variables_high,
            introspection_matcher: None,
        });
        registry.add_mock(high_priority);

        // Low priority mock for any GetUser
        let mut low_priority = create_mock(
            "low",
            100,
            vec![Method::POST],
            vec![UrlPattern::exact("/graphql")],
            vec![],
        );
        low_priority.request.graphql_matcher = Some(crate::types::GraphQLMatcher {
            operation_name: Some("GetUser".to_string()),
            operation_type: None,
            match_any: false,
            variable_matchers: FxHashMap::default(),
            introspection_matcher: None,
        });
        registry.add_mock(low_priority);

        let matcher = MockMatcher::new(registry);

        // Request with id=999 should match high priority mock
        let high_priority_body = r#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","operationName":"GetUser","variables":{"id":"999"}}"#;
        let result = matcher
            .find_match(
                &Method::POST,
                "/graphql",
                None,
                &HeaderMap::new(),
                Some(high_priority_body.as_bytes()),
            )
            .unwrap();
        assert_eq!(result.mock.id, "high");

        // Request with different id should match low priority mock
        let low_priority_body = r#"{"query":"query GetUser($id: ID!) { user(id: $id) { name } }","operationName":"GetUser","variables":{"id":"123"}}"#;
        let result = matcher
            .find_match(
                &Method::POST,
                "/graphql",
                None,
                &HeaderMap::new(),
                Some(low_priority_body.as_bytes()),
            )
            .unwrap();
        assert_eq!(result.mock.id, "low");
    }
}
