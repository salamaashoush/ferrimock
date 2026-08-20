#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_precision_loss
)]

use chrono::{Datelike, Timelike};

use super::*;

fn moments(seed: u64, entity: &str, count: u64) -> Vec<i64> {
    (0..count).map(|i| moment_of(seed, entity, i)).collect()
}

/// The window is an output. Nothing that answers for one record may consult
/// how many records there are, or bumping `world.counts` rewrites the creation
/// time of every record that already existed — silently, because a delta
/// conflict is only raised when an ordinal disappears.
#[test]
fn an_arrival_does_not_depend_on_how_many_arrivals_there_are() {
    let few = moments(4, "Folder", 12);
    let many = moments(4, "Folder", 400);
    assert_eq!(few, many[..12]);
}

/// Both ends are anchored without consulting the count: the oldest at the
/// start of the entity's history, the newest near the present.
#[test]
fn the_newest_record_is_recent_whatever_the_world_holds() {
    for count in [12_u64, 40, 600] {
        for seed in 0..8 {
            let drawn = moments(seed, "Folder", count);
            let newest = drawn.iter().copied().max().unwrap();
            let age_days = (now() - newest) as f64 / 86_400.0;
            let allowed = if count < 40 { 170.0 } else { 60.0 };
            assert!(
                (0.0..allowed).contains(&age_days),
                "seed {seed}, {count} records: newest is {age_days:.1} days old"
            );
        }
    }
}

#[test]
fn a_bigger_world_is_denser_rather_than_older() {
    let oldest = |held: &[i64]| now() - held.iter().copied().min().unwrap();
    let small = moments(2, "Folder", 20);
    let large = moments(2, "Folder", 600);
    assert_eq!(
        oldest(&small),
        oldest(&large),
        "the start of an entity's history does not move when it gains records"
    );

    let newest = |held: &[i64]| now() - held.iter().copied().max().unwrap();
    assert!(newest(&large) < newest(&small));
}

/// A record's ordinal, its key and its age all rise together, which is the
/// only thing that lets a sequential id agree with a creation time.
#[test]
fn a_later_ordinal_is_a_later_record() {
    for seed in 0..8 {
        let drawn = moments(seed, "Folder", 400);
        let out_of_order = drawn.windows(2).filter(|pair| pair[1] < pair[0]).count();
        assert_eq!(
            out_of_order, 0,
            "seed {seed}: the warp has to be monotone or an id stops agreeing \
             with the time beside it"
        );
    }
}

#[test]
fn nothing_was_created_in_the_future() {
    for seed in 0..8 {
        for moment in moments(seed, "File", 200) {
            assert!(moment <= now(), "seed {seed}");
        }
    }
}

/// A histogram of any real collection has a weekly shape and a daily one.
///
/// Sampled across many entities rather than down one: arrivals close on the
/// present, so the deep end of a single entity crowds into a few days and
/// would not cover a week at all.
#[test]
fn arrivals_cluster_into_working_days_and_working_hours() {
    let drawn: Vec<chrono::DateTime<chrono::Utc>> = (0..120)
        .flat_map(|entity| moments(5, &format!("Entity{entity}"), 40))
        .filter_map(|moment| chrono::DateTime::from_timestamp(moment, 0))
        .collect();

    let weekend = drawn
        .iter()
        .filter(|at| at.weekday().number_from_monday() > 5)
        .count() as f64
        / drawn.len() as f64;
    assert!(
        (0.01..0.15).contains(&weekend),
        "a flat clock puts two sevenths of a week at the weekend: {weekend}"
    );

    let overnight = drawn.iter().filter(|at| at.hour() < 6).count() as f64 / drawn.len() as f64;
    assert!(
        overnight < 0.05,
        "a flat clock puts a quarter of a day before six: {overnight}"
    );

    let midday = drawn
        .iter()
        .filter(|at| (11..16).contains(&at.hour()))
        .count() as f64
        / drawn.len() as f64;
    assert!(
        midday > 0.30,
        "the middle of the day should be busiest: {midday}"
    );
}

#[test]
fn two_entities_do_not_share_one_history() {
    let folders = moments(3, "Folder", 200);
    let files = moments(3, "File", 200);
    let span =
        |held: &[i64]| held.iter().copied().max().unwrap() - held.iter().copied().min().unwrap();
    assert_ne!(span(&folders), span(&files));
}

#[test]
fn a_field_waits_after_the_record_it_belongs_to_and_never_past_now() {
    let arrived = now() - 400 * 86_400;
    for i in 0..2000 {
        let derived = crate::fake_data::rng::derive_seed(1, "field", i);
        let at = field_moment(arrived, derived, 1);
        assert_eq!(
            field_moment(arrived, derived, 0),
            arrived,
            "the field naming the opening is the arrival itself"
        );
        assert!(at >= arrived, "a field moved before the record existed");
        assert!(at <= now(), "a field moved into the future");
    }
}
