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

/// Extensions the loader reads as a schema.
///
/// `.yaml` and `.json` are absent on purpose: those are mock collections, and
/// sniffing a file's contents to decide which it is would break the moment a
/// collection happened to carry a key an OpenAPI document also uses. A schema
/// with an ordinary extension has to be named under `world.schemas`.
pub const SCHEMA_EXTENSIONS: [&str; 2] = ["graphql", "gql"];

/// Whether a path names a schema the loader picks up on its own.
#[must_use]
pub fn is_schema_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SCHEMA_EXTENSIONS.contains(&ext))
}

/// What reading a schema cost.
#[derive(Debug, Default)]
pub struct SchemaLoad {
    /// Entities the schema contributed.
    pub entities: usize,
    /// Malformations repaired to read the file, when `lenient` was set.
    pub repaired: Vec<SdlDefect>,
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
