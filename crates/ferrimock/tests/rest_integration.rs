//! An OpenAPI document, end to end.
//!
//! A directory holding a document, a collection that says where it is served,
//! a hand-written override and a template — loaded through the one seam every
//! consumer uses, and asserted through ordinary matching. The claim under test
//! is the same one `world_integration` makes for GraphQL: these are not two
//! systems. The difference is that a document designs many endpoints, so each
//! one mounts as its own mock and is individually overridable, countable and
//! verifiable.

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
use http::{HeaderMap, Method, StatusCode};
use serde_json::Value as JsonValue;

const DOCUMENT: &str = r#"
openapi: 3.0.3
info:
  title: Filestore Content API
  version: "2.0"
servers:
  - url: https://api.example.com/2.0
paths:
  /folders:
    get:
      operationId: listFolders
      parameters:
        - { name: limit, in: query, schema: { type: integer } }
        - { name: offset, in: query, schema: { type: integer } }
        - { name: name, in: query, schema: { type: string } }
      responses:
        "200":
          content:
            application/json:
              schema:
                type: object
                properties:
                  entries:
                    type: array
                    items: { $ref: '#/components/schemas/Folder' }
                  total_count: { type: integer }
                  limit: { type: integer }
                  offset: { type: integer }
    post:
      operationId: createFolder
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Folder' }
      responses:
        "201":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
    delete:
      operationId: deleteFolder
      responses:
        "204": { description: deleted }
  /folders/{folder_id}/items:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: listFolderItems
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/File' }
  /files/{file_id}:
    parameters:
      - { name: file_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getFile
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/File' }
  /users/{user_id}:
    parameters:
      - { name: user_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getUser
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
  /files/content:
    post:
      operationId: uploadFile
      responses:
        "201":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/File' }
components:
  schemas:
    Folder:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
        size: { type: integer }
        user_id: { type: string }
    File:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
    User:
      type: object
      required: [id]
      properties:
        id: { type: string }
        login: { type: string }
"#;

const COLLECTION: &str = r#"
name: Filestore API
world:
  schemas:
    - filestore.openapi.yaml
  seed: 42
  counts:
    User: 3
    Folder: 5
    File: 9

mocks:
  # The whole document, at an absolute base URL a proxy can front.
  - id: filestore-rest
    match:
      url: https://api.example.com/2.0
    serve: rest

  # One endpoint forced to fail; the rest of the document keeps serving.
  - id: uploads-down
    match:
      POST: https://api.example.com/2.0/files/content
    response:
      status: 503
      json:
        message: Uploads are down

  # A declarative template reading the same entities the document serves.
  - id: folder-count
    match:
      GET: https://api.example.com/2.0/stats/folders
    response:
      template: '{"total": {{ entity_count(type="Folder") }}}'
"#;

/// A registry over its own world, so these tests do not race the
/// process-global one that templates reach.
async fn load() -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("filestore.openapi.yaml"), DOCUMENT).unwrap();
    std::fs::write(dir.path().join("mocks.yaml"), COLLECTION).unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

/// The request a server actually receives for `https://api.example.com/2.0/…`:
/// the path, plus a Host header. Never the whole URL.
fn api_host() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(http::header::HOST, "api.example.com".parse().unwrap());
    headers
}

struct Call<'a> {
    method: Method,
    path: &'a str,
    query: Option<&'a str>,
    body: Option<&'a str>,
    /// What the request says about who is asking.
    credential: Option<String>,
    /// Conditional and idempotency headers, as sent.
    sent: Vec<(&'static str, String)>,
}

impl<'a> Call<'a> {
    fn get(path: &'a str) -> Self {
        Self {
            method: Method::GET,
            path,
            query: None,
            body: None,
            credential: None,
            sent: Vec::new(),
        }
    }

    fn presenting(mut self, token: &str) -> Self {
        self.credential = Some(format!("Bearer {token}"));
        self
    }

    fn sending(mut self, name: &'static str, value: &str) -> Self {
        self.sent.push((name, value.to_string()));
        self
    }

    fn delete(path: &'a str) -> Self {
        Self {
            method: Method::DELETE,
            path,
            query: None,
            body: None,
            credential: None,
            sent: Vec::new(),
        }
    }

    fn put(path: &'a str, body: &'a str) -> Self {
        Self {
            method: Method::PUT,
            path,
            query: None,
            body: Some(body),
            credential: None,
            sent: Vec::new(),
        }
    }

    fn post(path: &'a str, body: &'a str) -> Self {
        Self {
            method: Method::POST,
            path,
            query: None,
            body: Some(body),
            credential: None,
            sent: Vec::new(),
        }
    }

    fn with_query(mut self, query: &'a str) -> Self {
        self.query = Some(query);
        self
    }
}

/// One request through the matcher and the matched mock, the way the server
/// does it — including the URL captures, which is how a path parameter reaches
/// the operation at all.
async fn request(registry: &Arc<MockRegistry>, call: Call<'_>) -> (String, StatusCode, JsonValue) {
    let (id, status, body, _) = answered(registry, call).await;
    (id, status, body)
}

/// The same call, keeping the headers the answer carried.
async fn answered(
    registry: &Arc<MockRegistry>,
    call: Call<'_>,
) -> (
    String,
    StatusCode,
    JsonValue,
    rustc_hash::FxHashMap<String, String>,
) {
    let mut sent = api_host();
    if let Some(credential) = &call.credential {
        sent.insert(
            http::header::AUTHORIZATION,
            credential.parse().expect("a header value"),
        );
    }
    for (name, value) in &call.sent {
        sent.insert(
            http::HeaderName::from_static(name),
            value.parse().expect("a header value"),
        );
    }

    let matcher = MockMatcher::new((**registry).clone());
    let found = matcher
        .find_match(
            &call.method,
            call.path,
            call.query,
            &sent,
            call.body.map(str::as_bytes),
        )
        .unwrap_or_else(|| panic!("no mock matches {} {}", call.method, call.path));

    let id = found.mock.id.to_string();
    let response = found
        .mock
        .response
        .generate_dynamic(
            call.method.as_str(),
            call.path,
            call.query,
            &sent,
            call.body.map(str::as_bytes),
            found.captures,
            found.mock.vars.as_ref(),
        )
        .await
        .expect("the matched mock renders");

    // A dynamic response may override the status; otherwise the definition's
    // own is what the server would send.
    let status = response.status.unwrap_or(found.mock.response.status);
    let body = if response.body.is_empty() {
        JsonValue::Null
    } else {
        serde_json::from_slice(&response.body).expect("a JSON body")
    };
    (id, status, body, response.headers.unwrap_or_default())
}

async fn first_folder_key(registry: &Arc<MockRegistry>) -> String {
    let (_, _, body) = request(registry, Call::get("/2.0/folders")).await;
    body["entries"][0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_document_mounts_one_mock_per_operation() {
    let (_dir, registry) = load().await;

    let mut mounted: Vec<String> = registry
        .get_all_mocks()
        .iter()
        .filter(|mock| mock.id.starts_with("filestore-rest#"))
        .map(|mock| mock.id.to_string())
        .collect();
    mounted.sort();

    assert_eq!(
        mounted,
        [
            "filestore-rest#createFolder",
            "filestore-rest#deleteFolder",
            "filestore-rest#getFile",
            "filestore-rest#getFolder",
            "filestore-rest#getUser",
            "filestore-rest#listFolderItems",
            "filestore-rest#listFolders",
            "filestore-rest#uploadFile",
        ],
        "coverage names the endpoints only if each one is its own mock"
    );
}

#[tokio::test]
async fn an_operation_answers_at_the_base_url_the_mock_names() {
    let (_dir, registry) = load().await;

    let (id, status, body) = request(&registry, Call::get("/2.0/folders")).await;
    assert_eq!(id, "filestore-rest#listFolders");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["entries"].as_array().unwrap().len(),
        5,
        "world.counts must reach the served document"
    );
    assert_eq!(body["total_count"], 5);
}

#[tokio::test]
async fn a_path_parameter_reaches_the_operation_as_a_key() {
    let (_dir, registry) = load().await;
    let key = first_folder_key(&registry).await;

    let (id, status, body) = request(&registry, Call::get(&format!("/2.0/folders/{key}"))).await;
    assert_eq!(id, "filestore-rest#getFolder");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], JsonValue::String(key));
}

#[tokio::test]
async fn a_document_does_not_serve_a_path_nobody_named() {
    let (_dir, registry) = load().await;
    let matcher = MockMatcher::new((*registry).clone());

    assert!(
        matcher
            .find_match(&Method::GET, "/folders", None, &api_host(), None)
            .is_none(),
        "the base URL is part of where the document answers"
    );
}

/// The reason an absolute `match.url` splits into a path and a Host matcher:
/// a proxy fronting several hosts must not answer for the wrong one.
#[tokio::test]
async fn a_document_does_not_serve_another_host_on_the_same_path() {
    let (_dir, registry) = load().await;
    let matcher = MockMatcher::new((*registry).clone());

    let mut elsewhere = HeaderMap::new();
    elsewhere.insert(http::header::HOST, "evil.example.com".parse().unwrap());

    assert!(
        matcher
            .find_match(&Method::GET, "/2.0/folders", None, &elsewhere, None)
            .is_none(),
        "the Host an absolute URL named has to be part of matching"
    );
}

#[tokio::test]
async fn a_hand_written_mock_overrides_exactly_one_endpoint() {
    let (_dir, registry) = load().await;

    let (id, status, body) = request(
        &registry,
        Call {
            method: Method::POST,
            path: "/2.0/files/content",
            query: None,
            body: Some("{}"),
            credential: None,
            sent: Vec::new(),
        },
    )
    .await;
    assert_eq!(id, "uploads-down", "the higher-priority mock has to win");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["message"], "Uploads are down");

    // And only that endpoint.
    let (id, status, _) = request(&registry, Call::get("/2.0/folders")).await;
    assert_eq!(id, "filestore-rest#listFolders");
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn overrides_and_mounted_operations_are_both_ordinary_mocks() {
    let (_dir, registry) = load().await;

    let served = registry
        .get_mock("filestore-rest#listFolders")
        .expect("the mounted operation");
    let override_mock = registry.get_mock("uploads-down").expect("the override");

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
            .map(|path| path.ends_with("mocks.yaml")),
        Some(true),
        "a mounted operation is tracked to the collection that declared it, not \
         to the document — reloading the document must not rebuild routes from a \
         file that declares none"
    );
}

/// The deepest claim: the store is not private to the document.
#[tokio::test]
async fn a_write_through_the_world_is_visible_to_the_served_document() {
    let (_dir, registry) = load().await;

    let created = registry
        .world()
        .create("Folder", serde_json::json!({ "name": "Ada Lovelace" }))
        .unwrap();
    let key = created["id"].as_str().unwrap().to_string();

    let (_, status, body) = request(&registry, Call::get(&format!("/2.0/folders/{key}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Ada Lovelace");
}

/// And the other direction: a write the document serves is visible outside it.
#[tokio::test]
async fn a_write_through_the_served_document_is_visible_to_the_world() {
    let (_dir, registry) = load().await;
    let before = registry.world().count("Folder");

    let (id, status, body) = request(
        &registry,
        Call {
            method: Method::POST,
            path: "/2.0/folders",
            query: None,
            body: Some(r#"{"name":"Reports"}"#),
            credential: None,
            sent: Vec::new(),
        },
    )
    .await;
    assert_eq!(id, "filestore-rest#createFolder");
    assert_eq!(status, StatusCode::CREATED, "the document declared 201");

    let key = body["id"].as_str().unwrap();
    assert_eq!(registry.world().count("Folder"), before + 1);
    assert_eq!(
        registry.world().get("Folder", key).unwrap()["name"],
        "Reports"
    );
}

#[tokio::test]
async fn a_delete_through_the_document_removes_the_record() {
    let (_dir, registry) = load().await;
    let key = first_folder_key(&registry).await;

    let (id, status, body) = request(
        &registry,
        Call {
            method: Method::DELETE,
            path: &format!("/2.0/folders/{key}"),
            query: None,
            body: None,
            credential: None,
            sent: Vec::new(),
        },
    )
    .await;
    assert_eq!(id, "filestore-rest#deleteFolder");
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(body, JsonValue::Null, "204 means no body");
    assert!(registry.world().get("Folder", &key).is_none());
}

#[tokio::test]
async fn a_nested_path_serves_the_parents_children() {
    let (_dir, registry) = load().await;
    let key = first_folder_key(&registry).await;

    let (id, status, body) =
        request(&registry, Call::get(&format!("/2.0/folders/{key}/items"))).await;
    assert_eq!(id, "filestore-rest#listFolderItems");
    assert_eq!(status, StatusCode::OK);

    let items = body
        .as_array()
        .expect("a bare array, as the document declared");
    let expected = registry
        .world()
        .related(
            "Folder",
            &key,
            "items",
            &ferrimock::core::EntityQuery::default(),
        )
        .unwrap();
    assert_eq!(
        items.len(),
        expected.records.len(),
        "the route and the world have to agree about who owns what"
    );
}

#[tokio::test]
async fn query_parameters_page_and_filter_the_same_collection() {
    let (_dir, registry) = load().await;

    let (_, _, page) = request(
        &registry,
        Call::get("/2.0/folders").with_query("limit=2&offset=1"),
    )
    .await;
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);
    assert_eq!(page["total_count"], 5, "the total is the whole collection");
    assert_eq!(page["offset"], 1);

    let (_, _, all) = request(&registry, Call::get("/2.0/folders")).await;
    let name = all["entries"][3]["name"].as_str().unwrap().to_string();
    let (_, _, filtered) = request(
        &registry,
        Call::get("/2.0/folders").with_query(&format!("name={name}")),
    )
    .await;
    assert_eq!(filtered["entries"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["entries"][0]["name"], JsonValue::String(name));
}

#[tokio::test]
async fn a_template_reads_the_same_entities_the_document_serves() {
    let (_dir, registry) = load().await;

    // `entity_count` reads the *global* world, so this asserts the route and
    // the template agree in shape rather than in count.
    let (id, status, body) = request(&registry, Call::get("/2.0/stats/folders")).await;
    assert_eq!(id, "folder-count");
    assert_eq!(status, StatusCode::OK);
    assert!(body["total"].is_number());
}

#[tokio::test]
async fn coverage_and_verify_work_one_endpoint_at_a_time() {
    let (_dir, registry) = load().await;

    request(&registry, Call::get("/2.0/folders")).await;
    request(&registry, Call::get("/2.0/folders")).await;
    let key = first_folder_key(&registry).await;
    request(&registry, Call::get(&format!("/2.0/folders/{key}"))).await;

    registry
        .verify("filestore-rest#listFolders", Expected::Exactly(3))
        .expect("a mounted operation counts its own matches");
    registry
        .verify("filestore-rest#getFolder", Expected::Exactly(1))
        .expect("and only its own");
    registry
        .verify("filestore-rest#deleteFolder", Expected::Never)
        .expect("an endpoint nobody called reads as never");

    let coverage = registry.coverage();
    assert!(
        coverage
            .unused
            .iter()
            .any(|entry| entry.mock_id == "filestore-rest#deleteFolder"),
        "an endpoint that never served a request has to show up in coverage"
    );
    assert!(
        coverage
            .served
            .iter()
            .any(|entry| entry.mock_id == "filestore-rest#listFolders"),
    );
}

#[tokio::test]
async fn a_document_with_no_mock_serving_it_registers_no_routes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("filestore.openapi.yaml"), DOCUMENT).unwrap();

    let registry = MockRegistry::with_world(Arc::new(World::new()));
    let loaded = registry.load_from_directory(dir.path()).await.unwrap();

    assert_eq!(
        loaded, 0,
        "a document declares entities, not where they live"
    );
    assert!(!registry.world().is_empty());
    assert!(registry.world().count("Folder") > 0);
}

#[tokio::test]
async fn a_method_on_the_mount_is_refused_because_operations_carry_their_own() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("filestore.openapi.yaml"), DOCUMENT).unwrap();
    std::fs::write(
        dir.path().join("mocks.yaml"),
        "world:\n  schemas: [filestore.openapi.yaml]\nmocks:\n  - id: filestore-rest\n    match:\n      \
         GET: https://api.example.com/2.0\n    serve: rest\n",
    )
    .unwrap();

    let registry = MockRegistry::with_world(Arc::new(World::new()));
    registry.load_from_directory(dir.path()).await.unwrap();

    assert!(
        registry
            .get_all_mocks()
            .iter()
            .all(|mock| !mock.id.starts_with("filestore-rest")),
        "a mount naming a method would either contradict the operations or be ignored"
    );
}

#[tokio::test]
async fn a_graphql_schema_and_a_document_serve_the_same_entities() {
    const SDL: &str = "type User { id: ID!, login: String! } type Query { users: [User!]! } schema { query: Query }";

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("filestore.openapi.yaml"), DOCUMENT).unwrap();
    std::fs::write(dir.path().join("schema.graphql"), SDL).unwrap();
    std::fs::write(
        dir.path().join("mocks.yaml"),
        "world:\n  seed: 42\n  counts: { User: 3 }\nmocks:\n  - id: filestore-rest\n    match:\n      \
         url: https://api.example.com/2.0\n    serve: rest\n  - id: filestore-graphql\n    match:\n      \
         POST: https://api.example.com/graphql\n    serve: graphql\n",
    )
    .unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();

    // Both bindings resolve, each to its own protocol, from one world.
    assert!(registry.get_mock("filestore-graphql").is_some());
    assert!(registry.get_mock("filestore-rest#getUser").is_some());

    let key = registry
        .world()
        .list("User", &ferrimock::core::EntityQuery::default())
        .unwrap()
        .records[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let (_, status, from_rest) = request(&registry, Call::get(&format!("/2.0/users/{key}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        from_rest["id"],
        JsonValue::String(key),
        "one `User`, whichever front end asked for it"
    );
}

// ===== Regressions =====

/// A document keyed by integers, which is most of them.
const NUMBERED: &str = r#"
openapi: 3.0.3
info: { title: Shop, version: "1.0" }
paths:
  /orders:
    get:
      operationId: listOrders
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Order' }
    post:
      operationId: createOrder
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Order' }
      responses:
        "201":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Order' }
  /orders/{order_id}:
    parameters:
      - { name: order_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: getOrder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Order' }
  /users/{user_id}:
    parameters:
      - { name: user_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: getUser
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
components:
  schemas:
    Order:
      type: object
      required: [id]
      properties:
        id: { type: integer }
        user_id: { type: integer }
        total: { type: number }
        customer: { $ref: '#/components/schemas/User' }
    User:
      type: object
      required: [id, slug]
      properties:
        id: { type: integer }
        slug: { type: string }
        sku: { type: string, pattern: "^[A-Z]{3}-[0-9]{4}$" }
"#;

const NUMBERED_COLLECTION: &str = r"
name: Shop
world:
  schemas:
    - shop.openapi.yaml
  seed: 7
  counts:
    User: 4
    Order: 4

mocks:
  - id: shop
    match:
      url: https://api.example.com
    serve: rest
";

async fn load_numbered() -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shop.openapi.yaml"), NUMBERED).unwrap();
    std::fs::write(dir.path().join("mocks.yaml"), NUMBERED_COLLECTION).unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

#[tokio::test]
async fn a_document_keyed_by_integers_answers_with_integers() {
    let (_dir, registry) = load_numbered().await;

    let (_, status, body) = request(&registry, Call::get("/orders")).await;
    assert_eq!(status, StatusCode::OK);
    for order in body.as_array().unwrap() {
        assert!(
            order["id"].is_i64(),
            "`id: {{ type: integer }}` has to answer with an integer, got {}",
            order["id"]
        );
        assert!(order["user_id"].is_i64(), "so does a foreign key");
    }

    // And the ids a client would actually try are the ones that resolve.
    let (_, status, one) = request(&registry, Call::get("/orders/1")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "`GET /orders/1` on an integer-keyed document must not be a 404"
    );
    assert_eq!(one["id"], JsonValue::from(1));
}

#[tokio::test]
async fn a_foreign_key_and_the_object_it_carries_name_one_user() {
    let (_dir, registry) = load_numbered().await;

    let (_, _, body) = request(&registry, Call::get("/orders")).await;
    for order in body.as_array().unwrap() {
        assert_eq!(
            order["user_id"], order["customer"]["id"],
            "a client filtering by the key and rendering the object sees one user"
        );
    }
}

#[tokio::test]
async fn a_created_record_carries_the_same_links_a_seeded_one_does() {
    let (_dir, registry) = load_numbered().await;

    let (_, status, created) = request(
        &registry,
        Call::post("/orders", r#"{"user_id": 3, "total": 11}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["user_id"], JsonValue::from(3));
    assert_eq!(
        created["customer"]["id"],
        JsonValue::from(3),
        "the link a creation stated has to resolve, not answer null"
    );

    // Read back through the document, not just the creation's own response.
    let id = created["id"].as_i64().unwrap();
    let (_, status, fetched) = request(&registry, Call::get(&format!("/orders/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(fetched["customer"]["id"], JsonValue::from(3));
}

#[tokio::test]
async fn a_page_number_past_what_an_offset_holds_is_answered_not_panicked() {
    let (_dir, registry) = load_numbered().await;

    let (_, status, body) = request(
        &registry,
        Call::get("/orders").with_query("page=18446744073709551615&limit=100"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0, "past the end is empty");
}

#[tokio::test]
async fn declared_string_shapes_are_honoured() {
    let (_dir, registry) = load_numbered().await;

    let (_, _, user) = request(&registry, Call::get("/users/1")).await;
    let slug = user["slug"].as_str().unwrap();
    assert!(
        !slug.contains(' '),
        "a `slug` field holds a slug, not a sentence: {slug}"
    );

    let sku = user["sku"].as_str().unwrap();
    let pattern = regex::Regex::new("^[A-Z]{3}-[0-9]{4}$").unwrap();
    assert!(
        pattern.is_match(sku),
        "a declared pattern is a promise: {sku}"
    );
}

/// A document that both declares a collection inline and offers a sub-path for
/// it, which is what most real ones do.
const NESTED: &str = r"
openapi: 3.0.3
info: { title: Workspace, version: '1.0' }
paths:
  /folders:
    get:
      operationId: listFolders
      responses:
        '200':
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: getFolder
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}/files:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: listFolderFiles
      responses:
        '200':
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/File' }
components:
  schemas:
    Folder:
      type: object
      required: [id, name]
      properties:
        id: { type: integer }
        name: { type: string }
        file_count: { type: integer }
        created_at: { type: string, format: date-time }
        updated_at: { type: string, format: date-time }
        files:
          type: array
          items: { $ref: '#/components/schemas/File' }
    File:
      type: object
      required: [id]
      properties:
        id: { type: integer }
        name: { type: string }
        folder: { $ref: '#/components/schemas/Folder' }
";

const NESTED_COLLECTION: &str = r"
name: Workspace
world:
  schemas:
    - workspace.openapi.yaml
  seed: 5
  counts:
    Folder: 6
    File: 40

mocks:
  - id: ws
    match:
      url: https://api.example.com
    serve: rest
";

async fn load_nested() -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("workspace.openapi.yaml"), NESTED).unwrap();
    std::fs::write(dir.path().join("mocks.yaml"), NESTED_COLLECTION).unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

#[tokio::test]
async fn a_sub_path_serves_the_collection_the_schema_declared() {
    let (_dir, registry) = load_nested().await;

    for id in 1..=6 {
        let (_, _, folder) = request(&registry, Call::get(&format!("/folders/{id}"))).await;
        let (_, _, files) = request(&registry, Call::get(&format!("/folders/{id}/files"))).await;
        let served = files.as_array().unwrap();

        assert_eq!(
            usize::try_from(folder["file_count"].as_u64().unwrap()).unwrap(),
            served.len(),
            "`file_count` has to agree with what the sub-path serves"
        );
        for file in served {
            assert_eq!(
                file["folder"]["id"], folder["id"],
                "a sub-path must serve that parent's children, not every child"
            );
        }
    }
}

#[tokio::test]
async fn children_are_shared_out_unevenly_across_parents() {
    let (_dir, registry) = load_nested().await;

    let mut sizes = Vec::new();
    for id in 1..=6 {
        let (_, _, folder) = request(&registry, Call::get(&format!("/folders/{id}"))).await;
        sizes.push(folder["file_count"].as_u64().unwrap());
    }

    assert_eq!(sizes.iter().sum::<u64>(), 40, "every file is in one folder");
    let busiest = sizes.iter().copied().max().unwrap();
    assert!(
        busiest >= 40 / 6 * 2,
        "real data is lopsided, and this is {sizes:?}"
    );
}

#[tokio::test]
async fn a_record_reads_like_something_a_product_wrote() {
    let (_dir, registry) = load_nested().await;

    let (_, _, folder) = request(&registry, Call::get("/folders/1")).await;

    let name = folder["name"].as_str().unwrap();
    assert!(
        !name.contains(' ') || name.split(' ').count() <= 4,
        "a folder name is a short phrase, not prose: {name}"
    );

    let created = folder["created_at"].as_str().unwrap();
    let updated = folder["updated_at"].as_str().unwrap();
    assert!(
        created <= updated,
        "a folder cannot be updated before it was created: {created} then {updated}"
    );
}

// ===== Field overrides =====

const OVERRIDDEN: &str = r"
openapi: 3.0.3
info: { title: Workspace, version: '1.0' }
paths:
  /folders:
    get:
      operationId: listFolders
      responses:
        '200':
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Folder' }
    post:
      operationId: createFolder
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Folder' }
      responses:
        '201':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: getFolder
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
components:
  schemas:
    Folder:
      type: object
      required: [id]
      properties:
        id: { type: integer }
        name: { type: string }
        status: { type: string }
        budget: { type: number, format: money }
        badge: { type: string }
";

const OVERRIDDEN_COLLECTION: &str = r#"
name: Workspace
world:
  schemas:
    - workspace.openapi.yaml
  seed: 5
  counts:
    Folder: 6
  scalars:
    money: { float: { min: 100, max: 999 } }
  fields:
    Folder.status: { one_of: [active, archived, pending] }
    Folder.badge: "{{ fake_word() | upper }}"
    "*.name": headline

mocks:
  - id: ws
    match:
      url: https://api.example.com
    serve: rest
"#;

async fn load_overridden() -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("workspace.openapi.yaml"), OVERRIDDEN).unwrap();
    std::fs::write(dir.path().join("mocks.yaml"), OVERRIDDEN_COLLECTION).unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

#[tokio::test]
async fn an_override_decides_what_a_field_of_an_openapi_document_holds() {
    let (_dir, registry) = load_overridden().await;

    // Only `id` is required, so the rest are allowed to be missing — the
    // override decides what a field holds when it holds anything.
    let (_, _, folders) = request(&registry, Call::get("/folders")).await;
    let mut seen = 0;
    for folder in folders.as_array().unwrap() {
        if let Some(status) = folder.get("status").and_then(JsonValue::as_str) {
            assert!(
                ["active", "archived", "pending"].contains(&status),
                "`one_of` decides the set: {status}"
            );
            seen += 1;
        }
        // Keyed on the OpenAPI `format`, not on a field name.
        if let Some(budget) = folder.get("budget").and_then(JsonValue::as_f64) {
            assert!((100.0..=999.0).contains(&budget), "budget {budget}");
        }
        if let Some(badge) = folder.get("badge").and_then(JsonValue::as_str) {
            assert_eq!(badge, badge.to_uppercase(), "the template ran: {badge}");
            assert!(!badge.is_empty());
        }
    }
    assert!(seen > 0, "the override should reach some folder");
}

#[tokio::test]
async fn an_override_applies_to_a_record_the_client_created() {
    let (_dir, registry) = load_overridden().await;

    let (_, status, created) =
        request(&registry, Call::post("/folders", r#"{"name":"Mine"}"#)).await;
    assert_eq!(status, StatusCode::CREATED);

    assert_eq!(
        created["name"],
        JsonValue::String("Mine".into()),
        "what was written stands"
    );
    if let Some(state) = created.get("status").and_then(JsonValue::as_str) {
        assert!(
            ["active", "archived", "pending"].contains(&state),
            "a created record obeys the same rules as a seeded one: {state}"
        );
    }
    if let Some(budget) = created.get("budget").and_then(JsonValue::as_f64) {
        assert!((100.0..=999.0).contains(&budget), "budget {budget}");
    }
    if let Some(badge) = created.get("badge").and_then(JsonValue::as_str) {
        assert_eq!(badge, badge.to_uppercase());
    }
}

#[tokio::test]
async fn an_override_does_not_disturb_determinism() {
    let (_dir, first) = load_overridden().await;
    let (_, _, once) = request(&first, Call::get("/folders")).await;

    let (_dir2, second) = load_overridden().await;
    let (_, _, twice) = request(&second, Call::get("/folders")).await;

    assert_eq!(once, twice, "the same seed still rebuilds the same world");
}

const VIEWER_DOC: &str = "
openapi: 3.0.3
info: { title: Directory, version: '1' }
paths:
  /me:
    get:
      operationId: getCurrentUser
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
  /users:
    get:
      operationId: listUsers
      responses:
        '200':
          content:
            application/json:
              schema: { type: array, items: { $ref: '#/components/schemas/User' } }
  /users/{user_id}:
    parameters:
      - { name: user_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getUser
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
components:
  schemas:
    User:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
";

async fn load_viewer(bound: bool) -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("directory.openapi.yaml"), VIEWER_DOC).unwrap();
    let viewer = if bound { "  viewer: User\n" } else { "" };
    std::fs::write(
        dir.path().join("mocks.yaml"),
        format!(
            "name: Directory\nworld:\n  schemas:\n    - directory.openapi.yaml\n  seed: 5\n{viewer}\nmocks:\n  - id: dir\n    match:\n      url: https://api.example.com\n    serve: rest\n"
        ),
    )
    .unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

/// `/me` answered with record zero, for every caller, with or without a token
/// — the one endpoint whose whole purpose is to differ per caller.
#[tokio::test]
async fn the_viewer_is_whoever_presented_the_credential() {
    let (_dir, registry) = load_viewer(true).await;

    let call = |token: &str| Call::get("/me").presenting(token);

    let (_, status, mine) = request(&registry, call("alice")).await;
    assert_eq!(status, StatusCode::OK);
    let (_, _, again) = request(&registry, call("alice")).await;
    assert_eq!(mine["id"], again["id"], "one token is one person");

    let mut landed = std::collections::BTreeSet::new();
    for token in 0..20 {
        let (_, _, who) = request(&registry, call(&format!("t{token}"))).await;
        landed.insert(who["id"].as_str().unwrap_or_default().to_string());
    }
    assert!(
        landed.len() > 5,
        "every caller was the same person: {landed:?}"
    );

    let (_, _, everyone) = request(&registry, Call::get("/users")).await;
    assert!(
        everyone
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|user| user["id"].as_str())
            .any(|id| Some(id) == mine["id"].as_str()),
        "the viewer has to be one of the world's own users"
    );
}

#[tokio::test]
async fn no_credential_is_a_401_that_says_what_to_present() {
    let (_dir, registry) = load_viewer(true).await;

    let (_, status, _, headers) = answered(&registry, Call::get("/me")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        headers.iter().any(
            |(name, value)| name.eq_ignore_ascii_case("www-authenticate")
                && value.contains("Bearer")
        ),
        "a client that retries on 401 reads the scheme out of the header: {headers:?}"
    );
}

/// With nothing bound, the endpoint is not answerable: the schema says a
/// `User` comes back and nothing says which. Counted rather than answered
/// wrongly.
#[tokio::test]
async fn an_unbound_viewer_is_counted_rather_than_guessed() {
    let (_dir, registry) = load_viewer(false).await;

    let (_, status, _) = request(&registry, Call::get("/me")).await;
    assert_eq!(status, StatusCode::OK, "the declared shape still answers");

    let world = registry.world();
    let schema = world.schemas().into_iter().next().unwrap();
    let ferrimock::core::world::Binding::OpenApi(table) = &schema.binding else {
        panic!("an openapi document")
    };
    let backend = ferrimock::spec::bind::rest::RestBackend::build(table, world);
    assert!(
        backend
            .coverage()
            .unclassified()
            .iter()
            .any(|id| id.contains("getCurrentUser")),
        "an unbound viewer is not classified: {:?}",
        backend.coverage().unclassified()
    );
}

const BEHAVING_DOC: &str = "
openapi: 3.0.3
info: { title: Notes, version: '1' }
paths:
  /notes:
    get:
      operationId: listNotes
      responses:
        '200':
          content:
            application/json:
              schema: { type: array, items: { $ref: '#/components/schemas/Note' } }
    post:
      operationId: createNote
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Note' }
      responses:
        '201':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Note' }
  /notes/{note_id}:
    parameters:
      - { name: note_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getNote
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Note' }
    put:
      operationId: replaceNote
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Note' }
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Note' }
    delete:
      operationId: deleteNote
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Note' }
components:
  schemas:
    Note:
      type: object
      required: [id, title]
      properties:
        id: { type: string }
        title: { type: string }
";

async fn load_behaving(behaviour: &str) -> (tempfile::TempDir, Arc<MockRegistry>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.openapi.yaml"), BEHAVING_DOC).unwrap();
    std::fs::write(
        dir.path().join("mocks.yaml"),
        format!(
            "name: Notes\nworld:\n  schemas:\n    - notes.openapi.yaml\n  seed: 9\n  counts:\n    Note: 5\n\nmocks:\n  - id: notes\n    match:\n      url: https://api.example.com\n    serve:\n      protocol: rest\n      behaviour: {{ {behaviour} }}\n"
        ),
    )
    .unwrap();

    let registry = Arc::new(MockRegistry::with_world(Arc::new(World::new())));
    registry.load_from_directory(dir.path()).await.unwrap();
    (dir, registry)
}

async fn first_note(registry: &Arc<MockRegistry>) -> String {
    let (_, _, notes) = request(registry, Call::get("/notes")).await;
    notes[0]["id"].as_str().unwrap().to_string()
}

/// A document declares shapes and status codes. It does not declare that a
/// second `GET` with the tag it was given answers 304 — and a client that
/// handles conditional requests has no way to exercise that against a mock
/// that does not.
#[tokio::test]
async fn a_representation_carries_a_tag_a_client_can_ask_against() {
    let (_dir, registry) = load_behaving("conditional: true").await;
    let id = first_note(&registry).await;

    let (_, status, body, headers) = answered(&registry, Call::get(&format!("/notes/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("etag"))
        .map(|(_, value)| value.clone())
        .expect("a representation carries its own tag");
    assert!(!body.is_null());

    let (_, status, again, _) = answered(
        &registry,
        Call::get(&format!("/notes/{id}")).sending("if-none-match", &etag),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(again.is_null(), "a 304 carries no body");

    let (_, status, _, _) = answered(
        &registry,
        Call::get(&format!("/notes/{id}")).sending("if-none-match", "\"something-else\""),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a tag that does not match is a read"
    );
}

#[tokio::test]
async fn a_write_against_a_version_that_moved_on_is_refused() {
    let (_dir, registry) = load_behaving("conditional: true").await;
    let id = first_note(&registry).await;

    let (_, _, _, headers) = answered(&registry, Call::get(&format!("/notes/{id}"))).await;
    let etag = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("etag"))
        .map(|(_, value)| value.clone())
        .unwrap();

    let (_, status, _, _) = answered(
        &registry,
        Call::put(&format!("/notes/{id}"), r#"{"title":"Renamed"}"#)
            .sending("if-match", "\"stale\""),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);

    let (_, status, _, _) = answered(
        &registry,
        Call::put(&format!("/notes/{id}"), r#"{"title":"Renamed"}"#).sending("if-match", &etag),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the tag it was given still holds");
}

/// 404 says try a different key. 410 says stop asking.
#[tokio::test]
async fn a_removed_record_says_it_was_removed() {
    let (_dir, registry) = load_behaving("soft_delete: true, problem_json: true").await;
    let id = first_note(&registry).await;

    let (_, status, _) = request(&registry, Call::delete(&format!("/notes/{id}"))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, status, body, headers) = answered(&registry, Call::get(&format!("/notes/{id}"))).await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(body["status"], JsonValue::from(410));
    assert_eq!(body["title"], JsonValue::from("Gone"));
    assert!(
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value.contains("problem+json")),
        "{headers:?}"
    );

    let (_, status, _) = request(&registry, Call::get("/notes/never-existed")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "nothing is not the same as gone"
    );
}

/// A `POST` answers with the record; the `GET` straight after does not list
/// it; a client that retries gets it. That is the code path a
/// read-your-writes bug lives in, and a mock with no lag can never exercise it.
#[tokio::test]
async fn a_lagging_replica_catches_up_in_writes_rather_than_seconds() {
    let (_dir, registry) = load_behaving("replica_lag: 2").await;

    let (_, status, made) = request(&registry, Call::post("/notes", r#"{"title":"Fresh"}"#)).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = made["id"].as_str().unwrap().to_string();

    let listed = |registry: &Arc<MockRegistry>, id: String| {
        let registry = Arc::clone(registry);
        async move {
            let (_, _, notes) = request(&registry, Call::get("/notes")).await;
            notes
                .as_array()
                .unwrap()
                .iter()
                .any(|note| note["id"].as_str() == Some(id.as_str()))
        }
    };

    assert!(
        !listed(&registry, id.clone()).await,
        "the replica has not caught up yet"
    );
    for _ in 0..3 {
        request(&registry, Call::post("/notes", r#"{"title":"Another"}"#)).await;
    }
    assert!(
        listed(&registry, id).await,
        "further writes are what a replica catches up on"
    );
}

/// A retry after a timeout is the case this exists for: the client never saw
/// the answer, sends the same request again, and a service without this makes
/// a second resource.
#[tokio::test]
async fn an_idempotency_key_is_answered_once() {
    let (_dir, registry) = load_behaving("idempotency: true").await;

    let send = || Call::post("/notes", r#"{"title":"Once"}"#).sending("idempotency-key", "abc-123");

    let (_, status, first) = request(&registry, send()).await;
    assert_eq!(status, StatusCode::CREATED);

    let (_, status, again, headers) = answered(&registry, send()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(first, again, "the same key gets the same answer");
    assert!(
        headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("idempotent-replay")),
        "a replay says so: {headers:?}"
    );

    let (_, _, other) = request(
        &registry,
        Call::post("/notes", r#"{"title":"Twice"}"#).sending("idempotency-key", "def-456"),
    )
    .await;
    assert_ne!(
        first["id"], other["id"],
        "a different key is a different act"
    );
}

#[tokio::test]
async fn a_creation_says_where_the_thing_it_made_now_lives() {
    let (_dir, registry) = load_behaving("").await;

    let (_, status, made, headers) =
        answered(&registry, Call::post("/notes", r#"{"title":"Fresh"}"#)).await;
    assert_eq!(status, StatusCode::CREATED);
    let location = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("location"))
        .map(|(_, value)| value.clone())
        .expect("a creation says where it went");
    assert_eq!(location, format!("/notes/{}", made["id"].as_str().unwrap()));

    let (_, status, followed) = request(&registry, Call::get(&location)).await;
    assert_eq!(status, StatusCode::OK, "and a client can follow it");
    assert_eq!(followed["id"], made["id"]);
}

/// Nothing beyond answering unless the mount asked for it.
#[tokio::test]
async fn a_mount_that_asked_for_nothing_gets_nothing() {
    let (_dir, registry) = load_behaving("").await;
    let id = first_note(&registry).await;

    let (_, status, _, headers) = answered(&registry, Call::get(&format!("/notes/{id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("etag")),
        "{headers:?}"
    );

    request(&registry, Call::delete(&format!("/notes/{id}"))).await;
    let (_, status, _) = request(&registry, Call::get(&format!("/notes/{id}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
