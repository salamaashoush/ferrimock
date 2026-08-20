#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use super::*;
use crate::core::world::model::{
    Cardinality, Carrier, CompositeKey, Confidence, EntityGraph, EntityType, FieldDef, Provenance,
    Relation, Rule, Scalar, ScalarKind, ValueSpec,
};
use crate::core::world::store::StoreConfig;
use crate::type_detector::{FieldType, TimestampFormat};

fn scalar(name: &str, kind: ScalarKind) -> FieldDef {
    FieldDef::new(name, ValueSpec::Scalar(Scalar::new(kind)), false)
}

fn optional(name: &str, kind: ScalarKind) -> FieldDef {
    FieldDef::new(name, ValueSpec::Scalar(Scalar::new(kind)), true)
}

fn semantic(name: &str, field_type: FieldType) -> FieldDef {
    let mut inner = Scalar::new(ScalarKind::String);
    inner.semantic = Some(field_type);
    FieldDef::new(name, ValueSpec::Scalar(inner), false)
}

fn enumeration(name: &str, members: &[&str]) -> FieldDef {
    FieldDef::new(
        name,
        ValueSpec::Enum(members.iter().map(|m| (*m).into()).collect()),
        false,
    )
}

fn list_of(name: &str, kind: ScalarKind) -> FieldDef {
    FieldDef::new(
        name,
        ValueSpec::List(Box::new(ValueSpec::Scalar(Scalar::new(kind)))),
        false,
    )
}

fn link(name: &str, target: &str, cardinality: Cardinality) -> FieldDef {
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
        CompositeKey::single("id"),
        Provenance::new(Rule::GraphQLSchema, name),
    )
    .with_field(scalar("id", ScalarKind::Id))
}

/// A world wide enough that every check has something to measure.
fn wide_world(seed: u64, count: usize) -> EntityStore {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Article")
            .with_field(scalar("title", ScalarKind::String))
            .with_field(scalar("views", ScalarKind::Int))
            .with_field(scalar("featured", ScalarKind::Boolean))
            .with_field(optional("subtitle", ScalarKind::String))
            .with_field(enumeration("status", &["draft", "review", "live"]))
            .with_field(list_of("tags", ScalarKind::String))
            .with_field(semantic(
                "created_at",
                FieldType::Timestamp {
                    format: TimestampFormat::Rfc3339Utc,
                },
            )),
    );
    EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed).with_count("Article", count),
    )
}

fn failed(report: &Report, check: Check) -> Vec<&Finding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.check == check)
        .collect()
}

fn skipped(report: &Report, check: Check) -> Vec<&Unmeasured> {
    report
        .unmeasured
        .iter()
        .filter(|item| item.check == check)
        .collect()
}

#[test]
fn the_doctor_names_the_tells_the_generators_leave() {
    let report = examine(&wide_world(3, 400));

    for check in [
        Check::NumberSupport,
        Check::ConstantListLength,
        Check::FairCoin,
        Check::UniformEnum,
        Check::SmallVocabulary,
        Check::IdTimeOrder,
    ] {
        assert!(
            !failed(&report, check).is_empty(),
            "`{}` should fire on the world as it stands: {}",
            check.name(),
            check.tell()
        );
    }
}

/// Closed tells: dates run to the end of the month, the window follows the
/// wall clock instead of a constant that went stale, and a field the schema
/// said may be missing sometimes is.
#[test]
fn the_calendar_no_longer_gives_the_world_away() {
    let report = examine(&wide_world(3, 400));
    for check in [Check::DayOfMonth, Check::StaleClock, Check::NeverAbsent] {
        assert!(
            failed(&report, check).is_empty(),
            "`{}` should not fire: {:?}",
            check.name(),
            failed(&report, check)
        );
    }
}

/// A check with nothing to measure is its own outcome. Reporting it as a pass
/// would say the opposite of what is true, and it is the number a bigger world
/// has to move before any distribution work can be tested at all.
#[test]
fn a_check_the_world_is_too_small_for_is_not_a_pass() {
    let report = examine(&wide_world(3, 8));

    assert!(failed(&report, Check::UniformEnum).is_empty());
    assert!(
        !skipped(&report, Check::UniformEnum).is_empty(),
        "a three-member enum needs fifteen draws and the world holds eight"
    );
    assert!(
        !failed(&report, Check::WorldSize).is_empty(),
        "eight records fit inside one default page"
    );
}

#[test]
fn a_world_larger_than_a_page_stops_reporting_its_size() {
    let report = examine(&wide_world(3, 400));
    assert!(failed(&report, Check::WorldSize).is_empty());
}

/// The two acceptance checks. Both directions of every relation agree and every
/// count field matches the collection it names, so neither may ever fire.
#[test]
fn nothing_in_a_seeded_world_disagrees_with_itself() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("User")
            .with_field(scalar("post_count", ScalarKind::Int))
            .with_field(link("posts", "Post", Cardinality::Many)),
    );
    graph.insert(
        entity("Post")
            .with_field(link("author", "User", Cardinality::One))
            .with_field(link("liked_by", "User", Cardinality::Many)),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(4)
            .with_count("User", 30)
            .with_count("Post", 120),
    );

    let report = examine(&store);
    assert!(
        failed(&report, Check::RelationDisagreement).is_empty(),
        "{:?}",
        failed(&report, Check::RelationDisagreement)
    );
    assert!(
        failed(&report, Check::CountDisagreement).is_empty(),
        "{:?}",
        failed(&report, Check::CountDisagreement)
    );
}

/// A levelled hierarchy has no fixed point to find, at any seed or size.
#[test]
fn a_hierarchy_reports_nothing_that_parents_itself() {
    for seed in 0..24 {
        let mut graph = EntityGraph::new();
        graph.insert(
            entity("Folder")
                .with_field(link("parent", "Folder", Cardinality::One))
                .with_field(link("children", "Folder", Cardinality::Many)),
        );
        let store = EntityStore::new(
            Arc::new(graph),
            StoreConfig::seeded(seed).with_count("Folder", 60),
        );

        let report = examine(&store);
        assert!(
            failed(&report, Check::SelfParent).is_empty(),
            "seed {seed}: {:?}",
            failed(&report, Check::SelfParent)
        );
        assert_eq!(report.broken(), 0, "seed {seed}");
    }
}

/// The partition files a parent's children side by side, so the number of runs
/// down an unsorted page equals the number of parents on it exactly.
#[test]
fn children_arrive_in_one_run_per_parent() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Shelf").with_field(link("books", "Book", Cardinality::Many)));
    graph.insert(entity("Book").with_field(link("shelf", "Shelf", Cardinality::One)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(2)
            .with_count("Shelf", 10)
            .with_count("Book", 120),
    );

    let report = examine(&store);
    let found = failed(&report, Check::ContiguousChildren);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].subject, "Shelf.books");
}

#[test]
fn a_many_to_many_reports_its_two_bar_degree_histogram() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Doc").with_field(link("collections", "Collection", Cardinality::Many)));
    graph.insert(entity("Collection").with_field(link("items", "Doc", Cardinality::Many)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(9)
            .with_count("Doc", 60)
            .with_count("Collection", 12),
    );

    let report = examine(&store);
    let found = failed(&report, Check::MembershipDegree);
    assert!(
        found.iter().any(|f| f.subject == "Doc.collections"),
        "{found:?}"
    );
}

#[test]
fn every_check_reports_a_number_that_can_move() {
    let report = examine(&wide_world(3, 400));
    assert!(!report.findings.is_empty());
    for finding in &report.findings {
        assert!(
            finding.measured.chars().any(|c| c.is_ascii_digit()),
            "`{}` on {} says `{}` and names no measurement",
            finding.check.name(),
            finding.subject,
            finding.measured
        );
    }
    assert!(report.sampled >= 400);
}

#[test]
fn a_date_is_read_out_of_whatever_format_wrote_it() {
    assert_eq!(day_of_month("2024-03-17T05:00:00Z"), Some(17));
    assert_eq!(day_of_month("17/03/2024"), Some(17));
    assert_eq!(day_of_month("17.03.2024"), Some(17));
    assert_eq!(day_of_month("20240317"), Some(17));
    assert_eq!(day_of_month("Tue, 17 Mar 2024 05:00:00 GMT"), Some(17));
    assert_eq!(day_of_month("not a date at all"), None);

    assert_eq!(year_of("2024-03-17T05:00:00Z"), Some(2024));
    assert_eq!(year_of("17/03/2024"), Some(2024));
    assert_eq!(year_of("Tue, 17 Mar 2024 05:00:00 GMT"), Some(2024));
}

#[test]
fn concordance_separates_an_order_from_a_shuffle() {
    assert!((concordance(&[1, 2, 3, 4, 5]) - 1.0).abs() < f64::EPSILON);
    assert!((concordance(&[5, 4, 3, 2, 1]) + 1.0).abs() < f64::EPSILON);
    assert!(concordance(&[3, 1, 4, 1, 5, 9, 2, 6]).abs() < 0.5);
}

#[test]
fn the_chi_square_cutoff_matches_the_published_table() {
    for (df, expected) in [(1usize, 3.841), (2, 5.991), (4, 9.488), (9, 16.919)] {
        let computed = chi_square_critical(df);
        assert!(
            (computed - expected).abs() < 0.05,
            "df {df}: {computed} against {expected}"
        );
    }
}
