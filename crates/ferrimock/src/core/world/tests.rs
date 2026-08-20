//! The properties the world exists to hold: one shared store, deterministic
//! from its seed, and writes that survive the world growing.

use super::*;
use crate::core::world::model::{
    CompositeKey, EntityType, FieldDef, Provenance, Rule, Scalar, ScalarKind, ValueSpec,
};

fn entity(name: &str) -> EntityType {
    EntityType::new(
        name,
        CompositeKey::single("id"),
        Provenance::new(Rule::Explicit, "test"),
    )
    .with_field(FieldDef::new(
        "id",
        ValueSpec::Scalar(Scalar::new(ScalarKind::Id)),
        false,
    ))
    .with_field(FieldDef::new(
        "name",
        ValueSpec::Scalar(Scalar::new(ScalarKind::String)),
        false,
    ))
}

fn graph_of(names: &[&str]) -> EntityGraph {
    let mut graph = EntityGraph::new();
    for name in names {
        graph.insert(entity(name));
    }
    graph
}

fn seeded(seed: u64, names: &[&str]) -> World {
    let world = World::new();
    world
        .configure(
            &WorldSettings {
                seed: Some(seed),
                ..WorldSettings::default()
            },
            Path::new("test.yaml"),
        )
        .unwrap();
    world.add_entities(&graph_of(names)).unwrap();
    world
}

#[test]
fn an_empty_world_serves_nothing() {
    let world = World::new();
    assert!(world.is_empty());
    assert!(world.entities().is_empty());
    assert_eq!(world.count("User"), 0);
}

#[test]
fn the_same_seed_builds_the_same_world() {
    let a = seeded(7, &["User"]);
    let b = seeded(7, &["User"]);

    let keys_a: Vec<_> = a.store().keys("User");
    let keys_b: Vec<_> = b.store().keys("User");
    assert_eq!(keys_a, keys_b);

    let first = keys_a.first().unwrap().to_string();
    assert_eq!(a.get("User", &first), b.get("User", &first));
}

#[test]
fn a_different_seed_builds_a_different_world() {
    let a = seeded(1, &["User"]);
    let b = seeded(2, &["User"]);
    assert_ne!(a.store().keys("User"), b.store().keys("User"));
}

#[test]
fn counts_are_honoured_per_entity() {
    let world = World::new();
    world
        .configure(
            &WorldSettings {
                seed: Some(1),
                default_count: Some(4),
                counts: std::iter::once((LeanString::from("Post"), 9)).collect(),
                ..WorldSettings::default()
            },
            Path::new("test.yaml"),
        )
        .unwrap();
    world.add_entities(&graph_of(&["User", "Post"])).unwrap();

    assert_eq!(world.count("User"), 4);
    assert_eq!(world.count("Post"), 9);
}

/// A schema does not say how big its world should be, and the answer differs
/// between a unit test and a screen someone is looking at.
#[test]
fn a_scale_asks_for_a_bigger_world_without_naming_every_entity() {
    let world = World::new();
    world
        .configure(
            &WorldSettings {
                seed: Some(1),
                default_count: Some(4),
                scale: Some(5.0),
                counts: std::iter::once((LeanString::from("Post"), 9)).collect(),
                ..WorldSettings::default()
            },
            Path::new("test.yaml"),
        )
        .unwrap();
    world.add_entities(&graph_of(&["User", "Post"])).unwrap();

    assert_eq!(world.count("User"), 20);
    assert_eq!(world.count("Post"), 9, "a stated count is left alone");
}

#[test]
fn two_collections_cannot_disagree_about_the_seed() {
    let world = World::new();
    world
        .configure(
            &WorldSettings {
                seed: Some(1),
                ..WorldSettings::default()
            },
            Path::new("a.yaml"),
        )
        .unwrap();

    let error = world
        .configure(
            &WorldSettings {
                seed: Some(2),
                ..WorldSettings::default()
            },
            Path::new("b.yaml"),
        )
        .unwrap_err()
        .to_string();

    assert!(error.contains("a.yaml"), "unexpected: {error}");
    assert!(error.contains("b.yaml"), "unexpected: {error}");
}

#[test]
fn repeating_the_same_seed_is_not_a_disagreement() {
    let world = World::new();
    let settings = WorldSettings {
        seed: Some(1),
        ..WorldSettings::default()
    };
    world.configure(&settings, Path::new("a.yaml")).unwrap();
    world.configure(&settings, Path::new("b.yaml")).unwrap();
}

#[test]
fn a_creation_is_visible_to_the_next_read() {
    let world = seeded(1, &["User"]);
    let created = world
        .create("User", serde_json::json!({ "name": "Ada" }))
        .unwrap();
    let key = created["id"].as_str().unwrap();

    assert_eq!(world.get("User", key).unwrap()["name"], "Ada");
    assert_eq!(world.count("User"), store::DEFAULT_SEED_COUNT + 1);
}

#[test]
fn a_delete_removes_an_instance() {
    let world = seeded(1, &["User"]);
    let key = world.store().keys("User").first().unwrap().to_string();

    world.delete("User", &key).unwrap();
    assert!(world.get("User", &key).is_none());
    assert_eq!(world.count("User"), store::DEFAULT_SEED_COUNT - 1);
}

/// The reason the store is rebuilt rather than replaced: loading a second
/// schema must not throw away state a handler already wrote.
#[test]
fn writes_survive_the_world_growing() {
    let world = seeded(1, &["User"]);

    // Taken before the creation: `keys` lists derived instances then created
    // ones, so `last()` afterwards would name the new record rather than a
    // seeded one.
    let derived = world.store().keys("User");
    let patched_key = derived.first().unwrap().to_string();
    let deleted_key = derived.last().unwrap().to_string();

    let created = world
        .create("User", serde_json::json!({ "name": "Ada" }))
        .unwrap();
    let created_key = created["id"].as_str().unwrap().to_string();

    world
        .update(
            "User",
            &patched_key,
            serde_json::json!({ "name": "patched" }),
        )
        .unwrap();
    world.delete("User", &deleted_key).unwrap();

    let conflicts = world.add_entities(&graph_of(&["Post"])).unwrap();
    assert!(conflicts.is_empty(), "unexpected: {conflicts:?}");

    assert_eq!(world.get("User", &created_key).unwrap()["name"], "Ada");
    assert_eq!(world.get("User", &patched_key).unwrap()["name"], "patched");
    assert!(world.get("User", &deleted_key).is_none());
    assert!(world.count("Post") > 0);
}

/// Adding a schema must not perturb the entities already in play, or every
/// assertion written against the old world silently breaks.
#[test]
fn existing_entities_keep_their_values_when_the_world_grows() {
    let world = seeded(3, &["User"]);
    let before: Vec<_> = world
        .store()
        .keys("User")
        .iter()
        .map(|key| world.get("User", &key.to_string()))
        .collect();

    world.add_entities(&graph_of(&["Post"])).unwrap();

    let after: Vec<_> = world
        .store()
        .keys("User")
        .iter()
        .map(|key| world.get("User", &key.to_string()))
        .collect();
    assert_eq!(before, after);
}

#[test]
fn reset_drops_writes_and_keeps_the_seeded_world() {
    let world = seeded(1, &["User"]);
    let key = world.store().keys("User").first().unwrap().to_string();
    let original = world.get("User", &key).unwrap();

    world
        .update("User", &key, serde_json::json!({ "name": "changed" }))
        .unwrap();
    world
        .create("User", serde_json::json!({ "name": "extra" }))
        .unwrap();
    assert!(world.pending_writes() > 0);

    world.reset();

    assert_eq!(world.pending_writes(), 0);
    assert_eq!(world.get("User", &key).unwrap(), original);
    assert_eq!(world.count("User"), store::DEFAULT_SEED_COUNT);
}

#[test]
fn a_filter_narrows_a_list() {
    let world = seeded(1, &["User"]);
    world
        .create("User", serde_json::json!({ "name": "needle" }))
        .unwrap();

    let query = EntityQuery {
        filter: std::iter::once(("name".to_string(), serde_json::json!("needle"))).collect(),
        ..EntityQuery::default()
    };
    let page = world.list("User", &query).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.records[0]["name"], "needle");
}

#[test]
fn an_operator_filter_is_read_as_an_operator() {
    let predicate = predicate_of("age", &serde_json::json!({ "gt": 30 }));
    assert_eq!(predicate.op, PredicateOp::Gt);
    assert_eq!(predicate.value, serde_json::json!(30));

    // A bare object with more than one key is a value, not an operator.
    let predicate = predicate_of("meta", &serde_json::json!({ "gt": 1, "lt": 2 }));
    assert_eq!(predicate.op, PredicateOp::Eq);
}

#[test]
fn a_limit_pages_a_list() {
    let world = seeded(1, &["User"]);
    let query = EntityQuery {
        limit: Some(3),
        ..EntityQuery::default()
    };
    let page = world.list("User", &query).unwrap();

    assert_eq!(page.records.len(), 3);
    assert_eq!(page.total, store::DEFAULT_SEED_COUNT);
    assert!(page.has_next);
    assert!(!page.has_previous);
}

#[test]
fn a_descending_sort_is_spelled_with_a_leading_dash() {
    let world = seeded(1, &["User"]);
    let ascending = world
        .list(
            "User",
            &EntityQuery {
                sort: vec!["name".to_string()],
                ..EntityQuery::default()
            },
        )
        .unwrap();
    let descending = world
        .list(
            "User",
            &EntityQuery {
                sort: vec!["-name".to_string()],
                ..EntityQuery::default()
            },
        )
        .unwrap();

    let mut reversed = descending.records;
    reversed.reverse();
    assert_eq!(ascending.records, reversed);
}

#[test]
fn an_unknown_entity_is_an_error_rather_than_an_empty_page() {
    let world = seeded(1, &["User"]);
    assert!(world.list("Nope", &EntityQuery::default()).is_err());
    assert!(world.get("Nope", "1").is_none());
}

#[test]
fn nearest_entity_catches_a_typo() {
    let graph = graph_of(&["Folder", "File"]);
    assert_eq!(
        nearest_entity(&graph, "Foldr").map(|e| e.name.to_string()),
        Some("Folder".to_string())
    );
    assert!(nearest_entity(&graph, "CompletelyDifferent").is_none());
}

// ===== Regressions =====

#[test]
fn one_collection_can_change_its_own_seed_and_count() {
    let world = World::new();
    let source = Path::new("mocks.yaml");

    world
        .configure(
            &WorldSettings {
                seed: Some(1),
                default_count: Some(4),
                ..WorldSettings::default()
            },
            source,
        )
        .unwrap();
    world.add_entities(&graph_of(&["User"])).unwrap();
    assert_eq!(world.count("User"), 4);

    // The same file, edited and reloaded. Refusing this made `seed:` and
    // `count:` changeable only by restarting the server.
    world
        .configure(
            &WorldSettings {
                seed: Some(2),
                default_count: Some(9),
                ..WorldSettings::default()
            },
            source,
        )
        .expect("a file may change what it itself asked for");
    assert_eq!(world.count("User"), 9);
    assert_eq!(world.seed(), 2);
}

#[test]
fn two_collections_still_cannot_disagree_about_the_count() {
    let world = World::new();
    world
        .configure(
            &WorldSettings {
                default_count: Some(4),
                ..WorldSettings::default()
            },
            Path::new("one.yaml"),
        )
        .unwrap();
    let clash = world.configure(
        &WorldSettings {
            default_count: Some(9),
            ..WorldSettings::default()
        },
        Path::new("two.yaml"),
    );
    assert!(clash.is_err(), "two files disagreeing is still a mistake");
}

#[test]
fn two_schemas_describing_one_entity_keep_both_their_fields() {
    let declared = |name: &str, fields: &[&str]| {
        let mut graph = EntityGraph::new();
        let mut entity = EntityType::new(
            name,
            CompositeKey::single("id"),
            Provenance::new(Rule::GraphQLSchema, name),
        );
        for field in fields {
            entity = entity.with_field(FieldDef::new(
                *field,
                ValueSpec::Scalar(Scalar::new(ScalarKind::String)),
                true,
            ));
        }
        graph.insert(entity);
        graph
    };

    let world = World::new();
    world
        .add_entities(&declared("User", &["id", "email"]))
        .unwrap();
    world
        .add_entities(&declared("User", &["id", "karma"]))
        .unwrap();

    let graph = world.graph();
    let user = graph.get("User").expect("one User");
    let fields: Vec<&str> = user.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(
        fields.contains(&"email") && fields.contains(&"karma"),
        "a surface that loses its own fields serves payloads its schema rejects, got {fields:?}"
    );
}
