//! Mounting a spec-derived backend as ordinary mocks.
//!
//! The backend is not a second serving path: it is a normal [`MockDefinition`]
//! whose body happens to be a Rust closure, so matching, priority, scopes,
//! call tracking, hot reload and both host lanes keep working unchanged — and
//! a user's own mock at a higher priority still wins, which is how you force
//! one endpoint to fail without giving up the rest of the backend.

use bytes::Bytes;
use http::StatusCode;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::sync::Arc;

use super::super::bind::graphql::{GraphQLBackend, parse_request};
use crate::types::{
    BodySource, ContextNeeds, DynamicResponse, GraphQLMatcher, MockDefinition, RequestMatcher,
    ResponseGenerator, UrlPattern,
};

/// Priority for spec-derived routes.
///
/// Below the handler API's 100, so a mock written by hand outranks the
/// backend without anyone having to think about numbers.
pub const SPEC_PRIORITY: u32 = 50;

/// Mount a GraphQL backend at an endpoint.
///
/// One mock, matching any GraphQL operation. Matching on operation *name*
/// would be finer grained, but the name is chosen by the client, not by the
/// schema — a spec-derived backend cannot know it in advance, and pretending
/// otherwise would leave real requests unmatched.
#[must_use]
pub fn mount_graphql(backend: Arc<GraphQLBackend>, endpoint: &str) -> MockDefinition {
    let handler = move |ctx: crate::types::RequestContext| {
        let backend = Arc::clone(&backend);
        Box::pin(async move { answer(&backend, ctx).await })
            as std::pin::Pin<Box<dyn Future<Output = _> + Send>>
    };

    let mut headers = FxHashMap::default();
    headers.insert("content-type".to_string(), "application/json".to_string());

    MockDefinition {
        id: format!("spec:graphql:{endpoint}").into(),
        priority: SPEC_PRIORITY,
        request: RequestMatcher {
            methods: SmallVec::from_elem(http::Method::POST, 1),
            url_patterns: SmallVec::from_elem(UrlPattern::Exact(endpoint.to_string()), 1),
            graphql_matcher: Some(GraphQLMatcher {
                match_any: true,
                ..GraphQLMatcher::default()
            }),
            ..RequestMatcher::default()
        },
        response: {
            let mut response = ResponseGenerator::new(
                StatusCode::OK,
                // The query is in the body and nothing else is read, so the
                // matching lanes can skip marshalling headers entirely.
                BodySource::handler_with_needs(Arc::new(handler), ContextNeeds::body_only()),
            );
            response.headers = headers;
            response
        },
        enabled: true,
        once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        vars: None,
        streaming: None,
    }
}

async fn answer(
    backend: &GraphQLBackend,
    ctx: crate::types::RequestContext,
) -> Result<DynamicResponse, crate::FerrimockError> {
    let body = ctx
        .body
        .as_deref()
        .map(str::as_bytes)
        .or(ctx.body_bytes.as_deref())
        .unwrap_or_default();

    // A malformed request is a 400 with a GraphQL-shaped error body, not a
    // transport failure: a client parsing the envelope should still be able to.
    let request = match parse_request(body) {
        Ok(request) => request,
        Err(error) => return Ok(error_response(&error.to_string())),
    };

    let response = backend.execute(request).await;
    let payload = serde_json::to_vec(&response)
        .map_err(|e| crate::mp_err!("Could not serialise the GraphQL response: {e}"))?;
    Ok(DynamicResponse::body_only(Bytes::from(payload)))
}

fn error_response(message: &str) -> DynamicResponse {
    let body = serde_json::json!({ "errors": [{ "message": message }] });
    let mut response = DynamicResponse::body_only(Bytes::from(body.to_string()));
    response.status = Some(StatusCode::BAD_REQUEST);
    response
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
    use crate::spec::infer::graphql::{parse_sdl, to_entity_graph};
    use crate::spec::store::{EntityStore, StoreConfig};
    use crate::types::RequestContext;

    fn backend() -> Arc<GraphQLBackend> {
        let parsed = parse_sdl("type User { id: ID!, name: String! } type Query { users: [User!]! }")
            .unwrap();
        let graph = to_entity_graph(&parsed);
        let store = EntityStore::new(
            Arc::new(graph),
            StoreConfig::seeded(1).with_count("User", 3),
        );
        Arc::new(GraphQLBackend::build(&parsed, Arc::new(store)).unwrap())
    }

    fn post(body: &str) -> RequestContext {
        let mut ctx = RequestContext::new();
        ctx.body = Some(body.to_string());
        ctx
    }

    #[test]
    fn the_mount_is_an_ordinary_mock() {
        let mock = mount_graphql(backend(), "/graphql");
        assert_eq!(mock.request.methods.as_slice(), [http::Method::POST]);
        assert!(mock.request.graphql_matcher.is_some());
        assert!(mock.response.body.is_handler());
        assert!(
            mock.priority < 100,
            "a hand-written mock must outrank the backend"
        );
    }

    #[test]
    fn the_mount_declares_that_it_reads_only_the_body() {
        let mock = mount_graphql(backend(), "/graphql");
        assert!(mock.response.context_uses_body);
        assert!(
            !mock.response.context_uses_headers,
            "a GraphQL query is in the body; headers need not be marshalled"
        );
    }

    #[tokio::test]
    async fn it_answers_a_query() {
        let backend = backend();
        let response = answer(&backend, post(r#"{"query":"{ users { id name } }"}"#))
            .await
            .unwrap();

        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(payload.get("errors").is_none(), "unexpected: {payload}");
        assert_eq!(payload["data"]["users"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_malformed_body_is_a_graphql_error_not_a_crash() {
        let backend = backend();
        let response = answer(&backend, post("not json")).await.unwrap();
        assert_eq!(response.status, Some(StatusCode::BAD_REQUEST));

        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(payload["errors"][0]["message"].is_string());
    }

    #[tokio::test]
    async fn a_query_error_still_comes_back_as_a_graphql_envelope() {
        let backend = backend();
        let response = answer(&backend, post(r#"{"query":"{ nope }"}"#))
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(
            !payload["errors"].as_array().unwrap().is_empty(),
            "a validation failure belongs in the envelope"
        );
    }
}
