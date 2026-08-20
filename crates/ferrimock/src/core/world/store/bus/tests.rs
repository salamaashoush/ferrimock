#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use super::*;
use crate::core::world::model::{Scalar, ScalarKind, ValueSpec};

fn field(name: &str) -> FieldDef {
    FieldDef::new(
        name,
        ValueSpec::Scalar(Scalar::new(ScalarKind::String)),
        false,
    )
}

fn record(pairs: &[(&str, &str)]) -> JsonMap<String, JsonValue> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), JsonValue::from(*value)))
        .collect()
}

fn wired(pairs: &[(&str, &str)], stated: &[String]) -> JsonMap<String, JsonValue> {
    let fields: Vec<FieldDef> = pairs.iter().map(|(name, _)| field(name)).collect();
    let mut held = record(pairs);
    wire(&fields, &mut held, stated);
    held
}

#[test]
fn a_full_name_is_its_own_parts() {
    let held = wired(
        &[
            ("first_name", "Ada"),
            ("last_name", "Lovelace"),
            ("full_name", "Marcus Whitfield"),
            ("initials", "QQ"),
        ],
        &[],
    );
    assert_eq!(held["full_name"], JsonValue::from("Ada Lovelace"));
    assert_eq!(held["initials"], JsonValue::from("AL"));
}

/// The local part is a function of the name; the domain the generator drew
/// carries the locale mix and stays.
#[test]
fn an_email_is_the_name_it_belongs_to() {
    let held = wired(
        &[
            ("first_name", "Ada"),
            ("last_name", "Lovelace"),
            ("full_name", "unused"),
            ("email", "perferendis@corbin-hills.example"),
        ],
        &[],
    );
    assert_eq!(
        held["email"],
        JsonValue::from("ada.lovelace@corbin-hills.example")
    );
}

#[test]
fn a_slug_is_the_title_it_belongs_to() {
    let held = wired(
        &[
            ("title", "Q4 Migration: Phase Two"),
            ("slug", "perferendis-non-adipisci"),
        ],
        &[],
    );
    assert_eq!(held["slug"], JsonValue::from("q4-migration-phase-two"));
}

#[test]
fn a_link_ends_in_the_record_it_belongs_to() {
    let held = wired(
        &[
            ("id", "0d5b1f8e"),
            ("avatar_url", "https://cdn.example.com/avatars/98d2c1"),
        ],
        &[],
    );
    assert_eq!(
        held["avatar_url"],
        JsonValue::from("https://cdn.example.com/avatars/0d5b1f8e")
    );
}

/// A name derived from parts has to be available to the handle derived from
/// it, or the address names a person the record does not have.
#[test]
fn a_value_derived_from_a_derived_value_reads_the_settled_one() {
    let held = wired(
        &[
            ("first_name", "Grace"),
            ("last_name", "Hopper"),
            ("name", "Someone Else"),
            ("username", "perferendis"),
        ],
        &[],
    );
    assert_eq!(held["name"], JsonValue::from("Grace Hopper"));
    assert_eq!(held["username"], JsonValue::from("grace.hopper"));
}

#[test]
fn what_the_caller_wrote_stands() {
    let held = wired(
        &[
            ("first_name", "Ada"),
            ("last_name", "Lovelace"),
            ("full_name", "Whatever I Said"),
        ],
        &["full_name".to_string()],
    );
    assert_eq!(held["full_name"], JsonValue::from("Whatever I Said"));
}

/// A key the record does not carry stays uncarried: deriving a value for it
/// would put an optional field back that was deliberately absent.
#[test]
fn an_absent_field_is_not_conjured_back() {
    let fields = vec![field("first_name"), field("last_name"), field("full_name")];
    let mut held = record(&[("first_name", "Ada"), ("last_name", "Lovelace")]);
    wire(&fields, &mut held, &[]);
    assert!(!held.contains_key("full_name"));
}

#[test]
fn wiring_a_record_twice_changes_nothing() {
    let pairs = [
        ("first_name", "Ada"),
        ("last_name", "Lovelace"),
        ("full_name", "x"),
        ("email", "x@y.example"),
        ("id", "abc"),
        ("avatar_url", "https://cdn.example.com/a/b"),
    ];
    let fields: Vec<FieldDef> = pairs.iter().map(|(name, _)| field(name)).collect();
    let mut once = record(&pairs);
    wire(&fields, &mut once, &[]);
    let mut twice = once.clone();
    wire(&fields, &mut twice, &[]);
    assert_eq!(once, twice);
}

#[test]
fn a_record_with_nothing_to_derive_from_is_left_alone() {
    let held = wired(&[("title", "Something"), ("email", "a@b.example")], &[]);
    assert_eq!(held["email"], JsonValue::from("a@b.example"));
}
