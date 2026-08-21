#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;
use crate::config::serve::Behaviour;

#[test]
fn a_tag_is_the_same_tag_for_the_same_bytes() {
    let body = serde_json::json!({ "id": "a", "name": "Reports" });
    assert_eq!(etag_of(&body), etag_of(&body.clone()));
    assert_ne!(
        etag_of(&body),
        etag_of(&serde_json::json!({ "id": "a", "name": "Archive" }))
    );
    assert!(etag_of(&body).starts_with('"') && etag_of(&body).ends_with('"'));
}

#[test]
fn a_client_can_name_several_tags_or_any_at_all() {
    let tag = "\"abc\"";
    assert!(matches_tag(tag, tag));
    assert!(matches_tag("*", tag));
    assert!(matches_tag("\"zzz\", \"abc\"", tag));
    assert!(
        matches_tag("W/\"abc\"", tag),
        "a weak tag is the same representation as far as a mock is concerned"
    );
    assert!(!matches_tag("\"zzz\"", tag));
}

#[test]
fn a_header_is_read_however_the_client_cased_it() {
    let mut ctx = RequestContext::new();
    ctx.headers
        .insert("If-None-Match".to_string(), "\"abc\"".to_string());
    assert_eq!(header(&ctx, "if-none-match"), Some("\"abc\""));
    assert_eq!(header(&ctx, "IF-NONE-MATCH"), Some("\"abc\""));
    assert_eq!(header(&ctx, "if-match"), None);

    ctx.headers.insert("X-Blank".to_string(), "  ".to_string());
    assert_eq!(header(&ctx, "x-blank"), None, "a blank header said nothing");
}

#[test]
fn a_problem_carries_what_a_generic_reader_looks_for() {
    let held = problem(StatusCode::PRECONDITION_FAILED, "the version moved on");
    assert_eq!(held["status"], serde_json::json!(412));
    assert_eq!(held["title"], serde_json::json!("Precondition Failed"));
    assert_eq!(held["detail"], serde_json::json!("the version moved on"));
    assert_eq!(held["type"], serde_json::json!("about:blank"));
}

#[test]
fn replay_gets_nothing_whatever_the_mount_asked_for() {
    let asked = Behaviour {
        conditional: true,
        soft_delete: true,
        problem_json: true,
        replica_lag: 3,
        idempotency: true,
    };
    assert!(!asked.is_none());
    assert!(Behaviour::none().is_none());
    assert_eq!(Behaviour::default(), Behaviour::none());
}
