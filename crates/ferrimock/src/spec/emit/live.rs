//! Binding a schema-derived backend onto an ordinary mock.
//!
//! There is no second serving path. A `serve:` mock is a normal
//! [`MockDefinition`] whose body happens to be a Rust closure over the world,
//! so matching, priority, scopes, call tracking, coverage, hot reload and both
//! host lanes keep working unchanged — and a mock written by hand at a higher
//! priority still wins, which is how you force one operation to fail without
//! giving up the rest of the backend.

use bytes::Bytes;
use http::StatusCode;
use smallvec::SmallVec;
use std::sync::Arc;

use super::super::bind::graphql::{GraphQLBackend, parse_request};
use crate::types::{
    BodySource, ContextNeeds, DynamicResponse, GraphQLMatcher, MockDefinition, ResponseGenerator,
};

/// Priority for schema-derived routes. Owned by the config layer, because
/// choosing a mock's priority is a config concern and there must be one.
pub use crate::config::serve::SERVED_PRIORITY as SPEC_PRIORITY;

/// Turn a definition into the GraphQL endpoint for a backend.
///
/// The definition arrives with its URL, priority, scope, headers and delay
/// already read from the mock that declared it — this supplies only the
/// behavior, which is the half a schema file can actually specify.
///
/// Matching is on *any* GraphQL operation. Matching by operation name would be
/// finer grained, but the name is chosen by the client, not by the schema: a
/// schema-derived backend cannot know it in advance, and pretending otherwise
/// would leave real requests unmatched.
pub fn bind_graphql(mock: &mut MockDefinition, backend: Arc<GraphQLBackend>) {
    let handler = move |ctx: crate::types::RequestContext| {
        let backend = Arc::clone(&backend);
        Box::pin(async move { answer(&backend, ctx).await })
            as std::pin::Pin<Box<dyn Future<Output = _> + Send>>
    };

    if mock.request.methods.is_empty() {
        mock.request.methods = SmallVec::from_elem(http::Method::POST, 1);
    }
    if mock.request.graphql_matcher.is_none() {
        mock.request.graphql_matcher = Some(GraphQLMatcher {
            match_any: true,
            ..GraphQLMatcher::default()
        });
    }

    let headers = std::mem::take(&mut mock.response.headers);
    let delay = mock.response.delay;
    let mut response = ResponseGenerator::new(
        StatusCode::OK,
        // The query is in the body and nothing else is read, so the matching
        // lanes can skip marshalling headers entirely.
        BodySource::handler_with_needs(Arc::new(handler), ContextNeeds::body_only()),
    );
    response.headers = headers;
    response
        .headers
        .entry("content-type".to_string())
        .or_insert_with(|| "application/json".to_string());
    response.delay = delay;
    mock.response = response;
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
    use crate::core::{World, WorldSettings};
    use crate::spec::infer::graphql::parse_sdl;
    use crate::spec::source::load_schema;
    use crate::types::{RequestContext, RequestMatcher};
    use std::path::Path;

    const SCHEMA: &str = "type User { id: ID!, name: String! } type Query { users: [User!]! }";

    fn backend() -> (Arc<World>, Arc<GraphQLBackend>) {
        let world = Arc::new(World::new());
        world
            .configure(
                &WorldSettings {
                    seed: Some(1),
                    counts: std::iter::once((lean_string::LeanString::from("User"), 3)).collect(),
                    ..WorldSettings::default()
                },
                Path::new("test"),
            )
            .unwrap();
        load_schema(SCHEMA, Path::new("s.graphql"), &world, false).unwrap();

        let parsed = parse_sdl(SCHEMA).unwrap();
        let backend = Arc::new(GraphQLBackend::build(&parsed, Arc::clone(&world)).unwrap());
        (world, backend)
    }

    fn bare(id: &str) -> MockDefinition {
        MockDefinition {
            id: id.into(),
            priority: SPEC_PRIORITY,
            request: RequestMatcher::default(),
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("")),
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
        }
    }

    fn post(body: &str) -> RequestContext {
        let mut ctx = RequestContext::new();
        ctx.body = Some(body.to_string());
        ctx
    }

    #[test]
    fn the_binding_leaves_an_ordinary_mock() {
        let (_world, backend) = backend();
        let mut mock = bare("filestore-graphql");
        bind_graphql(&mut mock, backend);

        assert_eq!(mock.request.methods.as_slice(), [http::Method::POST]);
        assert!(mock.request.graphql_matcher.is_some());
        assert!(mock.response.body.is_handler());
        assert!(
            mock.priority < 100,
            "a hand-written mock must outrank the backend"
        );
    }

    #[test]
    fn the_binding_declares_that_it_reads_only_the_body() {
        let (_world, backend) = backend();
        let mut mock = bare("filestore-graphql");
        bind_graphql(&mut mock, backend);

        assert!(mock.response.context_uses_body);
        assert!(
            !mock.response.context_uses_headers,
            "a GraphQL query is in the body; headers need not be marshalled"
        );
    }

    #[test]
    fn headers_declared_on_the_mock_survive_the_binding() {
        let (_world, backend) = backend();
        let mut mock = bare("filestore-graphql");
        mock.response
            .headers
            .insert("x-served-by".to_string(), "mock".to_string());
        bind_graphql(&mut mock, backend);

        assert_eq!(
            mock.response.headers.get("x-served-by").map(String::as_str),
            Some("mock")
        );
        assert_eq!(
            mock.response
                .headers
                .get("content-type")
                .map(String::as_str),
            Some("application/json")
        );
    }

    #[tokio::test]
    async fn it_answers_a_query() {
        let (_world, backend) = backend();
        let response = answer(&backend, post(r#"{"query":"{ users { id name } }"}"#))
            .await
            .unwrap();

        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(payload.get("errors").is_none(), "unexpected: {payload}");
        assert_eq!(payload["data"]["users"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_malformed_body_is_a_graphql_error_not_a_crash() {
        let (_world, backend) = backend();
        let response = answer(&backend, post("not json")).await.unwrap();
        assert_eq!(response.status, Some(StatusCode::BAD_REQUEST));

        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(payload["errors"][0]["message"].is_string());
    }

    #[tokio::test]
    async fn a_query_error_still_comes_back_as_a_graphql_envelope() {
        let (_world, backend) = backend();
        let response = answer(&backend, post(r#"{"query":"{ nope }"}"#))
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert!(
            !payload["errors"].as_array().unwrap().is_empty(),
            "a validation failure belongs in the envelope"
        );
    }

    /// The point of the whole design: a write through the world is visible to
    /// a schema-derived route, because they are the same store.
    #[tokio::test]
    async fn a_write_through_the_world_is_visible_to_the_backend() {
        let (world, backend) = backend();
        world
            .create("User", serde_json::json!({ "name": "Ada" }))
            .unwrap();

        let response = answer(&backend, post(r#"{"query":"{ users { name } }"}"#))
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        let names: Vec<&str> = payload["data"]["users"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|u| u["name"].as_str())
            .collect();

        assert!(names.contains(&"Ada"), "unexpected: {names:?}");
    }
}
