//! The mess a real recording carries.
//!
//! A generated value is always well formed, and a recorded one very often is
//! not: it comes back empty, or redacted by a proxy, or truncated by whatever
//! wrote the log, or filled with a placeholder that means "we did not have
//! one". A corpus without any of that produces a model that has never met the
//! values it will actually be asked about, and the first field it fails on is
//! the one holding `N/A`.
//!
//! Two rules keep the disturbance from turning into mislabelling. The only
//! sample a field has is never disturbed -- with nothing else to compare it to,
//! a lone `***` is not an email address by any reading. And at most a third of a
//! field's samples are, so what the field holds stays legible from the rest.

use super::dialect::NoiseLevel;
use super::rng::Rng;

/// Placeholders that stand in for a value nobody had.
const PLACEHOLDERS: [&str; 10] = [
    "N/A", "n/a", "-", "--", "unknown", "UNKNOWN", "null", "none", "TBD", "not set",
];

/// What a redacting proxy leaves behind.
const REDACTIONS: [&str; 7] = [
    "***",
    "*****",
    "[REDACTED]",
    "[redacted]",
    "<hidden>",
    "\u{2022}\u{2022}\u{2022}\u{2022}",
    "XXXXXXXX",
];

/// Disturb a share of a field's samples.
///
/// Returns how many were changed, which the census reads to report how much of
/// the corpus is disturbed at all.
pub fn disturb(values: &mut [String], level: NoiseLevel, rng: &mut Rng) -> usize {
    if values.len() < 2 {
        return 0;
    }

    let ceiling = values.len() / 3;
    if ceiling == 0 {
        return 0;
    }

    let chance = level.per_sample_chance();
    let mut changed = 0;
    for index in 0..values.len() {
        if changed >= ceiling {
            break;
        }
        if !rng.chance(chance, 16) {
            continue;
        }
        let Some(value) = values.get_mut(index) else {
            continue;
        };
        let disturbed = disturb_one(value, rng);
        if disturbed != *value {
            *value = disturbed;
            changed += 1;
        }
    }
    changed
}

/// One sample, as something a recording might really have held instead.
fn disturb_one(value: &str, rng: &mut Rng) -> String {
    match rng.weighted(&[3, 3, 3, 3, 2, 2, 1, 1]) {
        0 => String::new(),
        1 => rng.pick(&PLACEHOLDERS).to_string(),
        2 => rng.pick(&REDACTIONS).to_string(),
        // Truncated by whatever wrote the log. Only worth doing to something
        // long enough for the truncation to be visible.
        3 => truncate(value, rng),
        4 => format!(" {value} "),
        5 => partially_redacted(value, rng),
        6 => value.to_uppercase(),
        _ => percent_encoded(value),
    }
}

fn truncate(value: &str, rng: &mut Rng) -> String {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() < 12 {
        return value.to_string();
    }
    let keep = rng.between(6, characters.len().saturating_sub(2));
    let head: String = characters.into_iter().take(keep).collect();
    format!("{head}{}", rng.pick(&["...", "\u{2026}", ""]))
}

/// The half-redaction a service applies to something it will still show you:
/// the last few characters of a card number, a key, an address.
fn partially_redacted(value: &str, rng: &mut Rng) -> String {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() < 6 {
        return value.to_string();
    }
    let keep = rng.between(2, 4).min(characters.len());
    let tail: String = characters
        .into_iter()
        .skip(value.chars().count() - keep)
        .collect();
    format!("****{tail}")
}

fn percent_encoded(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            ' ' => "%20".to_string(),
            '/' => "%2F".to_string(),
            ':' => "%3A".to_string(),
            '@' => "%40".to_string(),
            other => other.to_string(),
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::string_slice
)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;

    fn samples(count: usize) -> Vec<String> {
        (0..count)
            .map(|n| format!("value-that-is-long-enough-{n:03}"))
            .collect()
    }

    #[test]
    fn a_lone_sample_is_never_disturbed() {
        // With nothing to compare it against, a single `***` is not evidence of
        // anything, and labelling it would be labelling noise.
        let mut rng = Rng::seeded(1);
        let mut only = vec!["a@b.com".to_string()];
        assert_eq!(disturb(&mut only, NoiseLevel::Messy, &mut rng), 0);
        assert_eq!(only, vec!["a@b.com".to_string()]);
    }

    #[test]
    fn no_more_than_a_third_of_a_field_is_disturbed() {
        let mut rng = Rng::seeded(2);
        for count in 2..40 {
            let mut values = samples(count);
            let original = values.clone();
            let changed = disturb(&mut values, NoiseLevel::Messy, &mut rng);

            assert!(
                changed <= count / 3,
                "{changed} of {count} samples were disturbed"
            );
            let differing = values
                .iter()
                .zip(original.iter())
                .filter(|(now, before)| now != before)
                .count();
            assert_eq!(differing, changed);
        }
    }

    #[test]
    fn a_messier_family_is_disturbed_more_often_than_a_clean_one() {
        let total = |level: NoiseLevel| -> usize {
            let mut rng = Rng::seeded(3);
            (0..400)
                .map(|_| {
                    let mut values = samples(9);
                    disturb(&mut values, level, &mut rng)
                })
                .sum()
        };
        assert!(total(NoiseLevel::Messy) > total(NoiseLevel::Typical));
        assert!(total(NoiseLevel::Typical) > total(NoiseLevel::Clean));
    }

    #[test]
    fn the_disturbances_cover_more_than_one_kind() {
        let mut rng = Rng::seeded(4);
        let mut seen: FxHashSet<String> = FxHashSet::default();
        for _ in 0..600 {
            seen.insert(disturb_one("someone@example.com", &mut rng));
        }
        assert!(
            seen.len() >= 8,
            "only {} kinds of mess: {seen:?}",
            seen.len()
        );
        assert!(seen.contains(""), "nothing ever came back empty");
        assert!(
            seen.iter()
                .any(|value| value.contains("REDACTED") || value.contains('*')),
            "nothing was ever redacted"
        );
    }

    #[test]
    fn a_short_value_is_not_truncated_into_nonsense() {
        let mut rng = Rng::seeded(5);
        assert_eq!(truncate("ab", &mut rng), "ab");
        assert_eq!(partially_redacted("ab", &mut rng), "ab");
    }

    #[test]
    fn truncation_keeps_the_start_of_what_it_truncated() {
        let mut rng = Rng::seeded(6);
        let long = "550e8400-e29b-41d4-a716-446655440000";
        for _ in 0..50 {
            let cut = truncate(long, &mut rng);
            assert!(long.starts_with(&cut[..6]), "{cut}");
            assert!(cut.chars().count() <= long.chars().count() + 3);
        }
    }

    #[test]
    fn disturbance_never_splits_a_character_in_half() {
        // Truncating by bytes rather than characters would produce invalid text
        // for exactly the locales this corpus exists to carry.
        let mut rng = Rng::seeded(7);
        for value in [
            "東京都渋谷区の請求書です",
            "Привет мир и все вокруг",
            "مرحبا بالعالم كله",
        ] {
            for _ in 0..40 {
                let disturbed = disturb_one(value, &mut rng);
                assert!(disturbed.chars().count() <= value.chars().count() + 8);
            }
        }
    }
}
