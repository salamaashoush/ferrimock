//! Reducing a value to the pattern it is made of.
//!
//! `2024-03-17` becomes `d{4}-d{2}-d{2}`, and every other ISO date becomes the
//! same string. Two things read this. A census counts distinct signatures to say
//! how much a corpus really covers, and feature extraction asks whether a
//! field's samples share one -- a field whose values all have the same shape is
//! a structured identifier, and one whose values do not is prose.
//!
//! A run of letters and digits is classified by what the *whole run* holds
//! rather than character by character. That is the difference between a useful
//! signature and a useless one: read one character at a time, two UUIDs differ
//! wherever one happens to have a digit where the other has a letter, and a
//! million UUIDs are a million shapes.

use std::fmt::Write;

/// What a run of alphanumeric characters is made of.
#[derive(Default, Clone, Copy)]
struct Composition {
    digits: bool,
    lower: bool,
    upper: bool,
    other_script: bool,
    length: usize,
}

impl Composition {
    fn class(self) -> char {
        match (self.digits, self.lower, self.upper, self.other_script) {
            (true, false, false, false) => 'd',
            (false, true, false, false) => 'a',
            (false, false, true, false) => 'A',
            // Everything outside ASCII collapses to one class: the distinction
            // that matters is Latin against not, not Greek against Thai.
            (false, false, false, true) => 'w',
            // Mixed. A hex digest, a base62 key and a UUID segment are all this,
            // which is exactly right: they are one shape.
            _ => 'x',
        }
    }
}

/// The shape of a value.
///
/// Punctuation is kept as itself, because the punctuation *is* the shape: a UUID
/// and a compact UUID differ in nothing else.
pub fn signature(value: &str) -> String {
    let mut signature = String::new();
    let mut run = Composition::default();

    for character in value.chars() {
        if character.is_ascii_digit() {
            run.digits = true;
        } else if character.is_ascii_lowercase() {
            run.lower = true;
        } else if character.is_ascii_uppercase() {
            run.upper = true;
        } else if character.is_alphanumeric() {
            run.other_script = true;
        } else {
            flush(&mut signature, run);
            run = Composition::default();
            signature.push(if character.is_whitespace() {
                '_'
            } else {
                character
            });
            continue;
        }
        run.length += 1;
    }
    flush(&mut signature, run);

    signature
}

fn flush(signature: &mut String, run: Composition) {
    if run.length == 0 {
        return;
    }
    let class = run.class();
    if run.length == 1 {
        signature.push(class);
    } else {
        // Long runs saturate: a 200-character and a 300-character blob are the
        // same shape, and counting them apart would make every long value its
        // own shape.
        let _ = write!(signature, "{class}{{{}}}", run.length.min(64));
    }
}

/// The share of values that have the most common shape among them.
///
/// One for a field whose samples all look alike, and near zero for one where
/// every sample is shaped differently. The single most useful thing that can be
/// said about a set of samples: an identifier column agrees with itself, and a
/// description column never does.
#[allow(clippy::cast_precision_loss)] // sample counts are tiny
pub fn agreement(values: &[&str]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut counts: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
    for value in values {
        *counts.entry(signature(value)).or_insert(0) += 1;
    }
    let modal = counts.values().copied().max().unwrap_or(0);
    modal as f64 / values.len() as f64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;

    #[test]
    fn a_signature_is_the_shape_and_not_the_value() {
        assert_eq!(signature("2024-03-17"), "d{4}-d{2}-d{2}");
        assert_eq!(signature("2019-11-02"), "d{4}-d{2}-d{2}");
        assert_eq!(
            signature("6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
            "x{8}-x{4}-x{4}-x{4}-x{12}"
        );
    }

    #[test]
    fn different_shapes_get_different_signatures() {
        let shapes: FxHashSet<String> = [
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            "6ba7b8109dad11d180b400c04fd430c8",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "1710668482",
        ]
        .into_iter()
        .map(signature)
        .collect();
        assert_eq!(shapes.len(), 4, "{shapes:?}");
    }

    #[test]
    fn every_script_outside_ascii_collapses_to_one_class() {
        assert_eq!(signature("東京"), signature("서울"));
        assert_ne!(signature("Tokyo"), signature("東京"));
    }

    #[test]
    fn a_long_run_saturates_rather_than_becoming_its_own_shape() {
        assert_eq!(signature(&"a".repeat(200)), signature(&"a".repeat(400)));
        assert_eq!(signature(&"a".repeat(200)), "a{64}");
    }

    #[test]
    fn an_empty_value_has_an_empty_signature() {
        assert_eq!(signature(""), "");
    }

    #[test]
    fn agreement_separates_an_identifier_column_from_prose() {
        let identifiers = agreement(&["cus_9s2Kf3aB1", "cus_7Ld0Pq4zX", "cus_2Mn8Rt6yW"]);
        let prose = agreement(&[
            "The invoice was updated by the owner.",
            "A much longer description of what happened, including several clauses.",
            "Short note.",
            "Renamed.",
        ]);

        assert!(identifiers > 0.9, "{identifiers}");
        // Hex is the case agreement handles least well: a UUID whose last
        // segment happens to be all digits is a different shape from one whose
        // is not, so a hex column agrees with itself less than it looks like it
        // should. Worth knowing rather than worth hiding.
        let hex = agreement(&[
            "550e8400-e29b-41d4-a716-446655440000",
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            "3f2504e0-4f89-11d3-9a0c-0305e82c3301",
        ]);
        assert!(hex > 0.6, "{hex}");
        assert!(prose < 0.5, "{prose}");
        assert_eq!(agreement(&[]), 0.0);
        assert_eq!(agreement(&["one"]), 1.0);
    }
}
