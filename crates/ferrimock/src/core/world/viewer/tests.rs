#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;
use rustc_hash::FxHashMap;

fn headers(pairs: &[(&str, &str)]) -> FxHashMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect()
}

fn keys(count: usize) -> Vec<EntityKey> {
    (0..count)
        .map(|i| EntityKey::single(format!("user-{i}")))
        .collect()
}

#[test]
fn a_credential_is_read_however_the_client_sends_it() {
    assert_eq!(
        Credential::read(&headers(&[("Authorization", "Bearer abc123")])),
        Credential::Presented("abc123".to_string())
    );
    assert_eq!(
        Credential::read(&headers(&[("authorization", "abc123")])),
        Credential::Presented("abc123".to_string()),
        "the scheme is not the caller: two client libraries send the same token twice"
    );
    assert_eq!(
        Credential::read(&headers(&[("X-Api-Key", "abc123")])),
        Credential::Presented("abc123".to_string())
    );
    assert_eq!(Credential::read(&headers(&[])), Credential::Absent);
    assert_eq!(
        Credential::read(&headers(&[("Authorization", "   ")])),
        Credential::Absent
    );
}

#[test]
fn the_same_token_is_the_same_person_every_time() {
    let held = keys(40);
    let one = Credential::Presented("abc".to_string());
    let first = one.bound_to(7, "User", &held).unwrap();
    assert_eq!(one.bound_to(7, "User", &held), Some(first));
}

#[test]
fn two_tokens_are_two_people() {
    let held = keys(40);
    let landed: std::collections::BTreeSet<String> = (0..40)
        .map(|i| {
            Credential::Presented(format!("token-{i}"))
                .bound_to(7, "User", &held)
                .unwrap()
                .to_string()
        })
        .collect();
    assert!(
        landed.len() > 10,
        "every caller landed on the same handful: {landed:?}"
    );
}

#[test]
fn nothing_presented_is_nobody() {
    assert_eq!(Credential::Absent.bound_to(7, "User", &keys(40)), None);
    assert_eq!(
        Credential::Presented("abc".to_string()).bound_to(7, "User", &[]),
        None,
        "a world with no users has no viewer either"
    );
}
