//! Ergonomic builder API for creating handler-based mock definitions.
//!
//! Provides an MSW-style API where handler functions are just another way to define mocks.
//! Handlers produce `MockDefinition`s that go into the same `MockRegistry` as declarative mocks.
//!
//! # Examples
//!
//! ```rust,ignore
//! use ferrimock::handler::{http, HttpResponse};
//! use ferrimock::prelude::*;
//!
//! let registry = MockRegistry::new();
//!
//! // Handler-based mock
//! registry.add_mock(http::get("/users/:id", |ctx| async move {
//!     let id = ctx.captures.get("id").unwrap();
//!     Ok(HttpResponse::json(&serde_json::json!({ "id": id }))?)
//! }));
//!
//! // Works alongside declarative mocks in the same registry
//! ```

use crate::types::{
    BodySource, DynamicResponse, HandlerFn, MockDefinition, RequestContext, RequestMatcher,
    ResponseGenerator, UrlPattern,
};
use ::http::{Method, StatusCode};
use lean_string::LeanString;
use smallvec::SmallVec;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for generating unique handler mock IDs.
static HANDLER_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique mock ID for a handler-based mock.
fn next_handler_id(prefix: &str) -> LeanString {
    let id = HANDLER_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    LeanString::from(format!("handler:{prefix}:{id}"))
}

/// Trait for converting closures into [`HandlerFn`].
///
/// Automatically implemented for async closures with the signature
/// `Fn(RequestContext) -> Future<Output = Result<DynamicResponse, crate::FerrimockError>>`.
pub trait IntoHandlerFn: Send + Sync + 'static {
    /// Convert this callable into a type-erased handler function.
    fn into_handler_fn(self) -> HandlerFn;
}

impl<F, Fut> IntoHandlerFn for F
where
    F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<DynamicResponse, crate::FerrimockError>> + Send + 'static,
{
    fn into_handler_fn(self) -> HandlerFn {
        Arc::new(move |ctx| Box::pin(self(ctx)))
    }
}

/// Also accept a pre-built `HandlerFn` directly (e.g., from napi bridge).
impl IntoHandlerFn for HandlerFn {
    fn into_handler_fn(self) -> HandlerFn {
        self
    }
}

/// Create a `MockDefinition` from method, path pattern, and handler function.
///
/// Absolute-URL predicates (`https://api.example.com/users/:id`) split into
/// a Host-header matcher plus the path pattern, so MSW handlers written
/// against full URLs match (scheme is ignored; requests carry no scheme by
/// the time they reach the matcher).
fn build_handler_mock(
    method: Option<Method>,
    path: &str,
    handler: impl IntoHandlerFn,
    id_prefix: &str,
) -> MockDefinition {
    let (host, path) = match UrlPattern::split_absolute_url(path) {
        Some((host, path)) => (Some(host), path),
        None => (None, path),
    };

    let url_pattern = if path == "*" {
        // Wildcard: match everything
        SmallVec::new() // empty url_patterns = match all
    } else if path.contains(':') || path.contains('*') {
        // Path pattern with :params or wildcards
        let pattern = UrlPattern::path_pattern(path).unwrap_or_else(|_| UrlPattern::exact(path));
        SmallVec::from_elem(pattern, 1)
    } else {
        // Exact path
        SmallVec::from_elem(UrlPattern::exact(path), 1)
    };

    let methods = match method {
        Some(m) => SmallVec::from_elem(m, 1),
        None => SmallVec::new(), // empty = match all methods
    };

    let header_matchers = match host {
        Some(host) => SmallVec::from_elem(
            crate::types::HeaderMatcher::exact(::http::header::HOST, host),
            1,
        ),
        None => SmallVec::new(),
    };

    MockDefinition {
        id: next_handler_id(id_prefix),
        priority: 100, // High default priority for handler mocks
        request: RequestMatcher {
            methods,
            url_patterns: url_pattern,
            header_matchers,
            ..RequestMatcher::default()
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::handler(handler.into_handler_fn()),
        ),
        enabled: true,
        once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        vars: None,
        streaming: None,
    }
}

/// HTTP method handler factories.
///
/// Each function creates a `MockDefinition` with the specified method, path pattern,
/// and handler function. The mock goes into the standard `MockRegistry`.
///
/// # Path patterns
///
/// - `/users/:id` — named parameter (captures as `ctx.captures["id"]`)
/// - `/files/*` — wildcard segment
/// - `/exact/path` — exact match
/// - `*` — match all paths
///
/// # Examples
///
/// ```rust,ignore
/// use ferrimock::handler::{http, HttpResponse};
///
/// let mock = http::get("/users/:id", |ctx| async move {
///     let id = ctx.captures.get("id").unwrap();
///     Ok(HttpResponse::json(&serde_json::json!({ "id": id, "name": "John" }))?)
/// });
/// ```
pub mod http {
    use super::*;

    /// Create a GET handler mock.
    pub fn get(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::GET), path, handler, "GET")
    }

    /// Create a POST handler mock.
    pub fn post(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::POST), path, handler, "POST")
    }

    /// Create a PUT handler mock.
    pub fn put(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::PUT), path, handler, "PUT")
    }

    /// Create a DELETE handler mock.
    pub fn delete(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::DELETE), path, handler, "DELETE")
    }

    /// Create a PATCH handler mock.
    pub fn patch(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::PATCH), path, handler, "PATCH")
    }

    /// Create a HEAD handler mock.
    pub fn head(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::HEAD), path, handler, "HEAD")
    }

    /// Create an OPTIONS handler mock.
    pub fn options(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(Some(Method::OPTIONS), path, handler, "OPTIONS")
    }

    /// Create a handler mock matching any HTTP method.
    pub fn all(path: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_handler_mock(None, path, handler, "ALL")
    }
}

/// GraphQL handler factories.
///
/// Creates `MockDefinition`s with GraphQL operation matchers.
///
/// # Examples
///
/// ```rust,ignore
/// use ferrimock::handler::{graphql, HttpResponse};
///
/// let mock = graphql::query("GetUser", |ctx| async move {
///     let body = ctx.body_json.as_ref().unwrap();
///     let variables = body.get("variables");
///     Ok(HttpResponse::json(&serde_json::json!({
///         "data": { "user": { "id": variables.and_then(|v| v.get("id")) } }
///     }))?)
/// });
/// ```
pub mod graphql {
    use super::*;
    use crate::types::{GraphQLMatcher, GraphQLOperationType};

    /// Create a handler mock for a GraphQL query operation.
    pub fn query(operation_name: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_graphql_mock(
            Some(GraphQLOperationType::Query),
            Some(operation_name),
            handler,
            "GQL_QUERY",
        )
    }

    /// Create a handler mock for a GraphQL mutation operation.
    pub fn mutation(operation_name: &str, handler: impl IntoHandlerFn) -> MockDefinition {
        build_graphql_mock(
            Some(GraphQLOperationType::Mutation),
            Some(operation_name),
            handler,
            "GQL_MUTATION",
        )
    }

    /// Create a handler mock matching any GraphQL operation.
    pub fn operation(handler: impl IntoHandlerFn) -> MockDefinition {
        let mut mock = build_handler_mock(Some(Method::POST), "*", handler, "GQL_OP");
        mock.request.graphql_matcher = Some(GraphQLMatcher {
            match_any: true,
            ..GraphQLMatcher::default()
        });
        mock
    }

    /// Create a handler mock for a GraphQL query whose operation name
    /// matches a regex (MSW's RegExp operation predicate).
    pub fn query_regex(pattern: regex::Regex, handler: impl IntoHandlerFn) -> MockDefinition {
        build_graphql_regex_mock(GraphQLOperationType::Query, pattern, handler, "GQL_QUERY")
    }

    /// Create a handler mock for a GraphQL mutation whose operation name
    /// matches a regex.
    pub fn mutation_regex(pattern: regex::Regex, handler: impl IntoHandlerFn) -> MockDefinition {
        build_graphql_regex_mock(
            GraphQLOperationType::Mutation,
            pattern,
            handler,
            "GQL_MUTATION",
        )
    }

    fn build_graphql_mock(
        op_type: Option<GraphQLOperationType>,
        op_name: Option<&str>,
        handler: impl IntoHandlerFn,
        id_prefix: &str,
    ) -> MockDefinition {
        // GraphQL requests are typically POST to any endpoint
        let mut mock = build_handler_mock(Some(Method::POST), "*", handler, id_prefix);
        mock.request.graphql_matcher = Some(GraphQLMatcher {
            operation_name: op_name.map(String::from),
            operation_type: op_type,
            ..GraphQLMatcher::default()
        });
        mock
    }

    fn build_graphql_regex_mock(
        op_type: GraphQLOperationType,
        pattern: regex::Regex,
        handler: impl IntoHandlerFn,
        id_prefix: &str,
    ) -> MockDefinition {
        let mut mock = build_handler_mock(Some(Method::POST), "*", handler, id_prefix);
        mock.request.graphql_matcher = Some(GraphQLMatcher {
            operation_name_regex: Some(pattern),
            operation_type: Some(op_type),
            ..GraphQLMatcher::default()
        });
        mock
    }
}

/// Convenience builders for [`DynamicResponse`] values.
///
/// Used inside handler functions to construct responses ergonomically.
///
/// # Examples
///
/// ```rust,ignore
/// use ferrimock::handler::HttpResponse;
///
/// // JSON response (200 OK)
/// let resp = HttpResponse::json(&serde_json::json!({"key": "value"}))?;
///
/// // Text with custom status
/// let resp = HttpResponse::text("Not Found").with_status(http::StatusCode::NOT_FOUND);
///
/// // Empty response
/// let resp = HttpResponse::empty(http::StatusCode::NO_CONTENT);
/// ```
pub struct HttpResponse;

impl HttpResponse {
    /// Create a JSON response with status 200.
    ///
    /// Sets `Content-Type: application/json` automatically.
    pub fn json<T: serde::Serialize>(data: &T) -> Result<DynamicResponse, serde_json::Error> {
        let body = serde_json::to_vec(data)?;
        Ok(DynamicResponse {
            status: Some(StatusCode::OK),
            headers: Some(
                std::iter::once(("content-type".to_string(), "application/json".to_string()))
                    .collect(),
            ),
            body: bytes::Bytes::from(body),
            ..DynamicResponse::default()
        })
    }

    /// Create a plain text response with status 200.
    ///
    /// Sets `Content-Type: text/plain` automatically.
    pub fn text(body: impl Into<String>) -> DynamicResponse {
        DynamicResponse {
            status: Some(StatusCode::OK),
            headers: Some(
                std::iter::once(("content-type".to_string(), "text/plain".to_string())).collect(),
            ),
            body: bytes::Bytes::from(body.into()),
            ..DynamicResponse::default()
        }
    }

    /// Create an HTML response with status 200.
    ///
    /// Sets `Content-Type: text/html` automatically.
    pub fn html(body: impl Into<String>) -> DynamicResponse {
        DynamicResponse {
            status: Some(StatusCode::OK),
            headers: Some(
                std::iter::once(("content-type".to_string(), "text/html".to_string())).collect(),
            ),
            body: bytes::Bytes::from(body.into()),
            ..DynamicResponse::default()
        }
    }

    /// Create an empty response with the given status code.
    pub fn empty(status: StatusCode) -> DynamicResponse {
        DynamicResponse {
            status: Some(status),
            ..DynamicResponse::default()
        }
    }
}

/// Extension methods on [`DynamicResponse`] for builder-style construction.
pub trait DynamicResponseExt {
    /// Override the status code.
    #[must_use]
    fn with_status(self, status: StatusCode) -> Self;
    /// Add a header to the response.
    #[must_use]
    fn with_header(self, name: impl Into<String>, value: impl Into<String>) -> Self;
}

impl DynamicResponseExt for DynamicResponse {
    fn with_status(mut self, status: StatusCode) -> Self {
        self.status = Some(status);
        self
    }

    fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .get_or_insert_with(Default::default)
            .insert(name.into(), value.into());
        self
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
    use crate::engine::{MockMatcher, MockRegistry};

    // ---- Path pattern tests ----

    #[test]
    fn test_path_pattern_simple_param() {
        let pattern = UrlPattern::path_pattern("/users/:id").unwrap();
        assert!(pattern.matches("/users/123"));
        assert!(pattern.matches("/users/abc"));
        assert!(!pattern.matches("/users/123/extra"));
        assert!(!pattern.matches("/users/"));
    }

    #[test]
    fn test_path_pattern_multiple_params() {
        let pattern = UrlPattern::path_pattern("/users/:userId/posts/:postId").unwrap();
        assert!(pattern.matches("/users/1/posts/42"));
        assert!(!pattern.matches("/users/1/posts"));
    }

    #[test]
    fn test_path_pattern_captures() {
        let pattern = UrlPattern::path_pattern("/users/:id").unwrap();
        let captures = pattern.extract_captures("/users/456").unwrap();
        assert_eq!(&captures["id"], "456");
    }

    #[test]
    fn test_path_pattern_multiple_captures() {
        let pattern = UrlPattern::path_pattern("/users/:userId/posts/:postId").unwrap();
        let captures = pattern.extract_captures("/users/7/posts/99").unwrap();
        assert_eq!(&captures["userId"], "7");
        assert_eq!(&captures["postId"], "99");
    }

    #[test]
    fn test_path_pattern_exact_fallback() {
        let pattern = UrlPattern::path_pattern("/api/health").unwrap();
        assert!(pattern.matches("/api/health"));
        assert!(!pattern.matches("/api/healthz"));
    }

    // ---- Handler builder tests ----

    #[test]
    fn test_http_get_creates_mock_definition() {
        let mock = http::get(
            "/users/:id",
            |_ctx| async move { Ok(HttpResponse::text("ok")) },
        );

        assert!(mock.enabled);
        assert_eq!(mock.priority, 100);
        assert_eq!(mock.request.methods.len(), 1);
        assert_eq!(mock.request.methods[0], Method::GET);
        assert!(matches!(mock.response.body, BodySource::Handler(_)));
    }

    #[test]
    fn test_http_all_matches_any_method() {
        let mock = http::all("/test", |_ctx| async move { Ok(HttpResponse::text("ok")) });

        // Empty methods = match all
        assert!(mock.request.methods.is_empty());
    }

    #[test]
    fn test_handler_ids_are_unique() {
        let mock1 = http::get("/a", |_ctx| async move { Ok(HttpResponse::text("a")) });
        let mock2 = http::get("/b", |_ctx| async move { Ok(HttpResponse::text("b")) });
        assert_ne!(mock1.id, mock2.id);
    }

    #[test]
    fn test_wildcard_path() {
        let mock = http::get("*", |_ctx| async move { Ok(HttpResponse::text("ok")) });
        // Wildcard = empty url_patterns (match all)
        assert!(mock.request.url_patterns.is_empty());
    }

    // ---- HttpResponse builder tests ----

    #[test]
    fn test_mock_response_json() {
        let resp = HttpResponse::json(&serde_json::json!({"key": "value"})).unwrap();
        assert_eq!(resp.status, Some(StatusCode::OK));
        assert_eq!(
            resp.headers.as_ref().unwrap().get("content-type").unwrap(),
            "application/json"
        );
        let body: serde_json::Value = serde_json::from_slice(&resp.body).unwrap();
        assert_eq!(body["key"], "value");
    }

    #[test]
    fn test_mock_response_text() {
        let resp = HttpResponse::text("hello");
        assert_eq!(resp.status, Some(StatusCode::OK));
        assert_eq!(
            resp.headers.as_ref().unwrap().get("content-type").unwrap(),
            "text/plain"
        );
        assert_eq!(resp.body.as_ref(), b"hello");
    }

    #[test]
    fn test_mock_response_empty() {
        let resp = HttpResponse::empty(StatusCode::NO_CONTENT);
        assert_eq!(resp.status, Some(StatusCode::NO_CONTENT));
        assert!(resp.body.is_empty());
    }

    #[test]
    fn test_dynamic_response_ext_with_status() {
        let resp = HttpResponse::text("err").with_status(StatusCode::BAD_REQUEST);
        assert_eq!(resp.status, Some(StatusCode::BAD_REQUEST));
    }

    #[test]
    fn test_dynamic_response_ext_with_header() {
        let resp = HttpResponse::text("ok").with_header("x-custom", "value");
        assert_eq!(
            resp.headers.as_ref().unwrap().get("x-custom").unwrap(),
            "value"
        );
    }

    // ---- Integration with MockRegistry/MockMatcher ----

    #[tokio::test]
    async fn test_handler_mock_in_registry() {
        let registry = MockRegistry::new();
        let mock = http::get("/api/hello", |_ctx| async move {
            Ok(HttpResponse::json(&serde_json::json!({"msg": "hi"})).unwrap())
        });

        registry.add_mock(mock);
        assert_eq!(registry.len(), 1);

        let matcher = MockMatcher::new(registry);
        let result = matcher.find_match(
            &Method::GET,
            "/api/hello",
            None,
            &::http::HeaderMap::new(),
            None,
        );

        assert!(result.is_some());
        let mock_match = result.unwrap();
        assert!(matches!(
            mock_match.mock.response.body,
            BodySource::Handler(_)
        ));
    }

    #[tokio::test]
    async fn test_handler_with_params_matching() {
        let registry = MockRegistry::new();
        let mock = http::get(
            "/users/:id",
            |_ctx| async move { Ok(HttpResponse::text("ok")) },
        );

        registry.add_mock(mock);

        let matcher = MockMatcher::new(registry);

        // Should match
        let result = matcher.find_match(
            &Method::GET,
            "/users/123",
            None,
            &::http::HeaderMap::new(),
            None,
        );
        assert!(result.is_some());
        let mock_match = result.unwrap();
        assert_eq!(&mock_match.captures["id"], "123");

        // Should not match wrong method
        let result = matcher.find_match(
            &Method::POST,
            "/users/123",
            None,
            &::http::HeaderMap::new(),
            None,
        );
        assert!(result.is_none());

        // Should not match wrong path
        let result = matcher.find_match(
            &Method::GET,
            "/posts/123",
            None,
            &::http::HeaderMap::new(),
            None,
        );
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_handler_response_generation() {
        use crate::engine::types::ResponseGeneratorExt;

        let registry = MockRegistry::new();
        let mock = http::get("/greet/:name", |ctx: RequestContext| async move {
            let name = ctx
                .captures
                .get("name")
                .cloned()
                .unwrap_or_else(|| "world".to_string());
            Ok(
                HttpResponse::json(&serde_json::json!({"greeting": format!("Hello, {name}!")}))
                    .unwrap(),
            )
        });

        registry.add_mock(mock);
        let matcher = MockMatcher::new(registry);

        let mock_match = matcher
            .find_match(
                &Method::GET,
                "/greet/Alice",
                None,
                &::http::HeaderMap::new(),
                None,
            )
            .unwrap();

        let response = mock_match
            .mock
            .response
            .generate_dynamic(
                "GET",
                "/greet/Alice",
                None,
                &::http::HeaderMap::new(),
                None,
                mock_match.captures,
                None,
            )
            .await
            .unwrap();

        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["greeting"], "Hello, Alice!");
        assert_eq!(response.status, Some(StatusCode::OK));
    }

    #[tokio::test]
    async fn test_handler_coexists_with_declarative_mocks() {
        let registry = MockRegistry::new();

        // Add a declarative mock (higher priority)
        let declarative = MockDefinition {
            id: "declarative-1".into(),
            priority: 200,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            request: RequestMatcher {
                methods: SmallVec::from_elem(Method::GET, 1),
                url_patterns: SmallVec::from_elem(UrlPattern::exact("/api/static"), 1),
                ..RequestMatcher::default()
            },
            response: ResponseGenerator::new(
                StatusCode::OK,
                BodySource::inline(r#"{"static":true}"#),
            ),
            vars: None,
            streaming: None,
        };
        registry.add_mock(declarative);

        // Add a handler mock (lower priority)
        let handler = http::get("/api/dynamic", |_ctx| async move {
            Ok(HttpResponse::json(&serde_json::json!({"dynamic": true})).unwrap())
        });
        registry.add_mock(handler);

        let matcher = MockMatcher::new(registry);

        // Both should match their respective paths
        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/static",
                    None,
                    &::http::HeaderMap::new(),
                    None
                )
                .is_some()
        );
        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/dynamic",
                    None,
                    &::http::HeaderMap::new(),
                    None
                )
                .is_some()
        );

        // Neither should match wrong paths
        assert!(
            matcher
                .find_match(
                    &Method::GET,
                    "/api/other",
                    None,
                    &::http::HeaderMap::new(),
                    None
                )
                .is_none()
        );
    }

    // ---- GraphQL handler tests ----

    #[test]
    fn test_graphql_query_handler() {
        let mock = graphql::query("GetUser", |_ctx| async move {
            Ok(HttpResponse::json(&serde_json::json!({"data": {"user": {}}})).unwrap())
        });

        assert!(mock.request.graphql_matcher.is_some());
        let gql = mock.request.graphql_matcher.as_ref().unwrap();
        assert_eq!(gql.operation_name.as_deref(), Some("GetUser"));
        assert!(!gql.match_any);
    }

    #[test]
    fn test_graphql_operation_handler() {
        let mock = graphql::operation(|_ctx| async move {
            Ok(HttpResponse::json(&serde_json::json!({"data": {}})).unwrap())
        });

        let gql = mock.request.graphql_matcher.as_ref().unwrap();
        assert!(gql.match_any);
    }
}
