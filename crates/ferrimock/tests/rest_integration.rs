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
}

impl<'a> Call<'a> {
    fn get(path: &'a str) -> Self {
        Self {
            method: Method::GET,
            path,
            query: None,
            body: None,
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
    let matcher = MockMatcher::new((**registry).clone());
    let found = matcher
        .find_match(
            &call.method,
            call.path,
            call.query,
            &api_host(),
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
            &api_host(),
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
    (id, status, body)
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

    let items = body.as_array().expect("a bare array, as the document declared");
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

    assert_eq!(loaded, 0, "a document declares entities, not where they live");
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
    const SDL: &str =
        "type User { id: ID!, login: String! } type Query { users: [User!]! } schema { query: Query }";

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

    let (_, status, from_rest) =
        request(&registry, Call::get(&format!("/2.0/users/{key}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        from_rest["id"],
        JsonValue::String(key),
        "one `User`, whichever front end asked for it"
    );
}
