//! Reading a schema into the world.
//!
//! A schema file is dispatched by the registry's loader like a collection, a
//! HAR or a script — but what it produces is *entities*, not routes. It cannot
//! produce routes: a `.graphql` has nowhere to write down that it is served at
//! `https://api.example.com/graphql` rather than at `localhost/graphql`, and
//! guessing is how a proxy ends up answering on the wrong host.
//!
//! So loading a schema populates [`crate::core::World`] and registers nothing.
//! A mock with `serve:` says where it answers, in the same URL syntax every
//! other mock uses.

use std::path::Path;
use std::sync::Arc;

use crate::core::World;
use crate::core::world::Binding;
use crate::core::world::store::DeltaConflict;

use super::infer::graphql::{SdlDefect, parse_sdl, parse_sdl_lenient, to_entity_graph};
use super::infer::openapi::{self, SpecDefect};

/// Extensions the loader reads as a GraphQL schema.
///
/// `.yaml` and `.json` are absent on purpose: those are mock collections, and
/// sniffing a file's contents to decide which it is would break the moment a
/// collection happened to carry a key an OpenAPI document also uses.
pub const SCHEMA_EXTENSIONS: [&str; 2] = ["graphql", "gql"];

/// Names that mark an OpenAPI document written in an ordinary data format.
///
/// The same reasoning: a bare `.yaml` is a mock collection, so a document that
/// wants picking up on its own has to say what it is in its name. Any path is
/// still loadable by being named under `world.schemas`, whatever it is called.
pub const OPENAPI_SUFFIXES: [&str; 3] = [".openapi.yaml", ".openapi.yml", ".openapi.json"];

/// How a schema file is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaFormat {
    GraphQL,
    OpenApi,
}

/// Whether a path names a schema the loader picks up on its own.
#[must_use]
pub fn is_schema_file(path: &Path) -> bool {
    is_graphql_file(path) || is_openapi_file(path)
}

#[must_use]
pub fn is_graphql_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SCHEMA_EXTENSIONS.contains(&ext))
}

/// Whether a path names an OpenAPI document by its own name.
#[must_use]
pub fn is_openapi_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|name| OPENAPI_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)))
}

/// The format a path is read as.
///
/// Decided by extension, never by contents: a file that has to be opened before
/// anyone can say what it is fails differently every time its contents change.
pub fn format_of(path: &Path) -> crate::Result<SchemaFormat> {
    if is_graphql_file(path) {
        return Ok(SchemaFormat::GraphQL);
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("yaml" | "yml" | "json") => Ok(SchemaFormat::OpenApi),
        Some(other) => Err(crate::mp_err!(
            "{}: `.{other}` is not a schema this reads; \
             `.graphql`/`.gql` are GraphQL, `.yaml`/`.yml`/`.json` are OpenAPI",
            path.display()
        )),
        None => Err(crate::mp_err!(
            "{}: a schema file needs an extension saying what it is",
            path.display()
        )),
    }
}

/// What reading a schema cost.
#[derive(Debug, Default)]
pub struct SchemaLoad {
    /// Entities the schema contributed.
    pub entities: usize,
    /// Malformations repaired to read the file, when `lenient` was set.
    pub repaired: Vec<SdlDefect>,
    /// Parts of an OpenAPI document that could not be read.
    pub defects: Vec<SpecDefect>,
    /// Schemas a path addressed but which nothing could key.
    pub skipped: Vec<openapi::entities::Skipped>,
    /// Writes the rebuilt store could not carry across.
    pub conflicts: Vec<DeltaConflict>,
}

/// Read a schema file into the world.
pub async fn load_schema_file(
    path: &Path,
    world: &Arc<World>,
    lenient: bool,
) -> crate::Result<SchemaLoad> {
    let source = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| crate::mp_err!("Could not read {}: {e}", path.display()))?;
    load_schema(&source, path, world, lenient)
}

/// Read schema source into the world.
pub fn load_schema(
    source: &str,
    path: &Path,
    world: &Arc<World>,
    lenient: bool,
) -> crate::Result<SchemaLoad> {
    load_schema_with(source, path, world, lenient, &crate::profile::DefaultProfile)
}

/// [`load_schema`] with a profile consulted ahead of the built-in rules.
///
/// Only the OpenAPI front end has anything to ask a profile: a GraphQL schema
/// declares its own types, while an OpenAPI document leaves the domain
/// questions — which `x-` key states a relation, what this API calls a cursor —
/// to whoever owns it.
pub fn load_schema_with(
    source: &str,
    path: &Path,
    world: &Arc<World>,
    lenient: bool,
    profile: &dyn crate::profile::ConsolidationProfile,
) -> crate::Result<SchemaLoad> {
    match format_of(path)? {
        SchemaFormat::GraphQL => load_graphql(source, path, world, lenient),
        SchemaFormat::OpenApi => load_openapi(source, path, world, profile),
    }
}

fn load_graphql(
    source: &str,
    path: &Path,
    world: &Arc<World>,
    lenient: bool,
) -> crate::Result<SchemaLoad> {
    let (parsed, repaired) = if lenient {
        parse_sdl_lenient(source)?
    } else {
        (parse_sdl(source)?, Vec::new())
    };

    let contribution = to_entity_graph(&parsed);
    let entities = contribution.len();

    let conflicts = world.add_schema(path, Binding::GraphQL(Arc::new(parsed)), &contribution)?;

    Ok(SchemaLoad {
        entities,
        repaired,
        conflicts,
        ..SchemaLoad::default()
    })
}

fn load_openapi(
    source: &str,
    path: &Path,
    world: &Arc<World>,
    profile: &dyn crate::profile::ConsolidationProfile,
) -> crate::Result<SchemaLoad> {
    let (table, defects) = openapi::parse_openapi(source)
        .map_err(|e| crate::mp_err!("{}: {e}", path.display()))?;

    let inference = openapi::to_entity_graph_with(&table, profile);
    let entities = inference.graph.len();

    let conflicts = world.add_schema(
        path,
        Binding::OpenApi(Arc::new(table)),
        &inference.graph,
    )?;

    Ok(SchemaLoad {
        entities,
        defects,
        skipped: inference.skipped,
        conflicts,
        ..SchemaLoad::default()
    })
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

    const SCHEMA: &str = "type User { id: ID!, name: String! } type Query { users: [User!]! }";

    fn world() -> Arc<World> {
        Arc::new(World::new())
    }

    #[test]
    fn a_schema_contributes_entities_and_no_routes() {
        let world = world();
        let loaded =
            load_schema(SCHEMA, Path::new("/mocks/schema.graphql"), &world, false).unwrap();

        assert_eq!(loaded.entities, 1);
        assert!(!world.is_empty());
        assert_eq!(
            world.count("User"),
            crate::core::world::store::DEFAULT_SEED_COUNT
        );
    }

    const OPENAPI: &str = r#"
openapi: 3.0.3
info: { title: Filestore }
paths:
  /folders:
    get:
      operationId: listFolders
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}:
    get:
      operationId: getFolder
      parameters:
        - { name: folder_id, in: path, required: true, schema: { type: string } }
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
components:
  schemas:
    Folder:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
"#;

    #[test]
    fn extensions_are_recognised() {
        assert!(is_schema_file(Path::new("schema.graphql")));
        assert!(is_schema_file(Path::new("a/b/schema.gql")));
        assert!(
            !is_schema_file(Path::new("mocks.yaml")),
            "a .yaml is a collection; a schema with that name has to be named under world.schemas"
        );
        assert!(!is_schema_file(Path::new("traffic.har")));
    }

    #[test]
    fn an_openapi_document_names_itself_to_be_picked_up() {
        assert!(is_openapi_file(Path::new("filestore-content.openapi.yaml")));
        assert!(is_openapi_file(Path::new("a/b/filestore.openapi.json")));
        assert!(is_openapi_file(Path::new("filestore.openapi.yml")));
        assert!(
            !is_openapi_file(Path::new("mocks.yaml")),
            "a bare .yaml stays a mock collection"
        );
        assert!(!is_graphql_file(Path::new("filestore.openapi.yaml")));
    }

    /// The registry keeps its own copy so an `.openapi.yaml` is routed the same
    /// way in a build without the `spec` feature. Two lists, one meaning.
    #[test]
    fn the_loaders_agree_on_what_an_openapi_document_is_called() {
        assert_eq!(
            OPENAPI_SUFFIXES,
            crate::engine::registry::OPENAPI_SUFFIXES,
            "spec::source and the registry must recognise the same names"
        );
    }

    #[test]
    fn the_format_comes_from_the_extension_not_the_contents() {
        assert_eq!(
            format_of(Path::new("s.graphql")).unwrap(),
            SchemaFormat::GraphQL
        );
        // Named under `world.schemas`, so the name is free; the extension is not.
        assert_eq!(
            format_of(Path::new("openapi/filestore.yaml")).unwrap(),
            SchemaFormat::OpenApi
        );
        assert_eq!(format_of(Path::new("s.json")).unwrap(), SchemaFormat::OpenApi);

        let error = format_of(Path::new("s.har")).unwrap_err().to_string();
        assert!(error.contains(".har"), "unexpected: {error}");
    }

    #[test]
    fn an_openapi_document_contributes_entities_and_no_routes() {
        let world = world();
        let loaded = load_schema(
            OPENAPI,
            Path::new("/mocks/filestore.openapi.yaml"),
            &world,
            false,
        )
        .unwrap();

        assert_eq!(loaded.entities, 1);
        assert!(loaded.defects.is_empty());
        assert_eq!(
            world.count("Folder"),
            crate::core::world::store::DEFAULT_SEED_COUNT
        );
        assert_eq!(world.schemas().len(), 1);
        assert_eq!(world.schemas()[0].binding.protocol(), "rest");
    }

    #[test]
    fn a_graphql_schema_and_an_openapi_document_merge_into_one_world() {
        let world = world();
        load_schema(SCHEMA, Path::new("a.graphql"), &world, false).unwrap();
        load_schema(OPENAPI, Path::new("b.openapi.yaml"), &world, false).unwrap();

        assert_eq!(world.entities().len(), 2);
        assert!(world.count("User") > 0);
        assert!(world.count("Folder") > 0);
    }

    /// The same claim the GraphQL loader makes, for the second front end: a
    /// document loading beside a schema must not discard what a handler wrote.
    #[test]
    fn a_write_survives_an_openapi_document_being_loaded() {
        let world = world();
        load_schema(SCHEMA, Path::new("a.graphql"), &world, false).unwrap();

        let created = world
            .create("User", serde_json::json!({ "name": "Ada" }))
            .unwrap();
        let key = created["id"].as_str().unwrap().to_string();

        let loaded =
            load_schema(OPENAPI, Path::new("b.openapi.yaml"), &world, false).unwrap();

        assert!(loaded.conflicts.is_empty());
        assert_eq!(
            world.get("User", &key).unwrap()["name"],
            serde_json::json!("Ada"),
            "a rebuilt store has to carry writes across, or loading a document \
             silently discards state"
        );
    }

    #[test]
    fn a_mock_collection_handed_to_the_schema_loader_says_what_is_wrong() {
        let error = load_schema(
            "name: my mocks\nmocks: []\n",
            Path::new("mocks.yaml"),
            &world(),
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("openapi"), "unexpected: {error}");
    }

    #[test]
    fn two_schemas_merge_into_one_world() {
        let world = world();
        load_schema(SCHEMA, Path::new("a.graphql"), &world, false).unwrap();
        load_schema(
            "type Post { id: ID!, title: String! } type Query { posts: [Post!]! }",
            Path::new("b.graphql"),
            &world,
            false,
        )
        .unwrap();

        assert_eq!(world.entities().len(), 2);
        assert!(world.count("User") > 0);
        assert!(world.count("Post") > 0);
    }

    #[test]
    fn a_write_survives_a_second_schema_being_loaded() {
        let world = world();
        load_schema(SCHEMA, Path::new("a.graphql"), &world, false).unwrap();

        let created = world
            .create("User", serde_json::json!({ "name": "Ada" }))
            .unwrap();
        let key = created["id"].as_str().unwrap().to_string();

        let loaded = load_schema(
            "type Post { id: ID! } type Query { posts: [Post!]! }",
            Path::new("b.graphql"),
            &world,
            false,
        )
        .unwrap();

        assert!(loaded.conflicts.is_empty());
        assert_eq!(
            world.get("User", &key).unwrap()["name"],
            serde_json::json!("Ada"),
            "a rebuilt store has to carry writes across, or loading a second schema \
             silently discards state"
        );
    }

    #[test]
    fn a_collision_is_reported_rather_than_silently_merged() {
        let world = world();
        load_schema(SCHEMA, Path::new("a.graphql"), &world, false).unwrap();
        load_schema(
            "type User { id: ID!, login: String! } type Query { me: User }",
            Path::new("b.graphql"),
            &world,
            false,
        )
        .unwrap();

        let collisions = world.collisions();
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].entity.as_str(), "User");
        assert_eq!(collisions[0].sources.len(), 2);
    }

    #[test]
    fn a_malformed_schema_is_refused_unless_repair_was_asked_for() {
        let broken = "type A {\n  \"The MIME type (e.g., \"text/plain\").\"\n  f: String\n}\ntype Query { a: A }";
        assert!(load_schema(broken, Path::new("s.graphql"), &world(), false).is_err());

        let loaded = load_schema(broken, Path::new("s.graphql"), &world(), true).unwrap();
        assert_eq!(loaded.repaired.len(), 1);
    }
}
