#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use super::*;
use crate::fake_data::rng;

#[test]
fn every_place_answers_every_question_it_is_asked() {
    let _scope = rng::scope_seeded(7);
    for place in places() {
        assert!(place.locale.contains('-'), "{}", place.locale);
        assert_eq!(place.country_code.len(), 2, "{}", place.country_code);
        assert_eq!(place.currency.len(), 3, "{}", place.currency);
        assert!(place.timezone.contains('/'), "{}", place.timezone);
        assert!(place.calling_code.starts_with('+'));

        assert!(place.person().contains(' '));
        assert!(!place.city().is_empty());
        assert!(place.phone().starts_with(place.calling_code));
        assert!(!place.postal_code().is_empty());
    }
}

#[test]
fn a_postal_code_is_written_the_way_its_country_writes_one() {
    let _scope = rng::scope_seeded(3);
    let by_code = |code: &str| {
        places()
            .iter()
            .find(|place| place.country_code == code)
            .unwrap()
    };

    let us = by_code("US").postal_code();
    assert_eq!(us.len(), 5);
    assert!(us.chars().all(|c| c.is_ascii_digit()), "{us}");

    let gb = by_code("GB").postal_code();
    assert!(gb.contains(' '), "{gb}");
    assert!(
        gb.chars().next().is_some_and(|c| c.is_ascii_uppercase()),
        "{gb}"
    );

    let nl = by_code("NL").postal_code();
    let (digits, letters) = nl.split_once(' ').unwrap();
    assert_eq!(digits.len(), 4);
    assert_eq!(letters.len(), 2);
}

#[test]
fn a_place_is_the_same_place_for_the_same_word() {
    for word in 0..500_u64 {
        let derived = rng::derive_seed(1, "place", word);
        assert_eq!(place_of(derived).locale, place_of(derived).locale);
    }
    let landed: std::collections::BTreeSet<&str> = (0..500_u64)
        .map(|word| place_of(rng::derive_seed(1, "place", word)).locale)
        .collect();
    assert_eq!(
        landed.len(),
        places().len(),
        "every place should be reachable"
    );
}
