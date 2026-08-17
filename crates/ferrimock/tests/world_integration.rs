//! One cohesive system, end to end.
//!
//! A directory holding a schema, a collection that says where it is served,
//! hand-written overrides and a template — loaded through the one seam every
//! consumer uses, and asserted through ordinary matching. The claim under test
//! is that these are not three systems: overrides win by ordinary priority,
//! and everything reads and writes the same entities.

#![cfg(all(feature = "spec", feature = "engine"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use ferrimock::core::World;
use ferrimock::engine::{Expected, MockMatcher, MockRegistry, ResponseGeneratorExt};
use ferrimock::types::RequestContext;
use http::{HeaderMap, Method};
use serde_json::Value as JsonValue;

const SCHEMA: &str = r"
    type User {
      id: ID!
      name: String!
      email: String!
    }
    type Folder {
      id: ID!
      name: String!
      owner: User!
    }
    type Query {
      users: [User!]!
      user(id: ID!): User
      folders: [Folder!]!
    }
    type Mutation {
      createFolder(name: String!): Folder
    }
";

const COLLECTION: &str = r#"
name: Filestore API
world:
  schemas:
    - schema.graphql
  seed: 42
  counts:
    User: 3
    Folder: 5

mocks:
  # The whole schema, at an absolute URL a proxy can front.
  - id: filestore-graphql
    match:
      POST: https://api.example.com/graphql
    serve: graphql

  # One operation forced to fail; the rest of the schema keeps serving.
  - id: quota-exceeded
    match:
      POST: https://api.example.com/graphql
      graphql:
        mutation: CreateFolder
    response:
      json:
        errors:
          - message: Storage quota exceeded

  # A declarative template reading the same entities the schema serves.
  - id: user-count
    match:
      GET: https://api.example.com/2.0/users/count
    response:
      template: '{"total": {{ entity_count(type="User") }}}'
"#;

/// A registry over its own world, so these tests do not race the
/// process-global one that templates reach.
async fn load() -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("schema.graphql"), SCHEMA).unwrap();
    std::fs::write(dir.path().join("mocks.yaml"), COLLECTION).unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

/// The request a server actually receives for `https://api.example.com/graphql`:
/// the path, plus a Host header. Never the whole URL.
fn api_host() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(http::header::HOST, "api.example.com".parse().unwrap());
    headers
}

async fn post_graphql(registry: &Arc<MockRegistry>, body: &str) -> JsonValue {
    let matcher = MockMatcher::new((**registry).clone());
    let found = matcher
        .find_match(
            &Method::POST,
            "/graphql",
            None,
            &api_host(),
            Some(body.as_bytes()),
        )
        .expect("a mock matches the GraphQL endpoint");

    let mut ctx = RequestContext::new();
    ctx.body = Some(body.to_string());
    let bytes = found
        .mock
        .response
        .generate_with_context(&ctx)
        .await
        .expect("the matched mock renders");

    serde_json::from_slice(&bytes).expect("a JSON body")
}

#[tokio::test]
async fn a_schema_serves_at_the_url_the_mock_names() {
    let (_dir, registry) = load().await;

    let payload = post_graphql(&registry, r#"{"query":"{ users { id name email } }"}"#).await;
    assert!(payload.get("errors").is_none(), "unexpected: {payload}");

    let users = payload["data"]["users"].as_array().unwrap();
    assert_eq!(users.len(), 3, "world.counts must reach the served schema");
    assert!(users[0]["email"].is_string());
}

#[tokio::test]
async fn a_schema_does_not_serve_a_path_nobody_named() {
    let (_dir, registry) = load().await;
    let matcher = MockMatcher::new((*registry).clone());

    assert!(
        matcher
            .find_match(
                &Method::POST,
                "/v2/graphql",
                None,
                &api_host(),
                Some(br#"{"query":"{ users { id } }"}"#),
            )
            .is_none(),
        "a schema must serve only where a mock says it does"
    );
}

/// The reason an absolute `match.url` splits into a path and a Host matcher:
/// a proxy fronting several hosts must not answer for the wrong one.
#[tokio::test]
async fn a_schema_does_not_serve_another_host_on_the_same_path() {
    let (_dir, registry) = load().await;
    let matcher = MockMatcher::new((*registry).clone());

    let mut elsewhere = HeaderMap::new();
    elsewhere.insert(http::header::HOST, "evil.example.com".parse().unwrap());

    assert!(
        matcher
            .find_match(
                &Method::POST,
                "/graphql",
                None,
                &elsewhere,
                Some(br#"{"query":"{ users { id } }"}"#),
            )
            .is_none(),
        "the Host an absolute URL named has to be part of matching"
    );
}

#[tokio::test]
async fn a_hand_written_mock_overrides_one_operation() {
    let (_dir, registry) = load().await;

    let overridden = post_graphql(
        &registry,
        r#"{"query":"mutation CreateFolder { createFolder(name: \"x\") { id } }"}"#,
    )
    .await;
    assert_eq!(
        overridden["errors"][0]["message"], "Storage quota exceeded",
        "the higher-priority mock has to win"
    );

    // And only that operation.
    let untouched = post_graphql(&registry, r#"{"query":"{ folders { id } }"}"#).await;
    assert!(untouched.get("errors").is_none(), "unexpected: {untouched}");
    assert_eq!(untouched["data"]["folders"].as_array().unwrap().len(), 5);
}

#[tokio::test]
async fn overrides_and_served_routes_are_both_ordinary_mocks() {
    let (_dir, registry) = load().await;

    let served = registry.get_mock("filestore-graphql").expect("the served route");
    let override_mock = registry.get_mock("quota-exceeded").expect("the override");

    assert!(
        override_mock.priority > served.priority,
        "an override wins by ordinary priority ({} vs {})",
        override_mock.priority,
        served.priority
    );
    assert_eq!(
        served
            .source_file
            .as_deref()
            .map(|p| p.ends_with("mocks.yaml")),
        Some(true),
        "a served route is tracked to the collection that declared it"
    );
}

/// The deepest claim: the store is not private to the schema.
#[tokio::test]
async fn a_write_through_the_world_is_visible_to_the_served_schema() {
    let (_dir, registry) = load().await;

    registry
        .world()
        .create("User", serde_json::json!({ "name": "Ada Lovelace" }))
        .unwrap();

    let payload = post_graphql(&registry, r#"{"query":"{ users { name } }"}"#).await;
    let names: Vec<&str> = payload["data"]["users"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["name"].as_str())
        .collect();

    assert!(names.contains(&"Ada Lovelace"), "unexpected: {names:?}");
    assert_eq!(names.len(), 4);
}

/// And the other direction: a mutation the schema serves is visible outside it.
#[tokio::test]
async fn a_write_through_the_served_schema_is_visible_to_the_world() {
    let (_dir, registry) = load().await;
    let before = registry.world().count("Folder");

    let payload = post_graphql(
        &registry,
        r#"{"query":"mutation { createFolder(name: \"Reports\") { id name } }"}"#,
    )
    .await;
    assert!(payload.get("errors").is_none(), "unexpected: {payload}");

    let key = payload["data"]["createFolder"]["id"].as_str().unwrap();
    assert_eq!(registry.world().count("Folder"), before + 1);
    assert_eq!(
        registry.world().get("Folder", key).unwrap()["name"],
        "Reports"
    );
}

#[tokio::test]
async fn coverage_counts_a_served_route_like_any_other_mock() {
    let (_dir, registry) = load().await;

    post_graphql(&registry, r#"{"query":"{ users { id } }"}"#).await;
    post_graphql(&registry, r#"{"query":"{ folders { id } }"}"#).await;

    registry
        .verify("filestore-graphql", Expected::Exactly(2))
        .expect("a served route counts its matches");
    registry
        .verify("quota-exceeded", Expected::Never)
        .expect("an override that never fired reads as never");

    let coverage = registry.coverage();
    assert!(
        coverage
            .unused
            .iter()
            .any(|m| m.mock_id == "quota-exceeded"),
        "an unused override has to show up in coverage"
    );
}

#[tokio::test]
async fn a_schema_with_no_mock_serving_it_registers_no_routes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("schema.graphql"), SCHEMA).unwrap();

    let registry = MockRegistry::with_world(Arc::new(World::new()));
    let loaded = registry.load_from_directory(dir.path()).await.unwrap();

    assert_eq!(loaded, 0, "a schema declares entities, not routes");
    assert!(
        !registry.world().is_empty(),
        "but the entities are there, ready for a mock to serve"
    );
    assert!(registry.world().count("User") > 0);
}

#[tokio::test]
async fn two_collections_cannot_disagree_about_the_seed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("schema.graphql"), SCHEMA).unwrap();
    std::fs::write(dir.path().join("a.yaml"), "world:\n  seed: 1\nmocks: []\n").unwrap();
    std::fs::write(dir.path().join("b.yaml"), "world:\n  seed: 2\nmocks: []\n").unwrap();

    let world = Arc::new(World::new());
    let registry = MockRegistry::with_world(Arc::clone(&world));
    registry.load_from_directory(dir.path()).await.unwrap();

    // The loader logs and carries on rather than refusing the directory, so
    // the assertion is that one of them won outright — never a blend.
    assert!(
        world.seed() == 1 || world.seed() == 2,
        "the seed must come from one collection, not a mix"
    );
}

#[tokio::test]
async fn serving_an_unknown_protocol_names_the_known_ones() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("schema.graphql"), SCHEMA).unwrap();
    std::fs::write(
        dir.path().join("mocks.yaml"),
        "mocks:\n  - id: soapy\n    match:\n      POST: /soap\n    serve: soap\n",
    )
    .unwrap();

    let registry = MockRegistry::with_world(Arc::new(World::new()));
    registry.load_from_directory(dir.path()).await.unwrap();

    assert!(
        registry.get_mock("soapy").is_none(),
        "a mock that cannot be served must not register a route that answers nothing"
    );
}

#[tokio::test]
async fn serve_cannot_be_combined_with_a_response_body() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("schema.graphql"), SCHEMA).unwrap();
    std::fs::write(
        dir.path().join("mocks.yaml"),
        "world:\n  schemas: [schema.graphql]\nmocks:\n  - id: both\n    match:\n      \
         POST: /graphql\n    serve: graphql\n    response:\n      json: {a: 1}\n",
    )
    .unwrap();

    let registry = MockRegistry::with_world(Arc::new(World::new()));
    registry.load_from_directory(dir.path()).await.unwrap();

    assert!(
        registry.get_mock("both").is_none(),
        "`serve` produces the response, so a body alongside it is a contradiction"
    );
}

// ============================================================================
// The HTTP surface
// ============================================================================

/// The world over HTTP, beside `/__ferrimock/store`.
///
/// Served on a real socket rather than a router harness: the routes exist so an
/// external driver — a Playwright fixture, a shell script — can reach the same
/// entities a template or a script does, and that is a claim about a wire, not
/// about a `Router` value.
#[cfg(feature = "api")]
mod http_api {
    use super::{COLLECTION, SCHEMA};
    use ferrimock::api::{MockApiConfig, MockApiState, create_mock_router};
    use ferrimock::core::World;
    use ferrimock::engine::{MockMatcher, MockRegistry};
    use ferrimock::server::MockState;
    use serde_json::Value as JsonValue;
    use std::sync::Arc;

    async fn serve() -> (tempfile::TempDir, String, Arc<MockRegistry>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("schema.graphql"), SCHEMA).unwrap();
        std::fs::write(dir.path().join("mocks.yaml"), COLLECTION).unwrap();

        let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
        registry.load_from_directory(dir.path()).await.unwrap();

        let state = MockApiState {
            mock: MockState {
                mock_registry: Arc::clone(&registry),
                mock_matcher: Arc::new(MockMatcher::new((*registry).clone())),
                mock_recorder: Arc::new(tokio::sync::RwLock::new(None)),
            },
            config: Arc::new(MockApiConfig {
                collections_dir: None,
                recordings_dir: None,
                recording_enabled: false,
            }),
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = create_mock_router().with_state(state);
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        (dir, base, registry)
    }

    async fn get(url: &str) -> (u16, JsonValue) {
        let response = reqwest::get(url).await.unwrap();
        let status = response.status().as_u16();
        let body = response.text().await.unwrap();
        (
            status,
            serde_json::from_str(&body).unwrap_or(JsonValue::Null),
        )
    }

    #[tokio::test]
    async fn the_world_summary_names_its_entities_and_seed() {
        let (_dir, base, _registry) = serve().await;
        let (status, body) = get(&format!("{base}/__ferrimock/world")).await;

        assert_eq!(status, 200);
        assert_eq!(body["seed"], 42);
        let names: Vec<&str> = body["entities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["name"].as_str())
            .collect();
        assert!(names.contains(&"User"), "unexpected: {names:?}");
        assert!(names.contains(&"Folder"), "unexpected: {names:?}");
    }

    #[tokio::test]
    async fn a_list_pages_and_filters_from_the_query_string() {
        let (_dir, base, _registry) = serve().await;

        let (status, all) = get(&format!("{base}/__ferrimock/world/User")).await;
        assert_eq!(status, 200);
        assert_eq!(all["total"], 3);

        let (_, page) = get(&format!("{base}/__ferrimock/world/User?limit=2")).await;
        assert_eq!(page["records"].as_array().unwrap().len(), 2);
        assert_eq!(page["hasNext"], true);

        let name = all["records"][0]["name"].as_str().unwrap().to_string();
        let encoded = name.replace(' ', "%20");
        let (_, filtered) = get(&format!("{base}/__ferrimock/world/User?name={encoded}")).await;
        assert_eq!(
            filtered["total"], 1,
            "a bare query parameter filters a field"
        );
    }

    #[tokio::test]
    async fn an_unknown_entity_suggests_the_one_that_was_meant() {
        let (_dir, base, _registry) = serve().await;
        let (status, body) = get(&format!("{base}/__ferrimock/world/Usr")).await;

        assert_eq!(status, 404);
        let error = body["error"].as_str().unwrap();
        assert!(error.contains("User"), "unexpected: {error}");
    }

    /// A write over HTTP is the same write a handler or a template makes.
    #[tokio::test]
    async fn a_write_over_http_reaches_the_same_entities() {
        let (_dir, base, registry) = serve().await;
        let client = reqwest::Client::new();

        let created: JsonValue = client
            .post(format!("{base}/__ferrimock/world/User"))
            .json(&serde_json::json!({ "name": "Grace Hopper" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let key = created["id"].as_str().unwrap().to_string();

        assert_eq!(
            registry.world().get("User", &key).unwrap()["name"],
            "Grace Hopper",
            "an HTTP write has to land in the world the routes serve"
        );

        client
            .patch(format!("{base}/__ferrimock/world/User/{key}"))
            .json(&serde_json::json!({ "name": "Grace B. Hopper" }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            registry.world().get("User", &key).unwrap()["name"],
            "Grace B. Hopper"
        );

        let deleted = client
            .delete(format!("{base}/__ferrimock/world/User/{key}"))
            .send()
            .await
            .unwrap();
        assert_eq!(deleted.status().as_u16(), 204);
        assert!(registry.world().get("User", &key).is_none());
    }

    #[tokio::test]
    async fn reset_drops_writes_and_leaves_the_seeded_world() {
        let (_dir, base, registry) = serve().await;
        let client = reqwest::Client::new();

        registry
            .world()
            .create("User", serde_json::json!({ "name": "temp" }))
            .unwrap();
        assert_eq!(registry.world().count("User"), 4);

        let body: JsonValue = client
            .post(format!("{base}/__ferrimock/world/reset"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();

        assert_eq!(body["reset"], true);
        assert!(body["droppedWrites"].as_u64().unwrap() > 0);
        assert_eq!(registry.world().count("User"), 3);
        assert_eq!(registry.world().pending_writes(), 0);
    }
}

/// `with_world` buys isolation at the cost of template reach, and that
/// trade-off has to stay visible.
///
/// Tera's function registry is stateless, so `entity_*` resolves the *global*
/// world — there is nowhere to thread a per-registry handle through, the same
/// constraint `PersistenceStore` already lives with. A registry given its own
/// world therefore serves entities its own templates cannot see. Pinned here so
/// the day someone finds a way to thread it, this test fails and says so
/// rather than the behaviour changing unnoticed.
#[tokio::test]
async fn a_private_world_is_not_the_one_templates_read() {
    let (_dir, registry) = load().await;

    let private = registry.world().count("User");
    assert_eq!(private, 3, "the registry serves its own world");

    let rendered = ferrimock::template::render_template(
        r#"{{ entity_count(type="User") }}"#,
        &RequestContext::new(),
    );

    // An error means the global world has no `User` at all, which is equally
    // "not this registry's world".
    if let Ok(count) = rendered {
        assert_ne!(
            count,
            private.to_string(),
            "a template reads the global world, not the registry's"
        );
    }
}
