#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use super::*;
use crate::fake_data::rng;

fn words(stream: &str, count: u64) -> Vec<u64> {
    (0..count).map(|i| rng::derive_seed(1, stream, i)).collect()
}

#[test]
fn a_uniform_draw_stays_inside_its_support() {
    let spread = Spread::Uniform {
        low: 3.0,
        high: 9.0,
    };
    for word in words("uniform", 2000) {
        let drawn = spread.draw(word);
        assert!((3.0..=9.0).contains(&drawn), "{drawn}");
    }
}

/// Equal mass per decade is the point: half of a two-decade support falls in
/// its lower decade, which is what a leading-digit profile is made of.
#[test]
fn a_log_uniform_draw_spreads_evenly_over_decades() {
    let spread = Spread::LogUniform {
        low: 1.0,
        high: 100.0,
    };
    let drawn: Vec<f64> = words("log", 4000)
        .into_iter()
        .map(|w| spread.draw(w))
        .collect();
    assert!(drawn.iter().all(|value| (1.0..=100.0).contains(value)));

    let lower = drawn.iter().filter(|value| **value < 10.0).count();
    let share = lower as f64 / drawn.len() as f64;
    assert!((0.45..0.55).contains(&share), "lower decade held {share}");
}

#[test]
fn a_uniform_draw_does_not_spread_evenly_over_decades() {
    let spread = Spread::Uniform {
        low: 1.0,
        high: 100.0,
    };
    let lower = words("flat", 4000)
        .into_iter()
        .filter(|word| spread.draw(*word) < 10.0)
        .count();
    assert!(lower < 500, "a flat draw puts a tenth in the lower decade");
}

#[test]
fn zero_inflation_puts_real_weight_on_zero_and_none_below_it() {
    let spread = Spread::ZeroInflated {
        zero: 0.3,
        inner: Box::new(Spread::LogUniform {
            low: 1.0,
            high: 1000.0,
        }),
    };
    let drawn: Vec<f64> = words("zero", 4000)
        .into_iter()
        .map(|w| spread.draw(w))
        .collect();
    let zeros = drawn.iter().filter(|value| **value == 0.0).count() as f64 / drawn.len() as f64;
    assert!((0.24..0.36).contains(&zeros), "zeros held {zeros}");
    assert!(drawn.iter().all(|value| *value >= 0.0));
}

#[test]
fn a_geometric_length_is_mostly_short_and_occasionally_long() {
    let spread = Spread::Geometric { mean: 3.0 };
    let drawn: Vec<f64> = words("geo", 4000)
        .into_iter()
        .map(|w| spread.draw(w))
        .collect();
    let average = drawn.iter().sum::<f64>() / drawn.len() as f64;
    assert!((2.0..4.5).contains(&average), "mean {average}");
    assert!(drawn.contains(&0.0));
    assert!(drawn.iter().any(|value| *value > 8.0));
}

#[test]
fn a_whole_draw_respects_the_bounds_it_was_given() {
    let spread = Spread::LogUniform {
        low: 1.0,
        high: 1e9,
    };
    for word in words("whole", 2000) {
        let drawn = spread.whole(word, 5, 40);
        assert!((5..=40).contains(&drawn), "{drawn}");
    }
}

/// The marginal has to be skewed — that is the only thing a check with no
/// corpus can test — without asserting which member carries the mass.
#[test]
fn a_ranking_is_skewed_and_does_not_favour_the_first_member() {
    let mut modal_is_first = 0;
    let mut flat = 0;
    for field in 0..40_u64 {
        let ranking = Ranking::of(5, rng::derive_seed(1, "field", field));
        let mut counts = [0_usize; 5];
        for word in words(&format!("draw{field}"), 600) {
            counts[ranking.pick(word)] += 1;
        }
        assert_eq!(counts.iter().sum::<usize>(), 600);

        let most = counts.iter().copied().max().unwrap();
        let least = counts.iter().copied().min().unwrap();
        if most < 200 {
            flat += 1;
        }
        assert!(most > least, "{counts:?}");
        if counts.iter().position(|c| *c == most) == Some(0) {
            modal_is_first += 1;
        }
    }
    assert_eq!(flat, 0, "every enum should be visibly skewed");
    assert!(
        (2..=18).contains(&modal_is_first),
        "declaration order must not decide the mode: {modal_is_first} of 40"
    );
}

#[test]
fn a_ranking_reaches_every_member() {
    for members in 2..12_usize {
        let ranking = Ranking::of(members, rng::derive_seed(1, "members", members as u64));
        let mut seen = vec![false; members];
        for word in words("reach", 20_000) {
            seen[ranking.pick(word)] = true;
        }
        assert!(seen.iter().all(|hit| *hit), "{members} members: {seen:?}");
    }
}

#[test]
fn a_permutation_is_a_bijection() {
    for members in 1..40_usize {
        for field in 0..30_u64 {
            let word = rng::derive_seed(3, "perm", field);
            let mut landed = vec![false; members];
            for index in 0..members {
                let at = permuted(index, members, word);
                assert!(!landed[at], "{members} members collided at {at}");
                landed[at] = true;
            }
        }
    }
}

/// Half the users of an API are not administrators.
#[test]
fn a_lopsided_chance_is_never_a_fair_coin() {
    for field in 0..500_u64 {
        let chance = lopsided_chance(rng::derive_seed(1, "flag", field));
        assert!((0.01..=0.99).contains(&chance), "{chance}");
        assert!(
            (chance - 0.5).abs() > 0.07,
            "a boolean should not read as a coin flip: {chance}"
        );
    }
}

#[test]
fn a_chance_is_the_rate_it_says_it_is() {
    for chance in [0.05, 0.25, 0.8] {
        let hits = words(&format!("coin{chance}"), 4000)
            .into_iter()
            .filter(|word| falls_within(chance, *word))
            .count() as f64
            / 4000.0;
        assert!((hits - chance).abs() < 0.03, "{chance} came out {hits}");
    }
}
