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
use crate::fake_data::distribution::FLATTEST_FLAG;
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

/// The measure itself, on two corpora, because that is the claim: it has to
/// separate text drawn from a handful of words from text that is merely long.
/// The default generators no longer leave this tell, which is the point — but
/// a check that cannot fail is not a check.
#[test]
fn a_closed_vocabulary_is_named_and_an_open_one_is_not() {
    fn measured(strings: Vec<String>) -> Report {
        let mut report = Report::default();
        let stats = FieldStats {
            strings,
            ..FieldStats::default()
        };
        check_vocabulary(&stats, "Note.body", &mut report);
        report
    }

    let words = ["alpha", "beta", "gamma"];
    let closed: Vec<String> = (0..200)
        .map(|i| {
            format!(
                "{} {} {}",
                words[i % 3],
                words[(i / 3) % 3],
                words[(i / 9) % 3]
            )
        })
        .collect();
    let report = measured(closed);
    assert_eq!(
        failed(&report, Check::SmallVocabulary).len(),
        1,
        "three words over six hundred draws is a closed vocabulary"
    );

    // Composed the way the generators compose: a phrase per record, drawn from
    // pools deep enough that a hundred words do not exhaust them.
    let open: Vec<String> = (0..200)
        .map(|i| {
            format!(
                "{} {} {}",
                crate::fake_data::fake_headline(),
                crate::fake_data::fake_label(),
                i
            )
        })
        .collect();
    let report = measured(open);
    assert!(
        failed(&report, Check::SmallVocabulary).is_empty(),
        "composed prose reads as closed: {:?}",
        failed(&report, Check::SmallVocabulary)
            .iter()
            .map(|f| f.measured.clone())
            .collect::<Vec<_>>()
    );
}

/// A key is written on every record whatever the schema said, so it is not a
/// field that is never absent — it is a field that cannot be. Every other
/// optional field still is.
#[test]
fn a_key_is_not_reported_for_always_being_there() {
    fn always_present(entity: &EntityType, field: &FieldDef) -> Report {
        let mut report = Report::default();
        let stats = FieldStats {
            present: 200,
            ..FieldStats::default()
        };
        check_nullability(entity, field, &stats, "Doc.field", &mut report);
        report
    }

    let key = optional("id", ScalarKind::Id);
    let ordinary = optional("subtitle", ScalarKind::String);
    let doc = entity("Doc")
        .with_field(key.clone())
        .with_field(ordinary.clone());

    assert!(
        failed(&always_present(&doc, &key), Check::NeverAbsent).is_empty(),
        "the key it is addressed by has to be on every record"
    );
    assert_eq!(
        failed(&always_present(&doc, &ordinary), Check::NeverAbsent).len(),
        1,
        "an optional field that is always there is still a tell"
    );
}

/// Sorting by id orders by *creation*. Every other instant on a record moves
/// independently of it — an update rewrites a timestamp and leaves the id
/// alone — so the check reads the creation field or none at all.
#[test]
fn id_order_is_judged_against_creation_and_nothing_else() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Article")
            .with_field(semantic(
                "created_at",
                FieldType::Timestamp {
                    format: TimestampFormat::Rfc3339Utc,
                },
            ))
            .with_field(semantic(
                "updated_at",
                FieldType::Timestamp {
                    format: TimestampFormat::Rfc3339Utc,
                },
            )),
    );
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(3).with_count("Article", 400),
    );
    let report = examine(&store);

    assert!(
        failed(&report, Check::IdTimeOrder).is_empty(),
        "ids order by creation: {:?}",
        failed(&report, Check::IdTimeOrder)
    );
    // And it read the creation field rather than settling on whichever
    // timestamp came first — whether it passed, failed or could not measure.
    let judged: Vec<&str> = report
        .findings
        .iter()
        .filter(|f| f.check == Check::IdTimeOrder)
        .map(|f| f.subject.as_str())
        .chain(
            report
                .unmeasured
                .iter()
                .filter(|u| u.check == Check::IdTimeOrder)
                .map(|u| u.subject.as_str()),
        )
        .collect();
    assert!(
        !judged.iter().any(|subject| subject.contains("updated_at")),
        "`updated_at` is not what an id is expected to track: {judged:?}"
    );
}

/// Closed tells. Each of these read as a flat line, a constant, or a support
/// that stopped at a round number, and each is now a distribution.
#[test]
fn the_calendar_no_longer_gives_the_world_away() {
    let report = examine(&wide_world(3, 400));
    for check in [
        Check::DayOfMonth,
        Check::StaleClock,
        Check::NeverAbsent,
        Check::NumberSupport,
        Check::ConstantListLength,
        Check::FairCoin,
        Check::UniformEnum,
        Check::IdTimeOrder,
    ] {
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
        "a three-member enum needs far more than the eight draws this holds"
    );
    assert!(
        !failed(&report, Check::WorldSize).is_empty(),
        "eight records fit inside one default page"
    );
}

/// The distribution checks read a world of their own.
///
/// Forty records is what a mount serves, not what a check needs, and a lint
/// has no reason to borrow that number: the generator draws the same shape
/// whatever the census size, so the checks that measure a shape are given a
/// census wide enough to see one. What the mount asked for is still what the
/// size check reports.
#[test]
fn a_lint_reads_a_wider_world_than_the_one_it_is_handed() {
    let mut graph = EntityGraph::new();
    graph.insert(
        entity("Article")
            .with_field(scalar("featured", ScalarKind::Boolean))
            .with_field(enumeration("status", &["draft", "review", "live"])),
    );
    let graph = Arc::new(graph);

    // A world the caller sized by hand is left alone: it is a fact about a
    // world someone wanted.
    let stated = EntityStore::new(
        Arc::clone(&graph),
        StoreConfig::seeded(3).with_count("Article", 8),
    );
    let report = examine(&stated);
    assert_eq!(report.sampled, 8);
    assert!(
        !skipped(&report, Check::FairCoin).is_empty(),
        "eight records is what was asked for, and it is not enough to measure"
    );

    // A world that merely defaulted to its size is not.
    let defaulted = EntityStore::new(Arc::clone(&graph), StoreConfig::seeded(3));
    let report = examine(&defaulted);
    assert!(
        report.sampled > defaulted.count("Article"),
        "the value checks read more than the {} record(s) served",
        defaulted.count("Article")
    );
    assert!(
        skipped(&report, Check::FairCoin).is_empty()
            && skipped(&report, Check::UniformEnum).is_empty(),
        "a defaulted world is widened until both can be measured: {:?}",
        report.unmeasured
    );
    // And the size of the world as served is still the size of the world as
    // served.
    assert_eq!(
        failed(&report, Check::WorldSize).len(),
        usize::from(defaulted.count("Article") <= DEFAULT_PAGE_SIZE)
    );
}

/// A state no move leads to is a typo or a forgotten edge, and no amount of
/// traffic will ever produce it — so it is the declaration that is wrong, which
/// makes it a defect rather than a tell.
///
/// Only a machine that draws its edges can be wrong this way. An ordering
/// reaches every state by construction, which is the expressiveness it trades
/// away, so it must never be reported.
#[test]
fn a_state_no_move_reaches_is_a_defect_and_an_ordering_never_is() {
    use crate::core::machine::{Edge, Machine, State as MachineState};

    fn state(name: &str, on: &[(&str, &str)]) -> MachineState {
        MachineState {
            name: name.into(),
            weight: 1.0,
            empty: Vec::new(),
            on: on
                .iter()
                .map(|(event, target)| Edge {
                    event: (*event).into(),
                    target: (*target).into(),
                    guard: None,
                })
                .collect(),
            after: Vec::new(),
        }
    }

    fn measured(machine: Machine) -> Report {
        let mut report = Report::default();
        let field = FieldDef::new("status", ValueSpec::Lifecycle(Box::new(machine)), false);
        check_reachable(&field, "Order.status", &mut report);
        report
    }

    // `refunded` is declared and nothing moves into it.
    let stranded = measured(Machine::new(vec![
        state("created", &[("pay", "paid")]),
        state("paid", &[("ship", "shipped")]),
        state("shipped", &[]),
        state("refunded", &[]),
    ]));
    assert_eq!(failed(&stranded, Check::UnreachableState).len(), 1);
    assert!(
        failed(&stranded, Check::UnreachableState)[0]
            .subject
            .contains("refunded"),
        "the finding names the state"
    );

    // Give it the edge it was missing and the finding goes away.
    let joined = measured(Machine::new(vec![
        state("created", &[("pay", "paid")]),
        state("paid", &[("ship", "shipped"), ("refund", "refunded")]),
        state("shipped", &[]),
        state("refunded", &[]),
    ]));
    assert!(failed(&joined, Check::UnreachableState).is_empty());

    // The same states as an ordering: every one of them is reachable by
    // position, so there is nothing to report.
    let ordered = measured(Machine::new(vec![
        state("created", &[]),
        state("paid", &[]),
        state("shipped", &[]),
        state("refunded", &[]),
    ]));
    assert!(
        failed(&ordered, Check::UnreachableState).is_empty(),
        "an ordering reaches everything: {:?}",
        failed(&ordered, Check::UnreachableState)
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

/// Ownership is contiguous in partition position; where an instance sits in
/// the census is a shuffle of that. So a page of children no longer reads as
/// one run per parent — the identity that made the partition visible in a
/// single response.
#[test]
fn children_do_not_arrive_in_one_run_per_parent() {
    for seed in 0..12 {
        let mut graph = EntityGraph::new();
        graph.insert(entity("Shelf").with_field(link("books", "Book", Cardinality::Many)));
        graph.insert(entity("Book").with_field(link("shelf", "Shelf", Cardinality::One)));
        let store = EntityStore::new(
            Arc::new(graph),
            StoreConfig::seeded(seed)
                .with_count("Shelf", 10)
                .with_count("Book", 120),
        );

        let report = examine(&store);
        assert!(
            failed(&report, Check::ContiguousChildren).is_empty(),
            "seed {seed}: {:?}",
            failed(&report, Check::ContiguousChildren)
        );
    }
}

/// Both sides of a many-to-many have a tail now: the degree is drawn rather
/// than fixed at one or two, and anchors are drawn by preference, so
/// inverting the attachment does not give every collection the same size.
#[test]
fn a_many_to_many_has_more_than_two_degrees_on_either_side() {
    let mut graph = EntityGraph::new();
    graph.insert(entity("Doc").with_field(link("collections", "Collection", Cardinality::Many)));
    graph.insert(entity("Collection").with_field(link("items", "Doc", Cardinality::Many)));
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(9)
            .with_count("Doc", 300)
            .with_count("Collection", 40),
    );

    let report = examine(&store);
    assert!(
        failed(&report, Check::MembershipDegree).is_empty(),
        "{:?}",
        failed(&report, Check::MembershipDegree)
    );

    let sizes = |entity: &str, field: &str| {
        let mut held: Vec<usize> = store
            .keys(entity)
            .into_iter()
            .filter_map(|key| {
                store
                    .related(entity, &key, field, &Selection::new())
                    .ok()
                    .map(|page| page.total)
            })
            .collect();
        held.sort_unstable();
        held
    };

    for (entity, field) in [("Doc", "collections"), ("Collection", "items")] {
        let held = sizes(entity, field);
        let largest = held.last().copied().unwrap_or(0);
        let middle = held.get(held.len() / 2).copied().unwrap_or(0);
        assert!(
            held.first() == Some(&0),
            "{entity}.{field}: nothing is in nothing"
        );
        assert!(
            largest > middle * 3,
            "{entity}.{field} has no tail: median {middle}, largest {largest}"
        );
    }
}

/// A finding has to say what it measured, or it cannot be argued with and it
/// cannot be watched for a regression.
///
/// Measured on a world small enough to trip several checks at once — the wide
/// one no longer trips any, which is the outcome the other tests assert.
#[test]
fn every_check_reports_a_number_that_can_move() {
    let report = examine(&wide_world(3, 12));
    assert!(
        !report.findings.is_empty(),
        "a twelve-record world trips at least the size check"
    );
    for finding in &report.findings {
        assert!(
            finding.measured.chars().any(|c| c.is_ascii_digit()),
            "`{}` on {} says `{}` and names no measurement",
            finding.check.name(),
            finding.subject,
            finding.measured
        );
    }

    // And a world large enough to measure is read in full rather than sampled
    // down to whatever the first check wanted.
    assert!(examine(&wide_world(3, 400)).sampled >= 400);
}

#[test]
fn a_date_is_read_out_of_whatever_format_wrote_it() {
    assert_eq!(calendar_of("2024-03-17T05:00:00Z"), Some((2024, 3, 17)));
    assert_eq!(calendar_of("17/03/2024"), Some((2024, 3, 17)));
    assert_eq!(calendar_of("17.03.2024"), Some((2024, 3, 17)));
    assert_eq!(calendar_of("20240317"), Some((2024, 3, 17)));
    assert_eq!(
        calendar_of("Tue, 17 Mar 2024 05:00:00 GMT"),
        Some((2024, 3, 17))
    );
    assert_eq!(calendar_of("not a date at all"), None);
    // The month is read as a name here, not as digits, so a value that is the
    // right shape and names no month is not a date.
    assert_eq!(calendar_of("Tue, 17 Zzz 2024 05:00:00 GMT"), None);

    assert_eq!(year_of("2024-03-17T05:00:00Z"), Some(2024));
    assert_eq!(year_of("17/03/2024"), Some(2024));
    assert_eq!(year_of("Tue, 17 Mar 2024 05:00:00 GMT"), Some(2024));
}

/// The day check counts distinct dates, not values, and asks for as many as
/// the window it is looking at can answer with.
///
/// A generator drawing `1..=28` is what it is for, and it has to keep saying
/// so. What it must not do is read a field whose values pile onto a handful of
/// days — which is what a recency-weighted arrival looks like — as a truncated
/// calendar because there were six hundred of them.
#[test]
fn a_truncated_calendar_is_named_and_a_repeated_one_is_not() {
    fn measured(dates: Vec<String>) -> Report {
        let mut report = Report::default();
        let stats = FieldStats {
            strings: dates,
            ..FieldStats::default()
        };
        check_day_of_month(&stats, "Doc.at", &mut report);
        report
    }

    fn written(year: i32, month: u32, day: u32) -> String {
        format!("{year:04}-{month:02}-{day:02}T00:00:00Z")
    }

    // A year of a day drawn out of `1..=28`. The window holds twenty-nine days
    // the draw can never reach, and never reaching one of them over three
    // hundred and thirty-six distinct dates is not a coincidence.
    let truncated: Vec<String> = (1..=12)
        .flat_map(|month| (1..=28).map(move |day| written(2024, month, day)))
        .collect();
    assert_eq!(
        failed(&measured(truncated), Check::DayOfMonth).len(),
        1,
        "a day drawn out of one to twenty-eight is the tell this check is for"
    );

    // The same window, the same six hundred values, sixteen distinct dates.
    // Nothing about sixteen draws says whether a 29th was reachable.
    let repeated: Vec<String> = (0..600)
        .map(|i: u32| written(2024, 3, i % 16 + 1))
        .collect();
    let report = measured(repeated);
    assert!(
        failed(&report, Check::DayOfMonth).is_empty(),
        "sixteen distinct dates cannot answer this, however often they repeat"
    );
    assert_eq!(skipped(&report, Check::DayOfMonth).len(), 1);

    // A window with no long-month day in it at all. February has no 29th to
    // find in 2023, so not finding one is the calendar.
    let february: Vec<String> = (1..=28).map(|day| written(2023, 2, day)).collect();
    assert!(
        failed(&measured(february), Check::DayOfMonth).is_empty(),
        "a February holds no day past the 28th to miss"
    );
}

/// The floors are what the *generator* needs to be seen, so they follow the
/// flattest thing it draws rather than its average one.
#[test]
fn a_sample_floor_follows_the_flattest_draw_the_generator_makes() {
    // A flat ranking spread over five members needs more draws than the same
    // ranking at the middle of `LEAST_SKEW..=MOST_SKEW`, and forty is short of
    // it — which is what three five-member enums were reported for.
    let flattest = draws_to_notice(&flattest_ranking(5));
    let middling: Vec<f64> = {
        let weight = |rank: usize| 1.0 / as_f64(rank + 1);
        let total: f64 = (0..5).map(weight).sum();
        (0..5).map(|rank| weight(rank) / total).collect()
    };
    assert!(
        flattest > draws_to_notice(&middling),
        "a flatter ranking takes more draws to separate from flat"
    );
    assert!(
        flattest > 40,
        "forty draws of a five-member enum is short of it: {flattest}"
    );

    // And the coin floor follows `lopsided_chance`, whose flattest draw is
    // further from the middle than a two-member ranking's is.
    let coin = draws_to_notice(&[FLATTEST_FLAG, 1.0 - FLATTEST_FLAG]);
    assert!(
        coin > draws_to_notice(&flattest_ranking(2)),
        "a 0.42 coin is flatter than a two-member ranking: {coin}"
    );
    assert!(
        coin > 120,
        "a 67-of-120 split is not evidence of an even one: {coin}"
    );
}

/// A boolean that really is a coin is still reported, once there is enough of
/// it to say so.
#[test]
fn an_even_split_is_named_once_there_are_enough_draws_to_see_one() {
    fn measured(trues: usize, falses: usize) -> Report {
        let mut report = Report::default();
        let stats = FieldStats {
            trues,
            falses,
            ..FieldStats::default()
        };
        check_boolean(&stats, "Doc.flag", &mut report);
        report
    }

    let enough = draws_to_notice(&[FLATTEST_FLAG, 1.0 - FLATTEST_FLAG]);
    let half = enough / 2 + 1;
    assert_eq!(
        failed(&measured(half, enough + 1 - half), Check::FairCoin).len(),
        1,
        "an even split over enough draws is the tell"
    );
    // The split the generator's flattest coin makes, over the same draws, is
    // not — it clears the threshold the floor was chosen to clear.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lopsided = (as_f64(enough + 1) * FLATTEST_FLAG).round() as usize;
    assert!(
        failed(&measured(lopsided, enough + 1 - lopsided), Check::FairCoin).is_empty(),
        "0.42 of enough draws is what the floor was sized to catch"
    );
    assert_eq!(
        skipped(&measured(half - 1, enough - half), Check::FairCoin).len(),
        1,
        "one draw short of the floor is unmeasurable, not even"
    );
}

/// A number that counts is not a number that was drawn, so the bound the draw
/// would have stopped at says nothing about it.
#[test]
fn a_numeric_key_is_not_reported_for_staying_inside_a_bound_it_never_used() {
    fn bounded(entity: &EntityType, field: &FieldDef) -> Report {
        let mut report = Report::default();
        let stats = FieldStats {
            numbers: (1..=40).map(f64::from).collect(),
            ..FieldStats::default()
        };
        check_numbers(entity, field, &stats, "Row.field", &mut report);
        report
    }

    let key = scalar("id", ScalarKind::Int);
    let ordinary = scalar("weight", ScalarKind::Int);
    let row = EntityType::new(
        "Row",
        CompositeKey::single("id"),
        Provenance::new(Rule::GraphQLSchema, "Row"),
    )
    .with_field(key.clone())
    .with_field(ordinary.clone());

    assert!(
        failed(&bounded(&row, &key), Check::NumberSupport).is_empty(),
        "a key numbers the census; its largest value is the world's size"
    );
    assert_eq!(
        failed(&bounded(&row, &ordinary), Check::NumberSupport).len(),
        1,
        "a drawn number that never leaves the default bound is still a tell"
    );
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
