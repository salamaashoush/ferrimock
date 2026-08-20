//! Mounting an OpenAPI document as one ordinary mock per operation.
//!
//! GraphQL mounts as a family of one, because the client chooses the operation
//! name. A document is the opposite: it *designs* the endpoints, so each one
//! becomes its own [`MockDefinition`] at its own method and path.
//!
//! That costs a definition per operation — 500 of them for a large document —
//! and buys the three things a single glob mock cannot give: coverage that
//! names the endpoints ("214 mounted, 9 served a request"), `verify()` on one
//! endpoint, and an override that is an ordinary higher-priority mock at that
//! path rather than a special case. The matcher's exact-path index is O(1) and
//! the sorted list is built once per invalidation, so the cost is memory, not
//! per-request work.

use http::StatusCode;
use smallvec::SmallVec;
use std::sync::Arc;

use crate::config::patterns::parse_url_pattern;
use crate::spec::bind::rest::RestBackend;
use crate::spec::bind::rest::answer::BoundOperation;
use crate::types::{
    BodySource, ContextNeeds, DynamicResponse, MockDefinition, ResponseGenerator, UrlPattern,
};

/// Turn a backend into one mock per operation.
///
/// `template` arrives fully lowered — its URL is already split into a base path
/// and a Host matcher, and its priority, scope, headers and delay are the
/// mock's business rather than the document's. Every emitted mock inherits all
/// of it and differs only in method, path and behavior.
pub fn bind_rest(
    template: &MockDefinition,
    backend: &RestBackend,
) -> crate::Result<Vec<MockDefinition>> {
    let base = base_path(template)?;

    backend
        .operations
        .iter()
        .map(|operation| mount(template, &base, operation))
        .collect()
}

/// The path an operation's path is appended to.
///
/// A base has to be a literal prefix: appending `/folders/{id}` to a regex or a
/// glob would produce a pattern that matches something nobody wrote.
fn base_path(template: &MockDefinition) -> crate::Result<String> {
    match template.request.url_patterns.as_slice() {
        [] => Ok(String::new()),
        [UrlPattern::Exact(path) | UrlPattern::Prefix(path)] => {
            Ok(path.trim_end_matches('/').to_string())
        }
        [_] => Err(crate::mp_err!(
            "mock `{}`: `serve: rest` needs a plain base URL to mount operations under — \
             a regex or glob `match.url` has no path to append `/folders/{{id}}` to",
            template.id
        )),
        many => Err(crate::mp_err!(
            "mock `{}`: `serve: rest` mounts one base URL, and this one names {}",
            template.id,
            many.len()
        )),
    }
}

fn mount(
    template: &MockDefinition,
    base: &str,
    operation: &Arc<BoundOperation>,
) -> crate::Result<MockDefinition> {
    let path = format!("{base}{}", operation.path);
    let pattern = parse_url_pattern(&path).map_err(|e| {
        crate::mp_err!(
            "mock `{}`: could not mount `{} {path}`: {e}",
            template.id,
            operation.method
        )
    })?;

    let mut mock = template.clone();
    mock.id = format!("{}#{}", template.id, operation.id).into();
    mock.request.methods = SmallVec::from_elem(operation.method.clone(), 1);
    mock.request.url_patterns = SmallVec::from_elem(pattern, 1);

    let bound = Arc::clone(operation);
    let handler = move |ctx: crate::types::RequestContext| {
        let bound = Arc::clone(&bound);
        Box::pin(async move { Ok::<DynamicResponse, crate::FerrimockError>(bound.answer(&ctx)) })
            as std::pin::Pin<Box<dyn Future<Output = _> + Send>>
    };

    let headers = std::mem::take(&mut mock.response.headers);
    let delay = mock.response.delay;
    // A REST operation reads the path, the query string and the body; headers
    // are the transport's, so the matching lanes can skip marshalling them.
    // The exception is the one endpoint that answers per caller: who is asking
    // arrives in a header and nowhere else, and it is declared per operation
    // so nothing that does not ask for it pays.
    let needs = if matches!(operation.plan, crate::spec::bind::RootPlan::Viewer { .. }) {
        ContextNeeds::ALL
    } else {
        ContextNeeds::body_only()
    };
    let mut response = ResponseGenerator::new(
        StatusCode::from_u16(operation.status.as_u16()).unwrap_or(StatusCode::OK),
        BodySource::handler_with_needs(Arc::new(handler), needs),
    );
    response.headers = headers;
    response
        .headers
        .entry("content-type".to_string())
        .or_insert_with(|| operation.content_type.to_string());
    response.delay = delay;
    mock.response = response;

    Ok(mock)
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
    use crate::engine::ResponseGeneratorExt;
    use crate::spec::infer::openapi::parse_openapi;
    use crate::spec::source::load_schema;
    use crate::types::RequestMatcher;
    use lean_string::LeanString;
    use serde_json::Value as JsonValue;
    use std::path::Path;

    const DOC: &str = r#"
openapi: 3.0.3
info: { title: Filestore }
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
    patch:
      operationId: updateFolder
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/Folder' }
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
    delete:
      operationId: deleteFolder
      responses:
        "204": { description: gone }
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
  /health:
    get:
      operationId: health
      responses:
        "200":
          content:
            application/json:
              schema:
                type: object
                properties:
                  status: { type: string }
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
        parent: { $ref: '#/components/schemas/Folder' }
    File:
      type: object
      required: [id]
      properties:
        id: { type: string }
        name: { type: string }
    User:
      type: object
      properties:
        id: { type: string }
"#;

    fn backend() -> (Arc<World>, RestBackend) {
        let world = Arc::new(World::new());
        world
            .configure(
                &WorldSettings {
                    seed: Some(7),
                    counts: [
                        (LeanString::from("Folder"), 5),
                        (LeanString::from("File"), 8),
                    ]
                    .into_iter()
                    .collect(),
                    ..WorldSettings::default()
                },
                Path::new("test"),
            )
            .unwrap();
        load_schema(DOC, Path::new("filestore.openapi.yaml"), &world, false).unwrap();

        let table = Arc::new(parse_openapi(DOC).unwrap().0);
        let backend = RestBackend::build(&table, &world);
        (world, backend)
    }

    fn template(id: &str, base: &str) -> MockDefinition {
        let mut request = RequestMatcher::default();
        if !base.is_empty() {
            request.url_patterns = SmallVec::from_elem(UrlPattern::exact(base), 1);
        }
        MockDefinition {
            id: id.into(),
            priority: crate::config::serve::SERVED_PRIORITY,
            request,
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("")),
            enabled: true,
            once: false,
            scope: None,
            source_file: Some("mocks.yaml".to_string()),
            request_transforms: None,
            vars: None,
            streaming: None,
        }
    }

    fn mounted() -> (Arc<World>, Vec<MockDefinition>) {
        let (world, backend) = backend();
        let mocks = bind_rest(&template("filestore-rest", "/2.0"), &backend).unwrap();
        (world, mocks)
    }

    fn find<'a>(mocks: &'a [MockDefinition], id: &str) -> &'a MockDefinition {
        mocks
            .iter()
            .find(|mock| mock.id == id)
            .unwrap_or_else(|| panic!("no mock `{id}` among {:?}", ids(mocks)))
    }

    fn ids(mocks: &[MockDefinition]) -> Vec<String> {
        mocks.iter().map(|mock| mock.id.to_string()).collect()
    }

    /// One request, through the same entry point the matcher uses.
    #[derive(Default)]
    struct Call<'a> {
        query: Option<&'a str>,
        body: Option<&'a str>,
        captures: Vec<(&'a str, &'a str)>,
    }

    async fn answer(
        mock: &MockDefinition,
        method: &str,
        path: &str,
        call: Call<'_>,
    ) -> (StatusCode, JsonValue) {
        let captures: rustc_hash::FxHashMap<String, String> = call
            .captures
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect();

        let response = mock
            .response
            .generate_dynamic(
                method,
                path,
                call.query,
                &http::HeaderMap::new(),
                call.body.map(str::as_bytes),
                captures,
                None,
            )
            .await
            .expect("the operation answers");

        let status = response.status.unwrap_or(StatusCode::OK);
        let body = if response.body.is_empty() {
            JsonValue::Null
        } else {
            serde_json::from_slice(&response.body).expect("a JSON body")
        };
        (status, body)
    }

    fn first_folder_key(world: &Arc<World>) -> String {
        world
            .list("Folder", &crate::core::EntityQuery::default())
            .unwrap()
            .records[0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// A folder deep enough to have a grandparent.
    ///
    /// Folders are levelled, so the first one is a root and its `parent` is
    /// null by construction. A test about how deep an expansion goes has to
    /// start somewhere there is depth.
    fn nested_folder_key(world: &Arc<World>) -> String {
        let parent_of = |key: &str| -> Option<String> {
            world
                .get("Folder", key)?
                .get("parent")?
                .as_str()
                .map(str::to_string)
        };
        world
            .list("Folder", &crate::core::EntityQuery::default())
            .unwrap()
            .records
            .iter()
            .filter_map(|record| record["id"].as_str())
            .find(|key| {
                parent_of(key)
                    .and_then(|parent| parent_of(&parent))
                    .is_some()
            })
            .expect("the fixture should hold a folder two levels down")
            .to_string()
    }

    #[test]
    fn every_operation_becomes_its_own_mock() {
        let (_world, mocks) = mounted();
        let mut names = ids(&mocks);
        names.sort();
        assert_eq!(
            names,
            [
                "filestore-rest#createFolder",
                "filestore-rest#deleteFolder",
                "filestore-rest#getFile",
                "filestore-rest#getFolder",
                "filestore-rest#getUser",
                "filestore-rest#health",
                "filestore-rest#listFolderItems",
                "filestore-rest#listFolders",
                "filestore-rest#updateFolder",
            ]
        );
    }

    #[test]
    fn each_mock_carries_its_own_method_and_path() {
        let (_world, mocks) = mounted();
        let get = find(&mocks, "filestore-rest#getFolder");
        assert_eq!(get.request.methods.as_slice(), [http::Method::GET]);
        assert!(get.request.url_patterns[0].matches("/2.0/folders/42"));
        assert!(!get.request.url_patterns[0].matches("/2.0/folders"));

        let list = find(&mocks, "filestore-rest#listFolders");
        assert!(list.request.url_patterns[0].matches("/2.0/folders"));
    }

    #[test]
    fn the_template_s_priority_and_source_reach_every_operation() {
        let (_world, mocks) = mounted();
        for mock in &mocks {
            assert!(
                mock.priority < 100,
                "a hand-written mock must outrank `{}`",
                mock.id
            );
            assert_eq!(
                mock.source_file.as_deref(),
                Some("mocks.yaml"),
                "a served route is tracked to the collection that declared it"
            );
        }
    }

    #[test]
    fn a_base_url_that_cannot_be_appended_to_is_refused_by_name() {
        let (_world, backend) = backend();
        let mut template = template("filestore-rest", "");
        template.request.url_patterns =
            SmallVec::from_elem(UrlPattern::regex("^/v[0-9]+$").unwrap(), 1);

        let error = bind_rest(&template, &backend).unwrap_err().to_string();
        assert!(error.contains("plain base URL"), "unexpected: {error}");
    }

    #[tokio::test]
    async fn a_lookup_answers_from_the_store() {
        let (world, mocks) = mounted();
        let key = first_folder_key(&world);

        let (status, body) = answer(
            find(&mocks, "filestore-rest#getFolder"),
            "GET",
            &format!("/2.0/folders/{key}"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["id"], JsonValue::String(key));
        assert!(body["name"].is_string());
    }

    #[tokio::test]
    async fn a_key_nobody_stored_is_a_404_not_an_invented_record() {
        let (_world, mocks) = mounted();
        let (status, body) = answer(
            find(&mocks, "filestore-rest#getFolder"),
            "GET",
            "/2.0/folders/nope",
            Call {
                captures: vec![("folder_id", "nope")],
                ..Call::default()
            },
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["status"], 404);
    }

    #[tokio::test]
    async fn a_list_comes_back_in_the_envelope_the_document_declared() {
        let (_world, mocks) = mounted();
        let (status, body) = answer(
            find(&mocks, "filestore-rest#listFolders"),
            "GET",
            "/2.0/folders",
            Call::default(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["entries"].as_array().unwrap().len(), 5);
        assert_eq!(body["total_count"], 5);
        assert!(body["limit"].is_number());
    }

    #[tokio::test]
    async fn query_parameters_page_the_list() {
        let (_world, mocks) = mounted();
        let (_, body) = answer(
            find(&mocks, "filestore-rest#listFolders"),
            "GET",
            "/2.0/folders",
            Call {
                query: Some("limit=2&offset=1"),
                ..Call::default()
            },
        )
        .await;
        assert_eq!(body["entries"].as_array().unwrap().len(), 2);
        assert_eq!(body["total_count"], 5, "the total is the whole collection");
        assert_eq!(body["offset"], 1);
    }

    #[tokio::test]
    async fn a_query_parameter_naming_a_field_filters_by_it() {
        let (world, mocks) = mounted();
        let all = world
            .list("Folder", &crate::core::EntityQuery::default())
            .unwrap();
        let name = all.records[2]["name"].as_str().unwrap().to_string();

        let (_, body) = answer(
            find(&mocks, "filestore-rest#listFolders"),
            "GET",
            "/2.0/folders",
            Call {
                query: Some(&format!("name={name}")),
                ..Call::default()
            },
        )
        .await;
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "unexpected: {body}");
        assert_eq!(entries[0]["name"], JsonValue::String(name));
    }

    #[tokio::test]
    async fn a_nested_path_reads_the_children_through_the_relation() {
        let (world, mocks) = mounted();
        let key = first_folder_key(&world);

        let (status, body) = answer(
            find(&mocks, "filestore-rest#listFolderItems"),
            "GET",
            &format!("/2.0/folders/{key}/items"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let items = body.as_array().expect("a bare array, as declared");
        // Which files belong to which folder is derived, so the assertion is
        // that the two directions agree rather than a fixed count.
        let expected = world
            .related(
                "Folder",
                &key,
                "items",
                &crate::core::EntityQuery::default(),
            )
            .unwrap();
        assert_eq!(items.len(), expected.records.len().min(25));
    }

    #[tokio::test]
    async fn a_creation_writes_to_the_world_and_answers_with_the_declared_status() {
        let (world, mocks) = mounted();
        let before = world.count("Folder");

        let (status, body) = answer(
            find(&mocks, "filestore-rest#createFolder"),
            "POST",
            "/2.0/folders",
            Call {
                body: Some(r#"{"name":"Reports"}"#),
                ..Call::default()
            },
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["name"], "Reports");
        assert_eq!(world.count("Folder"), before + 1);
    }

    #[tokio::test]
    async fn a_patch_merges_and_is_visible_to_the_next_read() {
        let (world, mocks) = mounted();
        let key = first_folder_key(&world);

        let (status, body) = answer(
            find(&mocks, "filestore-rest#updateFolder"),
            "PATCH",
            &format!("/2.0/folders/{key}"),
            Call {
                body: Some(r#"{"name":"Renamed"}"#),
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "Renamed");

        let (_, body) = answer(
            find(&mocks, "filestore-rest#getFolder"),
            "GET",
            &format!("/2.0/folders/{key}"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;
        assert_eq!(body["name"], "Renamed");
    }

    #[tokio::test]
    async fn a_delete_answers_204_with_no_body_and_removes_the_record() {
        let (world, mocks) = mounted();
        let key = first_folder_key(&world);

        let (status, body) = answer(
            find(&mocks, "filestore-rest#deleteFolder"),
            "DELETE",
            &format!("/2.0/folders/{key}"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(body, JsonValue::Null, "204 means no body");
        assert!(world.get("Folder", &key).is_none());
    }

    #[tokio::test]
    async fn an_unclassified_operation_answers_from_its_declared_shape_and_is_counted() {
        let (_world, backend) = backend();
        let mocks = bind_rest(&template("filestore-rest", "/2.0"), &backend).unwrap();

        assert_eq!(
            backend.coverage().unclassified(),
            ["health"],
            "an operation that invents data has to say so"
        );
        assert!(backend.coverage().ratio() > 0.8);

        let (status, body) = answer(
            find(&mocks, "filestore-rest#health"),
            "GET",
            "/2.0/health",
            Call::default(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["status"].is_string(), "unexpected: {body}");
        assert_eq!(backend.coverage().fallback_hits(), 1);
    }

    /// The point of the whole design: a write through the world is visible to
    /// a document-derived route, because they are the same store.
    #[tokio::test]
    async fn a_write_through_the_world_is_visible_to_the_mounted_operations() {
        let (world, mocks) = mounted();
        let created = world
            .create("Folder", serde_json::json!({ "name": "Ada" }))
            .unwrap();
        let key = created["id"].as_str().unwrap().to_string();

        let (status, body) = answer(
            find(&mocks, "filestore-rest#getFolder"),
            "GET",
            &format!("/2.0/folders/{key}"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], "Ada");
    }

    /// Found by running the server: a query string is percent-encoded, and a
    /// filter compared raw matches nothing at all.
    #[tokio::test]
    async fn a_filter_value_is_decoded_before_it_is_compared() {
        let (world, mocks) = mounted();
        let all = world
            .list("Folder", &crate::core::EntityQuery::default())
            .unwrap();
        let name = all.records[1]["name"].as_str().unwrap().to_string();
        assert!(
            name.contains(' '),
            "the fixture needs a name worth encoding"
        );

        // Both spellings of a space in a query string. `%20` is what a browser
        // emits and `+` is what `curl --data-urlencode` and most client
        // libraries do; a mock that understands only one of them looks broken
        // half the time.
        for encoded in [
            urlencoding::encode(&name).into_owned(),
            name.replace(' ', "+"),
        ] {
            let (_, body) = answer(
                find(&mocks, "filestore-rest#listFolders"),
                "GET",
                "/2.0/folders",
                Call {
                    query: Some(&format!("name={encoded}")),
                    ..Call::default()
                },
            )
            .await;
            assert_eq!(
                body["entries"].as_array().unwrap().len(),
                1,
                "`{encoded}` should find one folder: {body}"
            );
        }
    }

    /// Also found by running it: a link the expansion stopped at was left as a
    /// bare key, so `parent` was an object one level down and a string the
    /// next. A client switching on the type breaks on exactly that.
    #[tokio::test]
    async fn a_link_the_expansion_stops_at_keeps_the_shape_the_schema_declared() {
        let (world, mocks) = mounted();
        let key = nested_folder_key(&world);

        let (_, body) = answer(
            find(&mocks, "filestore-rest#getFolder"),
            "GET",
            &format!("/2.0/folders/{key}"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;

        let parent = &body["parent"];
        assert!(parent.is_object(), "unexpected: {body}");
        assert!(parent["name"].is_string(), "one level is expanded in full");

        let grandparent = &parent["parent"];
        assert!(
            grandparent.is_object(),
            "the depth cap must not change the field's type: {grandparent}"
        );
        assert!(grandparent["id"].is_string());
        assert!(
            grandparent.get("name").is_none(),
            "a mini representation carries the key and nothing else"
        );
    }

    #[tokio::test]
    async fn a_foreign_key_field_holds_a_key_that_resolves() {
        let (world, mocks) = mounted();
        let key = first_folder_key(&world);

        let (_, body) = answer(
            find(&mocks, "filestore-rest#getFolder"),
            "GET",
            &format!("/2.0/folders/{key}"),
            Call {
                captures: vec![("folder_id", &key)],
                ..Call::default()
            },
        )
        .await;

        let user_id = body["user_id"].as_str().expect("a key, not an object");
        assert!(
            world.get("User", user_id).is_some(),
            "a field named like another entity's key has to hold one that resolves"
        );
    }
}
