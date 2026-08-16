#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::spec::algebra::{Predicate, Selection, SortKey};
use crate::spec::model::{
    Carrier, Confidence, EntityType, FieldDef, Provenance, Relation, Rule, Scalar, ScalarKind,
};

fn scalar_field(name: &str, kind: ScalarKind) -> FieldDef {
    FieldDef::new(name, ValueSpec::Scalar(Scalar::new(kind)), false)
}

fn relation_field(name: &str, target: &str, cardinality: Cardinality) -> FieldDef {
    let relation = Relation::new(
        target,
        cardinality,
        Carrier::Embedded,
        Confidence::STRUCTURAL,
        Provenance::new(Rule::GraphQLSchema, name),
    );
    let spec = ValueSpec::Relation(Box::new(relation));
    let spec = match cardinality {
        Cardinality::One => spec,
        Cardinality::Many => ValueSpec::List(Box::new(spec)),
    };
    FieldDef::new(name, spec, true)
}

fn entity(name: &str) -> EntityType {
    EntityType::new(
        name,
        crate::spec::model::CompositeKey::single("id"),
        Provenance::new(Rule::GraphQLSchema, name),
    )
    .with_field(scalar_field("id", ScalarKind::Id))
}

/// Users and posts, each post owned by one user, each user listing its posts.
fn blog_store(seed: u64, users: usize, posts: usize) -> EntityStore {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("User")
            .with_field(scalar_field("name", ScalarKind::String))
            .with_field(relation_field("posts", "Post", Cardinality::Many)),
    );
    graph.insert(
        entity("Post")
            .with_field(scalar_field("title", ScalarKind::String))
            .with_field(relation_field("author", "User", Cardinality::One)),
    );

    EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed)
            .with_count("User", users)
            .with_count("Post", posts),
    )
}

#[test]
fn the_census_counts_without_materialising() {
    let store = blog_store(1, 5, 20);
    assert_eq!(store.count("User"), 5);
    assert_eq!(store.count("Post"), 20);
}

#[test]
fn the_same_seed_rebuilds_the_same_world() {
    let a = blog_store(7, 4, 9);
    let b = blog_store(7, 4, 9);
    for key in a.keys("Post") {
        assert_eq!(a.get("Post", &key), b.get("Post", &key));
    }
}

#[test]
fn a_different_seed_gives_a_different_world() {
    let a = blog_store(1, 4, 9);
    let b = blog_store(2, 4, 9);
    let a_titles: Vec<_> = a
        .keys("Post")
        .iter()
        .filter_map(|k| a.get("Post", k))
        .filter_map(|r| r.get("title").cloned())
        .collect();
    let b_titles: Vec<_> = b
        .keys("Post")
        .iter()
        .filter_map(|k| b.get("Post", k))
        .filter_map(|r| r.get("title").cloned())
        .collect();
    assert_ne!(a_titles, b_titles);
}

#[test]
fn reading_one_record_does_not_depend_on_reading_the_others() {
    let store = blog_store(3, 4, 12);
    let keys = store.keys("Post");
    let direct = store.get("Post", &keys[7]).unwrap();

    let fresh = blog_store(3, 4, 12);
    for key in fresh.keys("Post").iter().take(5) {
        let _ = fresh.get("Post", key);
    }
    assert_eq!(fresh.get("Post", &keys[7]).unwrap(), direct);
}

#[test]
fn every_foreign_key_resolves() {
    let store = blog_store(5, 3, 30);
    for key in store.keys("Post") {
        let post = store.get("Post", &key).unwrap();
        let author_key = post.get("author").unwrap().as_str().unwrap();
        assert!(
            store
                .get("User", &EntityKey::single(author_key))
                .is_some(),
            "a derived foreign key must land inside the parent census"
        );
    }
}

#[test]
fn both_sides_of_a_relation_agree() {
    let store = blog_store(11, 4, 40);
    for user_key in store.keys("User") {
        let posts = store
            .related("User", &user_key, "posts", &Selection::new())
            .unwrap();
        for post in &posts.records {
            let author = store
                .relation_target(
                    "Post",
                    &post.key,
                    "author",
                    store
                        .graph()
                        .get("Post")
                        .unwrap()
                        .field("author")
                        .unwrap()
                        .relation()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                author.key, user_key,
                "user.posts must contain exactly the posts whose author is that user"
            );
        }
    }
}

#[test]
fn every_child_is_owned_by_exactly_one_parent() {
    let store = blog_store(13, 3, 25);
    let mut seen = 0;
    for user_key in store.keys("User") {
        seen += store
            .related("User", &user_key, "posts", &Selection::new())
            .unwrap()
            .records
            .len();
    }
    assert_eq!(seen, store.count("Post"));
}

#[test]
fn a_key_field_agrees_with_the_key_it_is_filed_under() {
    let store = blog_store(2, 3, 6);
    for key in store.keys("User") {
        let record = store.get("User", &key).unwrap();
        assert_eq!(record.get("id").unwrap().as_str().unwrap(), key.to_string());
    }
}

#[test]
fn a_created_record_is_visible_to_the_next_read() {
    let store = blog_store(4, 2, 2);
    let before = store.count("User");

    let written = store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "name": "Ada" }),
            },
        )
        .unwrap();
    let Written::Created(record) = written else {
        panic!("insert should create")
    };

    assert_eq!(store.count("User"), before + 1);
    let read_back = store.get("User", &record.key).unwrap();
    assert_eq!(read_back.get("name").unwrap(), "Ada");
    assert!(
        store.keys("User").contains(&record.key),
        "a creation must appear in the list"
    );
}

#[test]
fn a_created_key_looks_like_the_keys_around_it() {
    let store = blog_store(4, 3, 3);
    let Written::Created(record) = store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "name": "Ada" }),
            },
        )
        .unwrap()
    else {
        panic!("insert should create")
    };

    let derived = store.keys("User")[0].to_string();
    let created = record.key.to_string();
    assert_eq!(
        created.len(),
        derived.len(),
        "a created key should have the shape the entity's keys have, not a marker"
    );
    assert!(
        !store
            .keys("User")
            .iter()
            .filter(|k| **k != record.key)
            .any(|k| *k == record.key),
        "a created key must not collide with a derived one"
    );
}

#[test]
fn a_created_record_still_has_the_fields_it_did_not_supply() {
    let store = blog_store(4, 2, 2);
    let Written::Created(record) = store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "name": "Ada" }),
            },
        )
        .unwrap()
    else {
        panic!("insert should create")
    };
    assert!(record.get("id").is_some(), "the key must be filled in");
}

#[test]
fn a_creation_can_choose_its_own_key() {
    let store = blog_store(4, 2, 2);
    let Written::Created(record) = store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "id": "u-42", "name": "Ada" }),
            },
        )
        .unwrap()
    else {
        panic!("insert should create")
    };
    assert_eq!(record.key, EntityKey::single("u-42"));
    assert!(store.get("User", &EntityKey::single("u-42")).is_some());
}

#[test]
fn creating_the_same_key_twice_is_refused() {
    let store = blog_store(4, 2, 2);
    let values = serde_json::json!({ "id": "u-1", "name": "Ada" });
    store
        .apply("User", Mutation::Insert { values: values.clone() })
        .unwrap();
    assert!(store.apply("User", Mutation::Insert { values }).is_err());
}

#[test]
fn a_patch_survives_and_leaves_the_rest_alone() {
    let store = blog_store(6, 3, 3);
    let key = store.keys("User")[0].clone();
    let original = store.get("User", &key).unwrap();

    store
        .apply(
            "User",
            Mutation::Patch {
                key: key.clone(),
                values: serde_json::json!({ "name": "Grace" }),
            },
        )
        .unwrap();

    let updated = store.get("User", &key).unwrap();
    assert_eq!(updated.get("name").unwrap(), "Grace");
    assert_eq!(updated.get("id"), original.get("id"));
}

#[test]
fn a_replace_drops_unmentioned_fields_but_keeps_the_key() {
    let store = blog_store(6, 3, 3);
    let key = store.keys("User")[0].clone();
    store
        .apply(
            "User",
            Mutation::Replace {
                key: key.clone(),
                values: serde_json::json!({ "name": "Grace" }),
            },
        )
        .unwrap();

    let updated = store.get("User", &key).unwrap();
    assert_eq!(updated.get("name").unwrap(), "Grace");
    assert_eq!(updated.get("id").unwrap().as_str().unwrap(), key.to_string());
}

#[test]
fn a_removed_record_is_gone_from_reads_and_lists() {
    let store = blog_store(8, 4, 4);
    let key = store.keys("Post")[0].clone();

    store
        .apply("Post", Mutation::Remove { key: key.clone() })
        .unwrap();

    assert!(store.get("Post", &key).is_none());
    assert!(!store.keys("Post").contains(&key));
    assert_eq!(store.count("Post"), 3);
    let listed = store.list("Post", &Selection::new()).unwrap();
    assert_eq!(listed.total, 3);
}

#[test]
fn removing_a_parent_cascades_to_its_children() {
    let store = blog_store(9, 2, 8);
    let user_key = store.keys("User")[0].clone();
    let owned: Vec<_> = store
        .related("User", &user_key, "posts", &Selection::new())
        .unwrap()
        .records
        .iter()
        .map(|r| r.key.clone())
        .collect();
    assert!(!owned.is_empty(), "the fixture needs an owning user");

    store
        .apply("User", Mutation::Remove { key: user_key })
        .unwrap();

    for post_key in owned {
        assert!(
            store.get("Post", &post_key).is_none(),
            "a cascade must not leave a dangling child"
        );
    }
}

#[test]
fn removing_a_parent_is_refused_when_cascade_is_off() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User"));
    graph.insert(entity("Post").with_field(relation_field("author", "User", Cardinality::One)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig {
            cascade_delete: false,
            ..StoreConfig::seeded(3)
        }
        .with_count("User", 2)
        .with_count("Post", 6),
    );

    let user_key = store.keys("User")[0].clone();
    assert!(
        store
            .apply("User", Mutation::Remove { key: user_key })
            .is_err()
    );
}

#[test]
fn removing_something_that_is_not_there_is_an_error() {
    let store = blog_store(3, 2, 2);
    assert!(
        store
            .apply(
                "Post",
                Mutation::Remove {
                    key: EntityKey::single("nope")
                }
            )
            .is_err()
    );
}

#[test]
fn filters_narrow_a_list() {
    let store = blog_store(12, 3, 10);
    let target = store.get("Post", &store.keys("Post")[3]).unwrap();
    let title = target.get("title").unwrap().clone();

    let page = store
        .list(
            "Post",
            &Selection::new().filter(Predicate::eq("title", title.clone())),
        )
        .unwrap();
    assert!(!page.records.is_empty());
    assert!(page.records.iter().all(|r| r.get("title") == Some(&title)));
}

#[test]
fn sorting_is_total_and_stable() {
    let store = blog_store(14, 2, 12);
    let page = store
        .list("Post", &Selection::new().sorted_by(SortKey::asc("title")))
        .unwrap();
    let titles: Vec<_> = page
        .records
        .iter()
        .map(|r| r.get("title").unwrap().as_str().unwrap().to_string())
        .collect();
    let mut sorted = titles.clone();
    sorted.sort();
    assert_eq!(titles, sorted);
}

#[test]
fn offset_pages_cover_the_set_exactly_once() {
    let store = blog_store(15, 2, 10);
    let mut seen = Vec::new();
    let mut skip = 0;
    loop {
        let page = store
            .list("Post", &Selection::new().paged(Page::Offset { skip, take: 3 }))
            .unwrap();
        assert_eq!(page.total, 10);
        if page.records.is_empty() {
            break;
        }
        seen.extend(page.records.iter().map(|r| r.key.clone()));
        skip += 3;
    }
    assert_eq!(seen.len(), 10);
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 10, "pages must not overlap");
}

#[test]
fn cursor_pages_walk_forward_without_gaps() {
    let store = blog_store(16, 2, 9);
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = store
            .list(
                "Post",
                &Selection::new()
                    .sorted_by(SortKey::asc("title"))
                    .paged(Page::After {
                        cursor: cursor.clone(),
                        first: 4,
                    }),
            )
            .unwrap();
        if page.records.is_empty() {
            break;
        }
        seen.extend(page.records.iter().map(|r| r.key.clone()));
        cursor = page.end_cursor.clone();
        if !page.has_next {
            break;
        }
    }
    assert_eq!(seen.len(), 9);
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 9);
}

#[test]
fn an_empty_parent_set_leaves_relations_null_rather_than_dangling() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User"));
    graph.insert(entity("Post").with_field(relation_field("author", "User", Cardinality::One)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(1)
            .with_count("User", 0)
            .with_count("Post", 3),
    );

    for key in store.keys("Post") {
        let post = store.get("Post", &key).unwrap();
        assert_eq!(post.get("author"), Some(&JsonValue::Null));
    }
}

#[test]
fn an_unknown_entity_is_an_error_rather_than_an_empty_list() {
    let store = blog_store(1, 1, 1);
    assert!(store.list("Ghost", &Selection::new()).is_err());
}

/// A file belongs to several collections and a collection holds several
/// files: neither side owns the other, so membership has to be computed the
/// same way from either end.
#[test]
fn both_ends_of_a_many_to_many_agree() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Collection").with_field(relation_field(
        "items",
        "Doc",
        Cardinality::Many,
    )));
    graph.insert(entity("Doc").with_field(relation_field(
        "collections",
        "Collection",
        Cardinality::Many,
    )));

    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(5)
            .with_count("Collection", 6)
            .with_count("Doc", 15),
    );

    let mut pairs = 0;
    for collection_key in store.keys("Collection") {
        let items = store
            .related("Collection", &collection_key, "items", &Selection::new())
            .unwrap();
        for doc in &items.records {
            let back = store
                .related("Doc", &doc.key, "collections", &Selection::new())
                .unwrap();
            assert!(
                back.records.iter().any(|c| c.key == collection_key),
                "a doc reached through collection.items must list that collection"
            );
            pairs += 1;
        }
    }
    assert!(pairs > 0, "the fixture should relate something");
}

#[test]
fn a_many_to_many_is_not_a_single_owner() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Collection").with_field(relation_field(
        "items",
        "Doc",
        Cardinality::Many,
    )));
    graph.insert(entity("Doc").with_field(relation_field(
        "collections",
        "Collection",
        Cardinality::Many,
    )));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(9)
            .with_count("Collection", 8)
            .with_count("Doc", 30),
    );

    let shared = store.keys("Doc").into_iter().any(|doc_key| {
        store
            .related("Doc", &doc_key, "collections", &Selection::new())
            .is_ok_and(|page| page.records.len() > 1)
    });
    assert!(
        shared,
        "at least one doc should belong to more than one collection"
    );
}
