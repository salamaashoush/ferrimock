#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::core::world::algebra::{Predicate, Selection, SortKey};
use crate::core::world::model::{
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
        crate::core::world::model::CompositeKey::single("id"),
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
            store.get("User", &EntityKey::single(author_key)).is_some(),
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
        .apply(
            "User",
            Mutation::Insert {
                values: values.clone(),
            },
        )
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

/// A replacement has to be a replacement on the next read too. Storing it as
/// a patch made the same verb mean two things: on a seeded record the delta
/// was merged back over the derived values, so a `PUT` that dropped a field
/// got that field back; on a created record it was returned verbatim.
#[test]
fn a_replace_still_reads_as_a_replace_on_the_next_get() {
    let store = blog_store(21, 3, 6);
    let key = store.keys("Post").into_iter().next().unwrap();
    assert!(store.get("Post", &key).unwrap().get("title").is_some());

    let replaced = store
        .apply(
            "Post",
            Mutation::Replace {
                key: key.clone(),
                values: serde_json::json!({ "author": null }),
            },
        )
        .unwrap();
    let Written::Updated(answered) = replaced else {
        panic!("a replace answers with the record")
    };
    assert!(answered.get("title").is_none());

    let read = store.get("Post", &key).unwrap();
    assert_eq!(
        read.fields, answered.fields,
        "the response to a PUT and the next GET are the same record"
    );
    assert!(
        read.get("title").is_none(),
        "a field the caller dropped came back from the derived layer"
    );
}

/// The same verb on a record the client created, which is where replacing
/// already worked — both provenances have to answer the same way.
#[test]
fn a_replace_reads_the_same_way_whatever_the_record_was() {
    let store = blog_store(21, 3, 6);
    let Written::Created(made) = store
        .apply(
            "Post",
            Mutation::Insert {
                values: serde_json::json!({ "title": "First" }),
            },
        )
        .unwrap()
    else {
        panic!("an insert answers with the record")
    };

    for key in [made.key, store.keys("Post").into_iter().next().unwrap()] {
        let Written::Updated(answered) = store
            .apply(
                "Post",
                Mutation::Replace {
                    key: key.clone(),
                    values: serde_json::json!({ "title": "Only" }),
                },
            )
            .unwrap()
        else {
            panic!("a replace answers with the record")
        };
        assert_eq!(store.get("Post", &key).unwrap().fields, answered.fields);
    }
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
    assert_eq!(
        updated.get("id").unwrap().as_str().unwrap(),
        key.to_string()
    );
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
    // Whichever user has children: they are not spread evenly, so the first
    // one may well have none.
    let (user_key, owned) = store
        .keys("User")
        .into_iter()
        .find_map(|user_key| {
            let owned: Vec<_> = store
                .related("User", &user_key, "posts", &Selection::new())
                .unwrap()
                .records
                .iter()
                .map(|r| r.key.clone())
                .collect();
            (!owned.is_empty()).then_some((user_key, owned))
        })
        .expect("the fixture needs an owning user");

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
            .list(
                "Post",
                &Selection::new().paged(Page::Offset { skip, take: 3 }),
            )
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

/// A list with nothing to filter or sort by is answered from the census, so
/// only the requested window is derived. That fast path must agree with the
/// general one exactly — it is the difference between microseconds and tens of
/// milliseconds on a large entity, and a page that disagreed would be worse
/// than a slow one.
#[test]
fn the_census_fast_path_agrees_with_materialising_everything() {
    let store = blog_store(11, 40, 5);

    // The general path, forced by a sort that does not change the order the
    // keys were already in.
    let materialised = |page: Page| {
        let mut records: Vec<Record> = store
            .keys("User")
            .into_iter()
            .filter_map(|key| store.get("User", &key))
            .collect();
        sort_records(&mut records, &[]);
        paginate(&records, &page)
    };

    let cursor = {
        let first = store.list(
            "User",
            &Selection::new().paged(Page::Offset { skip: 3, take: 1 }),
        );
        first.unwrap().end_cursor.unwrap()
    };

    for page in [
        Page::All,
        Page::Offset { skip: 0, take: 10 },
        Page::Offset { skip: 7, take: 10 },
        // Past the end, and a window that runs off it.
        Page::Offset {
            skip: 100,
            take: 10,
        },
        Page::Offset { skip: 35, take: 10 },
        Page::After {
            cursor: Some(cursor.clone()),
            first: 5,
        },
        Page::After {
            cursor: None,
            first: 5,
        },
        Page::Before {
            cursor: Some(cursor),
            last: 5,
        },
        Page::Before {
            cursor: None,
            last: 5,
        },
    ] {
        let fast = store
            .list("User", &Selection::new().paged(page.clone()))
            .unwrap();
        let slow = materialised(page.clone());

        assert_eq!(fast.total, slow.total, "total for {page:?}");
        assert_eq!(fast.has_next, slow.has_next, "has_next for {page:?}");
        assert_eq!(
            fast.has_previous, slow.has_previous,
            "has_previous for {page:?}"
        );
        assert_eq!(
            fast.start_cursor, slow.start_cursor,
            "start_cursor for {page:?}"
        );
        assert_eq!(fast.end_cursor, slow.end_cursor, "end_cursor for {page:?}");
        assert_eq!(fast.records, slow.records, "records for {page:?}");
    }
}

/// The fast path reads through the delta like any other read.
#[test]
fn the_census_fast_path_sees_writes() {
    let store = blog_store(11, 5, 2);

    let created = match store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "name": "Ada" }),
            },
        )
        .unwrap()
    {
        Written::Created(record) => record.key,
        other => panic!("expected a creation, got {other:?}"),
    };
    let removed = store.keys("User")[0].clone();
    store
        .apply(
            "User",
            Mutation::Remove {
                key: removed.clone(),
            },
        )
        .unwrap();

    let page = store.list("User", &Selection::new()).unwrap();
    let keys: Vec<_> = page.records.iter().map(|r| r.key.clone()).collect();

    assert_eq!(page.total, 5, "one created, one removed, five to start");
    assert!(keys.contains(&created), "a creation has to appear");
    assert!(!keys.contains(&removed), "a tombstone has to disappear");
}

// ===== Regressions =====

/// An entity keyed by a declared integer, the shape most REST documents use.
fn numbered_store(seed: u64, count: usize) -> EntityStore {
    let mut graph = EntityGraph::new();
    graph.insert(
        EntityType::new(
            "User",
            crate::core::world::model::CompositeKey::single("id"),
            Provenance::new(Rule::CollectionItemPair, "User"),
        )
        // Named `id`, so the detector reads it as a uuid; the declared kind has
        // to win or every integer-keyed document is keyed by uuids.
        .with_field(FieldDef::new(
            "id",
            ValueSpec::Scalar(
                Scalar::new(ScalarKind::Int).with_semantic(crate::type_detector::FieldType::Uuid),
            ),
            false,
        ))
        .with_field(scalar_field("name", ScalarKind::String)),
    );
    EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed).with_count("User", count),
    )
}

#[test]
fn a_declared_integer_key_is_an_integer_on_the_wire() {
    let store = numbered_store(3, 4);
    let keys = store.keys("User");
    assert_eq!(
        keys.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["1", "2", "3", "4"],
        "an integer key counts"
    );

    let record = store.get("User", &EntityKey::single("1")).unwrap();
    assert_eq!(
        record.fields.get("id"),
        Some(&JsonValue::from(1)),
        "the payload carries the kind the schema declared, not a string"
    );
}

#[test]
fn a_written_integer_key_stays_an_integer() {
    let store = numbered_store(3, 2);
    let written = store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "id": 42, "name": "Grace" }),
            },
        )
        .unwrap();
    let Written::Created(record) = written else {
        panic!("expected a creation")
    };
    assert_eq!(record.fields.get("id"), Some(&JsonValue::from(42)));
}

#[test]
fn a_page_past_the_end_does_not_overflow() {
    let store = blog_store(5, 6, 6);
    // Sorting forces the materialising path, where an unbounded `take` used to
    // be added to a non-zero offset and wrap.
    let selection = Selection::new()
        .sorted_by(SortKey::asc("name"))
        .paged(Page::Offset {
            skip: 1,
            take: usize::MAX,
        });
    let page = store.list("User", &selection).unwrap();
    assert_eq!(page.total, 6);
    assert_eq!(page.records.len(), 5, "everything after the first");

    let cursor = Cursor::new(store.keys("User")[0].to_string());
    let after = Selection::new()
        .sorted_by(SortKey::asc("name"))
        .paged(Page::After {
            cursor: Some(cursor),
            first: usize::MAX,
        });
    assert!(store.list("User", &after).is_ok());
}

#[test]
fn a_created_record_carries_the_links_a_seeded_one_does() {
    let store = blog_store(11, 4, 4);
    let Written::Created(post) = store
        .apply(
            "Post",
            Mutation::Insert {
                values: serde_json::json!({ "title": "Hello" }),
            },
        )
        .unwrap()
    else {
        panic!("expected a creation")
    };

    let author = post.fields.get("author").and_then(JsonValue::as_str);
    assert!(
        author.is_some(),
        "a created record with no link is a record a client cannot render"
    );

    let relation = store
        .graph()
        .get("Post")
        .unwrap()
        .field("author")
        .unwrap()
        .relation()
        .unwrap()
        .clone();
    let resolved = store.relation_target("Post", &post.key, "author", &relation);
    assert!(resolved.is_some(), "and the link has to resolve");
}

#[test]
fn a_created_record_joins_the_collection_it_points_at() {
    let store = blog_store(11, 4, 4);
    let owner = store.keys("User")[2].clone();
    let owner_id = owner.to_string();

    let Written::Created(post) = store
        .apply(
            "Post",
            Mutation::Insert {
                values: serde_json::json!({ "title": "Written", "author": owner_id }),
            },
        )
        .unwrap()
    else {
        panic!("expected a creation")
    };

    let page = store
        .related("User", &owner, "posts", &Selection::new())
        .unwrap();
    assert!(
        page.records.iter().any(|record| record.key == post.key),
        "a post written against a user has to appear among that user's posts"
    );

    for other in store.keys("User").iter().filter(|key| **key != owner) {
        let page = store
            .related("User", other, "posts", &Selection::new())
            .unwrap();
        assert!(
            !page.records.iter().any(|record| record.key == post.key),
            "and among nobody else's"
        );
    }
}

#[test]
fn a_write_names_a_link_however_the_caller_spells_it() {
    let store = blog_store(11, 4, 4);
    let owner = store.keys("User")[1].to_string();

    for values in [
        serde_json::json!({ "title": "a", "author": owner }),
        serde_json::json!({ "title": "b", "author": { "id": owner } }),
        serde_json::json!({ "title": "c", "authorId": owner }),
        serde_json::json!({ "title": "d", "author_id": owner }),
    ] {
        let Written::Created(post) = store.apply("Post", Mutation::Insert { values }).unwrap()
        else {
            panic!("expected a creation")
        };
        assert_eq!(
            post.fields.get("author").and_then(JsonValue::as_str),
            Some(owner.as_str()),
            "every spelling of the link means the same link"
        );
        assert!(
            !post.fields.contains_key("authorId") && !post.fields.contains_key("author_id"),
            "an input alias is consumed, not left on the record"
        );
    }
}

#[test]
fn a_grown_count_does_not_hand_out_a_created_key_twice() {
    let graph = {
        let mut graph = EntityGraph::new();
        graph.insert(entity("User").with_field(scalar_field("name", ScalarKind::String)));
        Arc::new(graph)
    };

    let store = EntityStore::new(
        Arc::clone(&graph),
        StoreConfig::seeded(7).with_count("User", 5),
    );
    let Written::Created(created) = store
        .apply(
            "User",
            Mutation::Insert {
                values: serde_json::json!({ "name": "Ada" }),
            },
        )
        .unwrap()
    else {
        panic!("expected a creation")
    };

    // The rebuild a hot reload performs, with the entity grown past the ordinal
    // the created record was minted at.
    let snapshot = store.export_delta();
    let grown = EntityStore::new_reserving(
        graph,
        StoreConfig::seeded(7).with_count("User", 8),
        snapshot.reserved_keys(),
    );
    grown.import_delta(snapshot);

    let keys = grown.keys("User");
    let unique: std::collections::BTreeSet<_> = keys.iter().collect();
    assert_eq!(unique.len(), keys.len(), "no key is served twice");
    assert_eq!(keys.len(), 9, "eight derived and the one that was created");
    assert!(
        keys.contains(&created.key),
        "the created record keeps the key a client already saw"
    );
    assert_eq!(grown.count("User"), 9);
}

#[test]
fn a_removed_record_is_counted_once() {
    let store = blog_store(2, 4, 4);
    let key = store.keys("Post")[0].clone();
    store
        .apply("Post", Mutation::Remove { key: key.clone() })
        .unwrap();
    assert_eq!(store.count("Post"), 3);
    // Removing it again fails rather than counting a second time.
    assert!(store.apply("Post", Mutation::Remove { key }).is_err());
    assert_eq!(store.count("Post"), 3);
}

#[test]
fn a_key_carried_by_a_sibling_field_is_the_same_link() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User").with_field(scalar_field("name", ScalarKind::String)));

    // What a document declaring both `user_id` and an embedded `customer`
    // compiles to: one link, carried by the scalar.
    let customer = FieldDef::new(
        "customer",
        ValueSpec::Relation(Box::new(Relation::new(
            "User",
            Cardinality::One,
            Carrier::ForeignKey(LeanString::from("user_id")),
            Confidence::STRUCTURAL,
            Provenance::new(Rule::SchemaRef, "Order.customer"),
        ))),
        true,
    );
    graph.insert(
        entity("Order")
            .with_field(scalar_field("user_id", ScalarKind::Id))
            .with_field(customer),
    );

    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(4)
            .with_count("User", 5)
            .with_count("Order", 5),
    );

    let relation = store
        .graph()
        .get("Order")
        .unwrap()
        .field("customer")
        .unwrap()
        .relation()
        .unwrap()
        .clone();

    for key in store.keys("Order") {
        let record = store.get("Order", &key).unwrap();
        let stated = record
            .fields
            .get("user_id")
            .and_then(JsonValue::as_str)
            .unwrap();
        let target = store
            .relation_target("Order", &key, "customer", &relation)
            .expect("the link resolves");
        assert_eq!(
            target.key.to_string(),
            stated,
            "the object and the key it is carried by name one user"
        );
        assert!(
            !record.fields.contains_key("customer"),
            "the carrier holds the key; the object is written by whoever expands it"
        );
    }
}

#[test]
fn a_long_chain_of_entities_does_not_overflow_the_stack() {
    const DEPTH: usize = 40_000;
    let mut graph = EntityGraph::new();
    for level in 0..DEPTH {
        let name = format!("Level{level}");
        let mut entity = entity(&name);
        if level + 1 < DEPTH {
            entity = entity.with_field(relation_field(
                "next",
                &format!("Level{}", level + 1),
                Cardinality::One,
            ));
        }
        graph.insert(entity);
    }

    let order = graph.seed_order();
    assert_eq!(order.order.len(), DEPTH);
    assert_eq!(
        order.order.first().map(LeanString::as_str),
        Some(format!("Level{}", DEPTH - 1).as_str()),
        "the end of the chain is seeded first"
    );
}

// ===== Distribution, counts and coherence =====

#[test]
fn children_are_not_spread_evenly_across_parents() {
    let store = blog_store(21, 20, 400);
    let sizes: Vec<usize> = store
        .keys("User")
        .into_iter()
        .map(|user| {
            store
                .related("User", &user, "posts", &Selection::new())
                .unwrap()
                .total
        })
        .collect();

    let total: usize = sizes.iter().sum();
    assert_eq!(total, 400, "every post still belongs to exactly one user");

    let busiest = sizes.iter().copied().max().unwrap_or(0);
    let even = 400 / 20;
    assert!(
        busiest >= even * 2,
        "real data is lopsided; the busiest user has {busiest} of 400 across 20 users: {sizes:?}"
    );
    assert!(
        sizes.iter().filter(|size| **size < even / 2).count() >= 3,
        "and most have far fewer than their share: {sizes:?}"
    );
}

#[test]
fn a_lopsided_relation_still_agrees_in_both_directions() {
    let store = blog_store(13, 12, 60);
    for user in store.keys("User") {
        let posts = store
            .related("User", &user, "posts", &Selection::new())
            .unwrap();
        for post in posts.records {
            let relation = store
                .graph()
                .get("Post")
                .unwrap()
                .field("author")
                .unwrap()
                .relation()
                .unwrap()
                .clone();
            let author = store
                .relation_target("Post", &post.key, "author", &relation)
                .expect("every post has an author");
            assert_eq!(
                author.key, user,
                "a post in a user's collection has to name that user back"
            );
        }
    }
}

/// A folder with an `item_count` beside its `items`, which is what real
/// payloads carry.
fn counted_store(seed: u64, folders: usize, files: usize) -> EntityStore {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Folder")
            .with_field(scalar_field("item_count", ScalarKind::Int))
            .with_field(relation_field("items", "File", Cardinality::Many)),
    );
    graph.insert(entity("File").with_field(relation_field("folder", "Folder", Cardinality::One)));
    EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed)
            .with_count("Folder", folders)
            .with_count("File", files),
    )
}

#[test]
fn a_count_field_reports_what_the_collection_actually_holds() {
    let store = counted_store(4, 8, 50);
    for folder in store.keys("Folder") {
        let record = store.get("Folder", &folder).unwrap();
        let stated = record.get("item_count").unwrap().as_u64().unwrap();
        let held = store
            .related("Folder", &folder, "items", &Selection::new())
            .unwrap()
            .total;
        assert_eq!(
            usize::try_from(stated).unwrap(),
            held,
            "`item_count` disagreeing with the list endpoint is worse than no field"
        );
    }
}

#[test]
fn a_count_field_follows_a_write() {
    let store = counted_store(4, 8, 50);
    let folder = store
        .keys("Folder")
        .into_iter()
        .find(|folder| {
            store
                .related("Folder", folder, "items", &Selection::new())
                .unwrap()
                .total
                > 0
        })
        .expect("some folder holds files");

    let before = store
        .get("Folder", &folder)
        .unwrap()
        .get("item_count")
        .unwrap()
        .as_u64()
        .unwrap();

    store
        .apply(
            "File",
            Mutation::Insert {
                values: serde_json::json!({ "folder": folder.to_string() }),
            },
        )
        .unwrap();

    let after = store
        .get("Folder", &folder)
        .unwrap()
        .get("item_count")
        .unwrap()
        .as_u64()
        .unwrap();
    assert_eq!(after, before + 1, "a file added to a folder counts");

    let held = store
        .related("Folder", &folder, "items", &Selection::new())
        .unwrap()
        .total;
    assert_eq!(
        usize::try_from(after).unwrap(),
        held,
        "and the two still agree"
    );
}

#[test]
fn a_count_field_that_names_nothing_stays_an_ordinary_number() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Post").with_field(scalar_field("word_count", ScalarKind::Int)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(2).with_count("Post", 4),
    );

    let counts: Vec<u64> = store
        .keys("Post")
        .into_iter()
        .filter_map(|key| store.get("Post", &key)?.get("word_count")?.as_u64())
        .collect();
    assert!(
        counts.iter().any(|count| *count > 1),
        "`word_count` counts no relation, so it is just a number: {counts:?}"
    );
}

#[test]
fn a_link_to_a_composite_keyed_entity_resolves() {
    use crate::core::world::model::{CompositeKey, KeyPart, KeySource};

    let repo = EntityType::new(
        "Repo",
        CompositeKey::parts([
            KeyPart {
                field: LeanString::from("owner"),
                source: KeySource::PathParam(LeanString::from("owner")),
            },
            KeyPart {
                field: LeanString::from("repo"),
                source: KeySource::PathParam(LeanString::from("repo")),
            },
        ]),
        Provenance::new(Rule::CollectionItemPair, "Repo"),
    )
    .with_field(scalar_field("owner", ScalarKind::String))
    .with_field(scalar_field("repo", ScalarKind::String));

    let mut graph = EntityGraph::new();
    graph.insert(repo);
    graph.insert(entity("Issue").with_field(relation_field(
        "repository",
        "Repo",
        Cardinality::One,
    )));

    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(6)
            .with_count("Repo", 4)
            .with_count("Issue", 12),
    );

    let relation = store
        .graph()
        .get("Issue")
        .unwrap()
        .field("repository")
        .unwrap()
        .relation()
        .unwrap()
        .clone();

    for key in store.keys("Issue") {
        let record = store.get("Issue", &key).unwrap();
        let carried = record.get("repository").unwrap().as_str().unwrap();
        assert!(
            carried.contains('/'),
            "a key of two parts is carried as both of them: {carried}"
        );
        let target = store
            .relation_target("Issue", &key, "repository", &relation)
            .expect("a link to a composite-keyed entity has to resolve");
        assert_eq!(target.key.to_string(), carried);
        assert_eq!(
            target.get("owner").unwrap().as_str().unwrap(),
            carried.split('/').next().unwrap(),
            "and the parts are the fields they were derived from"
        );
    }
}

#[test]
fn a_field_name_outside_ascii_does_not_break_the_count_check() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Thing")
            .with_field(scalar_field("名前", ScalarKind::String))
            .with_field(scalar_field("cnt", ScalarKind::Int)),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(1).with_count("Thing", 3),
    );

    for key in store.keys("Thing") {
        let record = store.get("Thing", &key).expect("a record still reads");
        assert!(record.get("名前").is_some());
    }
}

/// A folder tree: one relation, both of its directions declared on the same
/// entity, with a count field beside them.
fn tree_store(seed: u64, folders: usize) -> EntityStore {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Folder")
            .with_field(scalar_field("children_count", ScalarKind::Int))
            .with_field(relation_field("parent", "Folder", Cardinality::One))
            .with_field(relation_field("children", "Folder", Cardinality::Many)),
    );
    EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed).with_count("Folder", folders),
    )
}

fn relation_of<'a>(store: &'a EntityStore, entity: &str, field: &str) -> &'a Relation {
    store
        .graph()
        .get(entity)
        .unwrap()
        .field(field)
        .unwrap()
        .relation()
        .unwrap()
}

#[test]
fn a_self_relation_answers_the_same_way_from_both_ends() {
    let store = tree_store(3, 24);
    let parent_relation = relation_of(&store, "Folder", "parent");

    for folder in store.keys("Folder") {
        let children = store
            .related("Folder", &folder, "children", &Selection::new())
            .unwrap();
        for child in &children.records {
            let parent = store
                .relation_target("Folder", &child.key, "parent", parent_relation)
                .unwrap();
            assert_eq!(
                parent.key, folder,
                "folder.children must hold exactly the folders whose parent is that folder"
            );
        }
        let stated = store
            .get("Folder", &folder)
            .unwrap()
            .get("children_count")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(
            usize::try_from(stated).unwrap(),
            children.total,
            "`children_count` must agree with the collection it counts"
        );
    }
}

#[test]
fn an_unrelated_back_edge_does_not_move_a_relation_onto_membership() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("User")
            .with_field(scalar_field("post_count", ScalarKind::Int))
            .with_field(relation_field("posts", "Post", Cardinality::Many)),
    );
    graph.insert(
        entity("Post")
            .with_field(relation_field("author", "User", Cardinality::One))
            .with_field(relation_field("liked_by", "User", Cardinality::Many)),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(17)
            .with_count("User", 6)
            .with_count("Post", 30),
    );
    let author_relation = relation_of(&store, "Post", "author");

    let mut reached = 0;
    for user in store.keys("User") {
        let posts = store
            .related("User", &user, "posts", &Selection::new())
            .unwrap();
        for post in &posts.records {
            let author = store
                .relation_target("Post", &post.key, "author", author_relation)
                .unwrap();
            assert_eq!(
                author.key, user,
                "a to-many beside a real foreign key must not change how the link resolves"
            );
        }
        reached += posts.total;
        let stated = store
            .get("User", &user)
            .unwrap()
            .get("post_count")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(usize::try_from(stated).unwrap(), posts.total);
    }
    assert_eq!(
        reached,
        store.count("Post"),
        "every post is owned by exactly one user"
    );
}

#[test]
fn a_count_field_on_a_many_to_many_reports_what_the_collection_holds() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Collection")
            .with_field(scalar_field("item_count", ScalarKind::Int))
            .with_field(relation_field("items", "Doc", Cardinality::Many)),
    );
    graph.insert(
        entity("Doc")
            .with_field(scalar_field("collection_count", ScalarKind::Int))
            .with_field(relation_field(
                "collections",
                "Collection",
                Cardinality::Many,
            )),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(5)
            .with_count("Collection", 6)
            .with_count("Doc", 20),
    );

    for collection in store.keys("Collection") {
        let held = store
            .related("Collection", &collection, "items", &Selection::new())
            .unwrap()
            .total;
        let stated = store
            .get("Collection", &collection)
            .unwrap()
            .get("item_count")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(usize::try_from(stated).unwrap(), held);
    }
    for doc in store.keys("Doc") {
        let held = store
            .related("Doc", &doc, "collections", &Selection::new())
            .unwrap()
            .total;
        let stated = store
            .get("Doc", &doc)
            .unwrap()
            .get("collection_count")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(usize::try_from(stated).unwrap(), held);
    }
}

#[test]
fn a_membership_an_entity_has_with_itself_is_symmetric() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User").with_field(relation_field("friends", "User", Cardinality::Many)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(21).with_count("User", 20),
    );

    let mut pairs = 0;
    for user in store.keys("User") {
        let friends = store
            .related("User", &user, "friends", &Selection::new())
            .unwrap();
        for friend in &friends.records {
            let back = store
                .related("User", &friend.key, "friends", &Selection::new())
                .unwrap();
            assert!(
                back.records.iter().any(|other| other.key == user),
                "friendship has one side, so both ends must read it the same way"
            );
            pairs += 1;
        }
    }
    assert!(pairs > 0, "the fixture should relate something");
}

#[test]
fn a_to_many_with_no_link_back_still_counts_what_it_lists() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Feed")
            .with_field(scalar_field("item_count", ScalarKind::Int))
            .with_field(relation_field("items", "Item", Cardinality::Many)),
    );
    graph.insert(entity("Item").with_field(scalar_field("title", ScalarKind::String)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(8)
            .with_count("Feed", 5)
            .with_count("Item", 40),
    );

    let mut seen = 0;
    for feed in store.keys("Feed") {
        let held = store
            .related("Feed", &feed, "items", &Selection::new())
            .unwrap()
            .total;
        let stated = store
            .get("Feed", &feed)
            .unwrap()
            .get("item_count")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(usize::try_from(stated).unwrap(), held);
        seen += held;
    }
    assert_eq!(seen, store.count("Item"));
}

#[test]
fn a_hierarchy_has_roots() {
    let store = tree_store(3, 40);
    let parent_relation = relation_of(&store, "Folder", "parent");

    let roots = store
        .keys("Folder")
        .into_iter()
        .filter(|key| {
            store
                .relation_target("Folder", key, "parent", parent_relation)
                .is_none()
        })
        .count();
    assert!(roots > 0, "a tree with no root is not a tree");
    assert!(roots < store.count("Folder"), "and not every folder is one");

    for key in store.keys("Folder") {
        let record = store.get("Folder", &key).unwrap();
        let stated = record.get("parent").unwrap();
        let resolves = store
            .relation_target("Folder", &key, "parent", parent_relation)
            .is_some();
        assert_eq!(
            stated.is_null(),
            !resolves,
            "a root says so rather than naming a folder that is not its parent"
        );
    }
}

/// Every chain terminates at a root, at every seed and every size.
///
/// A partition of a census against itself has a fixed point for every seed:
/// the owning map is monotone over a rising boundary vector, so
/// `owner_of(i) - i` has to cross zero. Levels are what make that impossible
/// rather than merely unlikely, so this walks the whole world rather than
/// sampling it.
#[test]
fn no_chain_of_parents_ever_returns_to_where_it_started() {
    for seed in 0..32 {
        let store = tree_store(seed, 50);
        let total = store.count("Folder");

        for key in store.keys("Folder") {
            let walked = ancestors_of(&store, key.clone(), total);
            let mut seen = walked.clone();
            seen.push(key.clone());
            seen.sort_by_key(ToString::to_string);
            let before = seen.len();
            seen.dedup();
            assert_eq!(
                before,
                seen.len(),
                "seed {seed}: the chain from {key} repeats"
            );
            assert!(
                walked.len() < total,
                "seed {seed}: the chain from {key} does not end"
            );
        }
    }
}

/// Every folder above one, nearest first, stopping at `bound` hops.
///
/// The bound is the point: a chain with a fixed point in it never ends, and a
/// test that hangs reports nothing at all.
fn ancestors_of(store: &EntityStore, from: EntityKey, bound: usize) -> Vec<EntityKey> {
    let parent_relation = relation_of(store, "Folder", "parent");
    let mut walked = Vec::new();
    let mut at = from;
    while walked.len() <= bound {
        let Some(parent) = store.relation_target("Folder", &at, "parent", parent_relation) else {
            break;
        };
        walked.push(parent.key.clone());
        at = parent.key;
    }
    walked
}

#[test]
fn a_hierarchy_is_deep_rather_than_flat() {
    let store = tree_store(11, 200);
    let total = store.count("Folder");

    let deepest = store
        .keys("Folder")
        .into_iter()
        .map(|key| ancestors_of(&store, key, total).len())
        .max()
        .unwrap_or(0);
    assert!(
        (2..=8).contains(&deepest),
        "a hierarchy should have a few levels, not one and not two hundred: {deepest}"
    );
}

/// A one-level cascade over a tree leaves every generation below the second
/// pointing at a record that is gone.
#[test]
fn removing_a_parent_cascades_to_every_generation() {
    let store = tree_store(5, 60);
    let parent_relation = relation_of(&store, "Folder", "parent");

    let root = store
        .keys("Folder")
        .into_iter()
        .find(|key| {
            store
                .relation_target("Folder", key, "parent", parent_relation)
                .is_none()
                && !store
                    .related("Folder", key, "children", &Selection::new())
                    .unwrap()
                    .records
                    .is_empty()
        })
        .expect("a root with children");

    let total = store.count("Folder");
    let descendants: Vec<EntityKey> = store
        .keys("Folder")
        .into_iter()
        .filter(|key| ancestors_of(&store, key.clone(), total).contains(&root))
        .collect();
    assert!(
        descendants.len() > 2,
        "the fixture should have more than one generation below the root"
    );

    store
        .apply("Folder", Mutation::Remove { key: root.clone() })
        .unwrap();

    assert!(store.get("Folder", &root).is_none());
    for key in &descendants {
        assert!(
            store.get("Folder", key).is_none(),
            "a descendant left behind points at a record that no longer exists"
        );
    }
    for key in store.keys("Folder") {
        if let Some(parent) = store.relation_target("Folder", &key, "parent", parent_relation) {
            assert!(
                store.get("Folder", &parent.key).is_some(),
                "every surviving folder's parent still exists"
            );
        }
    }
}

/// The cascade is one generation deep in a chain of entities too, not only in
/// a hierarchy — the tree is what makes it common, not what makes it wrong.
#[test]
fn a_cascade_follows_a_chain_of_entities_all_the_way_down() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Org"));
    graph.insert(entity("Team").with_field(relation_field("org", "Org", Cardinality::One)));
    graph.insert(entity("Member").with_field(relation_field("team", "Team", Cardinality::One)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(3)
            .with_count("Org", 4)
            .with_count("Team", 12)
            .with_count("Member", 40),
    );

    let team_of = relation_of(&store, "Team", "org");
    let member_of = relation_of(&store, "Member", "team");
    let org = store
        .keys("Org")
        .into_iter()
        .find(|org| {
            store.keys("Team").iter().any(|team| {
                store
                    .relation_target("Team", team, "org", team_of)
                    .is_some_and(|owner| &owner.key == org)
                    && store.keys("Member").iter().any(|member| {
                        store
                            .relation_target("Member", member, "team", member_of)
                            .is_some_and(|owner| &owner.key == team)
                    })
            })
        })
        .expect("an org with a team that has a member");

    store.apply("Org", Mutation::Remove { key: org }).unwrap();

    for member in store.keys("Member") {
        let team = store
            .relation_target("Member", &member, "team", member_of)
            .expect("a surviving member still belongs to a team");
        assert!(
            store.get("Team", &team.key).is_some(),
            "the team a surviving member points at was tombstoned two levels up"
        );
    }
}

/// A schema's shape says how many of each thing there are. One constant for
/// every entity says the opposite, and it is wrong in the direction a client
/// notices: the collection it pages through most is the one furthest down.
#[test]
fn an_entity_further_down_the_graph_is_more_numerous() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User"));
    graph.insert(entity("Folder").with_field(relation_field("owner", "User", Cardinality::One)));
    graph.insert(entity("File").with_field(relation_field("folder", "Folder", Cardinality::One)));
    let store = EntityStore::new(Arc::new(graph), StoreConfig::seeded(1));

    assert_eq!(store.count("User"), DEFAULT_SEED_COUNT);
    assert!(store.count("Folder") > store.count("User"));
    assert!(store.count("File") > store.count("Folder"));
    assert!(
        store.count("User") > crate::core::world::algebra::DEFAULT_PAGE_SIZE,
        "one unpaginated request must not return the whole population"
    );
}

#[test]
fn a_self_relation_is_not_a_step_down_the_graph() {
    let store = tree_store(1, 30);
    assert_eq!(store.count("Folder"), 30);

    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Folder")
            .with_field(relation_field("parent", "Folder", Cardinality::One))
            .with_field(relation_field("children", "Folder", Cardinality::Many)),
    );
    let derived = EntityStore::new(Arc::new(graph), StoreConfig::seeded(1));
    assert_eq!(
        derived.count("Folder"),
        DEFAULT_SEED_COUNT,
        "a hierarchy is one entity, and its depth is levels within itself"
    );
}

#[test]
fn the_fanout_stops_rather_than_running_away() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("L0"));
    for depth in 1..10 {
        graph.insert(entity(&format!("L{depth}")).with_field(relation_field(
            "up",
            &format!("L{}", depth - 1),
            Cardinality::One,
        )));
    }
    let store = EntityStore::new(Arc::new(graph), StoreConfig::seeded(1));
    assert_eq!(store.count("L9"), MAX_SEED_COUNT);
}

#[test]
fn a_scale_multiplies_the_default_and_leaves_a_stated_count_alone() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User"));
    graph.insert(entity("Folder").with_field(relation_field("owner", "User", Cardinality::One)));

    let mut config = StoreConfig::seeded(1).with_count("Folder", 7);
    config.scale = 2.0;
    let store = EntityStore::new(Arc::new(graph), config);

    assert_eq!(store.count("User"), DEFAULT_SEED_COUNT * 2);
    assert_eq!(
        store.count("Folder"),
        7,
        "a count the caller stated is what the caller said"
    );
}

#[test]
fn a_flat_default_still_overrides_the_graph() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("User"));
    graph.insert(entity("Folder").with_field(relation_field("owner", "User", Cardinality::One)));

    let mut config = StoreConfig::seeded(1);
    config.default_count = Some(5);
    let store = EntityStore::new(Arc::new(graph), config);

    assert_eq!(store.count("User"), 5);
    assert_eq!(store.count("Folder"), 5);
}

#[test]
fn a_cycle_does_not_make_a_world_the_census_cannot_build() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("A").with_field(relation_field("b", "B", Cardinality::One)));
    graph.insert(entity("B").with_field(relation_field("a", "A", Cardinality::One)));
    let store = EntityStore::new(Arc::new(graph), StoreConfig::seeded(1));

    for name in ["A", "B"] {
        assert!(store.count(name) >= DEFAULT_SEED_COUNT);
        assert!(store.count(name) <= MAX_SEED_COUNT);
    }
}

/// The partition is what makes counting arithmetic, and it was also why every
/// parent's children came out as exactly one run of the default order — a
/// deterministic identity, not a statistic, visible in one response.
#[test]
fn a_parents_children_are_scattered_through_the_census() {
    let store = counted_store(3, 10, 200);
    let folders = store.keys("Folder");
    let files = store.keys("File");
    let position = |key: &EntityKey| files.iter().position(|held| held == key);

    let mut spread = 0;
    for folder in &folders {
        let held = store
            .related("Folder", folder, "items", &Selection::new())
            .unwrap();
        if held.records.len() < 4 {
            continue;
        }
        let mut at: Vec<usize> = held
            .records
            .iter()
            .filter_map(|r| position(&r.key))
            .collect();
        at.sort_unstable();
        let runs = at
            .windows(2)
            .filter(|pair| pair.first().map(|s| s + 1) != pair.get(1).copied())
            .count()
            + 1;
        assert!(
            runs > 1,
            "the children of one folder sit side by side in the census: {at:?}"
        );
        spread += 1;
    }
    assert!(spread > 0, "the fixture should give some folder children");
}

/// Ordering is still total and still census order, so a page of a collection
/// reads the way the entity's own list does.
#[test]
fn a_collection_is_read_in_the_order_the_entity_lists_in() {
    let store = counted_store(6, 8, 120);
    let files = store.keys("File");
    for folder in store.keys("Folder") {
        let held = store
            .related("Folder", &folder, "items", &Selection::new())
            .unwrap();
        let at: Vec<usize> = held
            .records
            .iter()
            .filter_map(|record| files.iter().position(|key| key == &record.key))
            .collect();
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]), "{at:?}");
    }
}

#[test]
fn scattering_is_a_bijection_over_the_census() {
    for count in [0_u32, 1, 2, 7, 64, 257] {
        let scatter = Scatter::of(9, "File", "Folder", "folder", count);
        let mut landed = vec![false; count as usize];
        for position in 0..count {
            let index = scatter.index_at(position).expect("every position lands");
            assert_eq!(scatter.position_of(index), Some(position));
            let slot = landed.get_mut(index as usize).expect("inside the census");
            assert!(!*slot, "{count}: two positions landed on {index}");
            *slot = true;
        }
        assert!(landed.iter().all(|hit| *hit));
    }
}

/// The bus has to settle after the store writes the key, or the link a record
/// carries ends in an id it was never filed under.
#[test]
fn a_record_agrees_with_itself() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Person")
            .with_field(scalar_field("first_name", ScalarKind::String))
            .with_field(scalar_field("last_name", ScalarKind::String))
            .with_field(scalar_field("full_name", ScalarKind::String))
            .with_field(scalar_field("initials", ScalarKind::String))
            .with_field(scalar_field("username", ScalarKind::String))
            .with_field(scalar_field("email", ScalarKind::String))
            .with_field(scalar_field("avatar_url", ScalarKind::String))
            .with_field(scalar_field("title", ScalarKind::String))
            .with_field(scalar_field("slug", ScalarKind::String)),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(3).with_count("Person", 60),
    );

    for key in store.keys("Person") {
        let record = store.get("Person", &key).unwrap();
        let text = |name: &str| {
            record
                .get(name)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        };

        assert_eq!(
            text("full_name"),
            format!("{} {}", text("first_name"), text("last_name"))
        );
        assert!(text("email").starts_with(text("username")));
        assert!(
            text("avatar_url").ends_with(&key.to_string()),
            "`{}` does not end in the id it is filed under `{key}`",
            text("avatar_url")
        );
        assert!(!text("slug").is_empty());
        assert_eq!(
            text("slug"),
            text("title")
                .chars()
                .map(|c| if c.is_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                })
                .collect::<String>()
                .split('-')
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("-")
        );
    }
}

/// What a client wrote stands, even where it disagrees with the rest of the
/// record.
#[test]
fn a_created_record_keeps_the_values_the_caller_sent() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Person")
            .with_field(scalar_field("first_name", ScalarKind::String))
            .with_field(scalar_field("last_name", ScalarKind::String))
            .with_field(scalar_field("full_name", ScalarKind::String)),
    );
    let store = EntityStore::new(Arc::new(graph), StoreConfig::seeded(3));

    let Written::Created(made) = store
        .apply(
            "Person",
            Mutation::Insert {
                values: serde_json::json!({ "full_name": "Whatever I Said" }),
            },
        )
        .unwrap()
    else {
        panic!("an insert answers with the record")
    };
    assert_eq!(
        made.get("full_name").and_then(|v| v.as_str()),
        Some("Whatever I Said")
    );
    assert_eq!(store.get("Person", &made.key).unwrap().fields, made.fields);
}

fn ordered_store(seed: u64, orders: usize) -> EntityStore {
    use crate::core::world::model::{Lifecycle, LifecycleState};

    let state = |name: &str, weight: f64, empty: &[&str]| LifecycleState {
        name: name.into(),
        weight,
        empty: empty.iter().map(|held| (*held).into()).collect(),
    };
    let lifecycle = Lifecycle {
        states: vec![
            state("draft", 5.0, &["paid_at", "shipped_at", "delivered_at"]),
            state("paid", 40.0, &["shipped_at", "delivered_at"]),
            state("shipped", 35.0, &["delivered_at"]),
            state("delivered", 20.0, &[]),
        ],
    };

    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Order")
            .with_field(FieldDef::new(
                "status",
                ValueSpec::Lifecycle(Box::new(lifecycle)),
                false,
            ))
            .with_field(timestamp_field("paid_at"))
            .with_field(timestamp_field("shipped_at"))
            .with_field(timestamp_field("delivered_at")),
    );
    EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed).with_count("Order", orders),
    )
}

fn timestamp_field(name: &str) -> FieldDef {
    let mut inner = Scalar::new(ScalarKind::String);
    inner.semantic = Some(crate::type_detector::FieldType::Timestamp {
        format: crate::type_detector::TimestampFormat::Rfc3339Utc,
    });
    FieldDef::new(name, ValueSpec::Scalar(inner), true)
}

/// `shipped` *means* `shipped_at` holds a value and `delivered_at` does not.
/// That is an implication, not a correlation: no latent produces it, because a
/// latent gives a probability where the schema needs a certainty.
#[test]
fn a_state_decides_what_the_rest_of_the_record_can_hold() {
    let store = ordered_store(3, 300);
    let mut seen: Vec<String> = Vec::new();

    for key in store.keys("Order") {
        let record = store.get("Order", &key).unwrap();
        let status = record.get("status").unwrap().as_str().unwrap().to_string();
        let filled = |name: &str| record.get(name).is_some_and(|held| !held.is_null());

        match status.as_str() {
            "draft" => {
                assert!(!filled("paid_at") && !filled("shipped_at") && !filled("delivered_at"));
            }
            "paid" => assert!(!filled("shipped_at") && !filled("delivered_at")),
            "shipped" => assert!(!filled("delivered_at")),
            "delivered" => {}
            other => panic!("`{other}` is not a state of this lifecycle"),
        }
        seen.push(status);
    }

    for state in ["draft", "paid", "shipped", "delivered"] {
        assert!(
            seen.iter().any(|held| held == state),
            "`{state}` never came up"
        );
    }
    let drafts = seen.iter().filter(|held| *held == "draft").count();
    let paid = seen.iter().filter(|held| *held == "paid").count();
    assert!(
        paid > drafts * 3,
        "the weights are the caller's: {paid} paid against {drafts} draft"
    );
}

/// A delivered order cannot return to draft. A service that let it would be
/// broken, and answering the way the real one does is the point of declaring
/// the lifecycle.
#[test]
fn a_record_cannot_move_backwards_through_its_own_lifecycle() {
    let store = ordered_store(3, 300);
    let delivered = store
        .keys("Order")
        .into_iter()
        .find(|key| {
            store
                .get("Order", key)
                .and_then(|record| {
                    record
                        .get("status")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                })
                .as_deref()
                == Some("delivered")
        })
        .expect("a delivered order");

    let refused = store.apply(
        "Order",
        Mutation::Patch {
            key: delivered.clone(),
            values: serde_json::json!({ "status": "draft" }),
        },
    );
    assert!(
        matches!(refused, Err(crate::FerrimockError::Conflict(_))),
        "{refused:?}"
    );

    let unknown = store.apply(
        "Order",
        Mutation::Patch {
            key: delivered,
            values: serde_json::json!({ "status": "incinerated" }),
        },
    );
    assert!(matches!(unknown, Err(crate::FerrimockError::Conflict(_))));

    let draft = store
        .keys("Order")
        .into_iter()
        .find(|key| {
            store
                .get("Order", key)
                .and_then(|record| {
                    record
                        .get("status")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                })
                .as_deref()
                == Some("draft")
        })
        .expect("a draft order");
    assert!(
        store
            .apply(
                "Order",
                Mutation::Patch {
                    key: draft,
                    values: serde_json::json!({ "status": "shipped" }),
                },
            )
            .is_ok(),
        "moving forward is what a lifecycle is for"
    );
}

#[test]
fn a_written_state_still_decides_what_the_record_holds() {
    let store = ordered_store(3, 300);
    let draft = store
        .keys("Order")
        .into_iter()
        .find(|key| {
            store
                .get("Order", key)
                .and_then(|record| {
                    record
                        .get("status")
                        .and_then(|s| s.as_str())
                        .map(str::to_string)
                })
                .as_deref()
                == Some("draft")
        })
        .expect("a draft order");

    store
        .apply(
            "Order",
            Mutation::Patch {
                key: draft.clone(),
                values: serde_json::json!({ "status": "paid" }),
            },
        )
        .unwrap();

    let record = store.get("Order", &draft).unwrap();
    assert_eq!(record.get("status").unwrap().as_str(), Some("paid"));
}

fn placed_field(name: &str, field_type: crate::type_detector::FieldType) -> FieldDef {
    let mut inner = Scalar::new(ScalarKind::String);
    inner.semantic = Some(field_type);
    FieldDef::new(name, ValueSpec::Scalar(inner), false)
}

/// Fields inside a record were mutually independent, so a user in Tokyo got a
/// French name, a `+44` phone and an `America/Bogota` timezone. None of those
/// is individually implausible and the combination is impossible.
#[test]
fn a_record_is_somewhere_rather_than_nowhere() {
    use crate::type_detector::FieldType;

    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Account")
            .with_field(placed_field("holder", FieldType::Name))
            .with_field(placed_field("phone", FieldType::PhoneNumber))
            .with_field(placed_field("country", FieldType::CountryCode))
            .with_field(placed_field("currency", FieldType::CurrencyCode))
            .with_field(placed_field("timezone", FieldType::Timezone))
            .with_field(placed_field("locale", FieldType::LocaleCode))
            .with_field(placed_field("postcode", FieldType::PostalCode)),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(4).with_count("Account", 200),
    );

    let mut countries: Vec<String> = Vec::new();
    for key in store.keys("Account") {
        let record = store.get("Account", &key).unwrap();
        let text = |name: &str| {
            record
                .get(name)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        };

        let place = crate::fake_data::places()
            .iter()
            .find(|place| place.country_code == text("country"))
            .unwrap_or_else(|| panic!("`{}` is not a country", text("country")));

        assert_eq!(text("currency"), place.currency);
        assert_eq!(text("timezone"), place.timezone);
        assert_eq!(text("locale"), place.locale);
        assert!(
            text("phone").starts_with(place.calling_code),
            "{}",
            text("phone")
        );
        let family = text("holder").split(' ').next_back().unwrap_or_default();
        assert!(
            place.family.contains(&family),
            "`{}` is not a name from {}",
            text("holder"),
            place.country
        );
        assert!(!text("postcode").is_empty());
        countries.push(text("country").to_string());
    }

    countries.sort_unstable();
    countries.dedup();
    assert!(countries.len() > 4, "the world should not be one country");
}

/// One hop, along the derived path only: a folder's files are in the same
/// place the folder is, and the folder's own place is its own draw rather than
/// its parent's — a chain would put every record in the world in one country.
#[test]
fn a_child_is_where_its_parent_is() {
    use crate::type_detector::FieldType;

    let mut graph = EntityGraph::new();
    graph.insert(entity("Office").with_field(placed_field("country", FieldType::CountryCode)));
    graph.insert(
        entity("Worker")
            .with_field(placed_field("country", FieldType::CountryCode))
            .with_field(relation_field("office", "Office", Cardinality::One)),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(6)
            .with_count("Office", 12)
            .with_count("Worker", 200),
    );
    let office_relation = relation_of(&store, "Worker", "office");

    let mut agreed = 0;
    for key in store.keys("Worker") {
        let worker = store.get("Worker", &key).unwrap();
        let office = store
            .relation_target("Worker", &key, "office", office_relation)
            .expect("every worker has an office");
        assert_eq!(
            worker.get("country"),
            office.get("country"),
            "a worker is where the office is"
        );
        agreed += 1;
    }
    assert!(agreed > 0);
}
