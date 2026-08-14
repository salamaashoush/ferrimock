//! Turning a field into numbers.
//!
//! A model sees a field name and a handful of sampled values. This renders that
//! into a fixed-width vector: what the name looks like, what the values look
//! like, and how much they agree with each other.
//!
//! The layout is **versioned**. A model artifact records the version it was
//! trained under and refuses to load against a different one. Without that, a
//! feature added in the middle silently shifts every dimension after it and the
//! model keeps predicting -- confidently, and wrongly.

use regex::Regex;
use std::sync::LazyLock;

/// Bumped whenever the meaning or order of any dimension changes.
///
/// Adding a feature at the end still counts: the width is part of the contract.
pub const FEATURE_LAYOUT_VERSION: u32 = 1;

/// Width of the vector [`extract`] produces.
pub const FEATURE_COUNT: usize = NAME_FEATURES + VALUE_FEATURES;

const NAME_FEATURES: usize = 8 + NAME_KEYWORDS.len();
const VALUE_FEATURES: usize = 12 + CHARACTER_CLASSES + VALUE_PATTERN_COUNT + 6;
const CHARACTER_CLASSES: usize = 6;
/// Number of entries in [`VALUE_PATTERNS`]. Stated rather than derived, because
/// the regexes are built lazily and a const cannot count them; a test holds the
/// two in step.
const VALUE_PATTERN_COUNT: usize = 24;

/// Words in a field name that hint at what it holds. Order is part of the
/// layout: each contributes one dimension at its own index.
const NAME_KEYWORDS: [&str; 24] = [
    "id", "uuid", "guid", "key", "token", "secret", "hash", "email", "mail", "phone", "name",
    "user", "login", "url", "uri", "link", "href", "path", "file", "type", "date", "time", "at",
    "count",
];

/// Regexes a value either matches or does not. Order is part of the layout.
static VALUE_PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    // Anchored: a value that merely contains an email is not an email field.
    let compile = |name: &'static str, pattern: &str| {
        #[allow(clippy::expect_used)] // Literal patterns; a bad one is a build-time bug
        (
            name,
            Regex::new(pattern).expect("feature pattern must compile"),
        )
    };
    vec![
        compile(
            "uuid",
            r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
        ),
        compile("email", r"^[^@\s]+@[^@\s]+\.[A-Za-z]{2,}$"),
        compile("url", r"^https?://\S+$"),
        compile("iso_date", r"^\d{4}-\d{2}-\d{2}$"),
        compile("iso_datetime", r"^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}"),
        compile("ipv4", r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$"),
        compile("semver", r"^\d+\.\d+\.\d+([-+][0-9A-Za-z.-]+)*$"),
        compile("hex", r"^[0-9a-fA-F]{6,}$"),
        compile("base64", r"^[A-Za-z0-9+/]{8,}={0,2}$"),
        compile("digits", r"^\d+$"),
        compile("decimal", r"^-?\d+\.\d+$"),
        compile("bool_word", r"^(?i:true|false)$"),
        compile("country_code", r"^[A-Z]{2}$"),
        compile("currency_code", r"^[A-Z]{3}$"),
        compile("locale", r"^[a-z]{2}([-_][A-Z]{2})?$"),
        compile("timezone", r"^[A-Z][A-Za-z]+/[A-Z][A-Za-z_]+$"),
        compile("mime", r"^[a-z]+/[a-z0-9.+-]+$"),
        compile("filename", r"^[^/\\]+\.[A-Za-z0-9]{1,6}$"),
        compile("path", r"^(/[^/]*)+$"),
        compile("jwt", r"^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$"),
        compile("phone", r"^\+?[\d][\d\s().-]{6,}$"),
        compile("postal", r"^[A-Za-z0-9][A-Za-z0-9 -]{2,9}$"),
        compile("sentence", r"^[A-Z][^.!?]{10,}[.!?]$"),
        compile("has_space", r"\s"),
    ]
});

/// A field rendered as numbers, all in `[0, 1]`.
pub fn extract(field_name: &str, values: &[&str]) -> Vec<f32> {
    let mut features = Vec::with_capacity(FEATURE_COUNT);
    push_name_features(field_name, &mut features);
    push_value_features(values, &mut features);
    debug_assert_eq!(features.len(), FEATURE_COUNT);
    features
}

/// Names of every dimension, in order. Used to print what a linear model
/// learned, which is the only way to tell a real signal from a lucky one.
pub fn feature_names() -> Vec<String> {
    let mut names = vec![
        "name.length".to_string(),
        "name.words".to_string(),
        "name.snake".to_string(),
        "name.camel".to_string(),
        "name.pascal".to_string(),
        "name.has_digits".to_string(),
        "name.leading_underscore".to_string(),
        "name.entropy".to_string(),
    ];
    names.extend(NAME_KEYWORDS.iter().map(|k| format!("name.kw.{k}")));

    names.extend(
        [
            "value.len_min",
            "value.len_max",
            "value.len_mean",
            "value.len_spread",
            "value.entropy",
            "value.distinct_ratio",
            "value.all_same_length",
            "value.sample_count",
            "value.empty_ratio",
            "value.numeric_ratio",
            "value.magnitude",
            "value.monotonic",
        ]
        .iter()
        .map(|n| (*n).to_string()),
    );
    names.extend(
        [
            "char.digit",
            "char.lower",
            "char.upper",
            "char.punct",
            "char.space",
            "char.other",
        ]
        .iter()
        .map(|n| (*n).to_string()),
    );
    names.extend(
        VALUE_PATTERNS
            .iter()
            .map(|(name, _)| format!("pattern.{name}")),
    );
    names.extend(
        [
            "value.prefix_shared",
            "value.suffix_shared",
            "value.low_cardinality",
            "value.single_sample",
            "value.name_echoed",
            "value.length_is_fixed_known",
        ]
        .iter()
        .map(|n| (*n).to_string()),
    );

    debug_assert_eq!(names.len(), FEATURE_COUNT);
    names
}

fn push_name_features(field_name: &str, out: &mut Vec<f32>) {
    let lowered = field_name.to_lowercase();
    let chars: Vec<char> = field_name.chars().collect();

    out.push(ratio(field_name.len(), 40));
    out.push(ratio(word_count(field_name), 6));
    out.push(flag(field_name.contains('_') || field_name.contains('-')));
    out.push(flag(chars.windows(2).any(|pair| {
        matches!(pair, [first, second] if first.is_lowercase() && second.is_uppercase())
    })));
    out.push(flag(chars.first().is_some_and(|c| c.is_uppercase())));
    out.push(flag(chars.iter().any(char::is_ascii_digit)));
    out.push(flag(field_name.starts_with('_')));
    out.push(shannon_entropy(field_name));

    // A keyword matches on a word boundary, not as a substring: `account`
    // contains `count`, and reading that as a counter is how a customer's
    // account number became a total.
    let words = split_words(&lowered);
    for keyword in NAME_KEYWORDS {
        out.push(flag(words.iter().any(|word| word == keyword)));
    }
}

#[allow(clippy::cast_precision_loss)] // Sample counts and lengths are small
fn push_value_features(values: &[&str], out: &mut Vec<f32>) {
    if values.is_empty() {
        out.extend(std::iter::repeat_n(0.0, VALUE_FEATURES));
        return;
    }

    let lengths: Vec<usize> = values.iter().map(|v| v.chars().count()).collect();
    let min = lengths.iter().copied().min().unwrap_or(0);
    let max = lengths.iter().copied().max().unwrap_or(0);
    let mean = lengths.iter().sum::<usize>() as f32 / lengths.len() as f32;

    out.push(ratio(min, 64));
    out.push(ratio(max, 64));
    out.push((mean / 64.0).clamp(0.0, 1.0));
    out.push(ratio(max.saturating_sub(min), 64));

    let joined: String = values.concat();
    out.push(shannon_entropy(&joined));

    let mut distinct: Vec<&&str> = values.iter().collect();
    distinct.sort_unstable();
    distinct.dedup();
    out.push(distinct.len() as f32 / values.len() as f32);

    out.push(flag(min == max));
    out.push(ratio(values.len(), 16));
    out.push(values.iter().filter(|v| v.is_empty()).count() as f32 / values.len() as f32);

    let numeric: Vec<f64> = values.iter().filter_map(|v| v.parse::<f64>().ok()).collect();
    out.push(numeric.len() as f32 / values.len() as f32);
    // Digit count rather than the value itself: a magnitude feature must not let
    // one enormous sample dominate the vector.
    let magnitude = numeric
        .iter()
        .map(|n| n.abs().max(1.0).log10())
        .fold(0.0_f64, f64::max);
    #[allow(clippy::cast_possible_truncation)] // Clamped to [0,1] before the cast
    out.push((magnitude / 20.0).clamp(0.0, 1.0) as f32);
    out.push(flag(
        numeric.len() == values.len() && numeric.windows(2).all(|w| w[1] > w[0]),
    ));

    let mut counts = [0usize; CHARACTER_CLASSES];
    let mut total = 0usize;
    for ch in joined.chars() {
        total += 1;
        let class = if ch.is_ascii_digit() {
            0
        } else if ch.is_lowercase() {
            1
        } else if ch.is_uppercase() {
            2
        } else if ch.is_ascii_punctuation() {
            3
        } else if ch.is_whitespace() {
            4
        } else {
            5
        };
        counts[class] += 1;
    }
    for count in counts {
        out.push(if total == 0 {
            0.0
        } else {
            count as f32 / total as f32
        });
    }

    // A pattern feature is the share of samples matching it, not whether any
    // one did. One stray URL in a list of names should not read as a URL field.
    for (_, pattern) in VALUE_PATTERNS.iter() {
        let matched = values.iter().filter(|v| pattern.is_match(v)).count();
        out.push(matched as f32 / values.len() as f32);
    }

    out.push(flag(shared_affix(values, Affix::Prefix) >= 3));
    out.push(flag(shared_affix(values, Affix::Suffix) >= 3));
    out.push(flag(distinct.len() <= 4 && values.len() >= 4));
    out.push(flag(values.len() == 1));
    // Placeholder dimensions kept meaningful rather than reserved: whether the
    // values echo something structural about themselves.
    out.push(flag(values.iter().all(|v| v.contains(':'))));
    out.push(flag(
        min == max && matches!(min, 8 | 16 | 20 | 24 | 32 | 36 | 40 | 64),
    ));
}

enum Affix {
    Prefix,
    Suffix,
}

/// Length of the longest prefix or suffix every value shares.
fn shared_affix(values: &[&str], affix: Affix) -> usize {
    let Some(first) = values.first() else {
        return 0;
    };
    let take = |value: &str, n: usize| -> Option<String> {
        let chars: Vec<char> = value.chars().collect();
        if chars.len() < n {
            return None;
        }
        Some(match affix {
            Affix::Prefix => chars.iter().take(n).collect(),
            Affix::Suffix => chars.iter().rev().take(n).collect(),
        })
    };

    let limit = first.chars().count().min(16);
    (1..=limit)
        .take_while(|n| {
            take(first, *n).is_some_and(|reference| {
                values
                    .iter()
                    .all(|value| take(value, *n).is_some_and(|other| other == reference))
            })
        })
        .last()
        .unwrap_or(0)
}

fn split_words(lowered: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for ch in lowered.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn word_count(field_name: &str) -> usize {
    let separated = field_name
        .chars()
        .filter(|c| *c == '_' || *c == '-')
        .count()
        + 1;
    let camel = field_name
        .chars()
        .zip(field_name.chars().skip(1))
        .filter(|(a, b)| a.is_lowercase() && b.is_uppercase())
        .count();
    separated + camel
}

#[allow(clippy::cast_precision_loss)]
fn shannon_entropy(text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    let mut counts = rustc_hash::FxHashMap::<char, usize>::default();
    for ch in text.chars() {
        *counts.entry(ch).or_insert(0) += 1;
    }
    let total = text.chars().count() as f32;
    #[allow(clippy::cast_precision_loss)] // Character counts are far below f32's exact range
    let entropy: f32 = counts
        .values()
        .map(|count| {
            let p = *count as f32 / total;
            -p * p.log2()
        })
        .sum();
    // 6 bits/char saturates: past that the distinction stops being informative.
    (entropy / 6.0).clamp(0.0, 1.0)
}

#[allow(clippy::cast_precision_loss)]
fn ratio(value: usize, cap: usize) -> f32 {
    (value as f32 / cap as f32).clamp(0.0, 1.0)
}

fn flag(condition: bool) -> f32 {
    if condition { 1.0 } else { 0.0 }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn named(name: &str) -> Vec<f32> {
        extract(name, &["a"])
    }

    fn feature(name: &str, values: &[&str], dimension: &str) -> f32 {
        let names = feature_names();
        let index = names
            .iter()
            .position(|n| n == dimension)
            .unwrap_or_else(|| panic!("no dimension {dimension}"));
        extract(name, values)[index]
    }

    #[test]
    fn the_declared_pattern_count_matches_the_patterns() {
        // The width is a const so it can size arrays; this is what keeps that
        // const honest when a pattern is added.
        assert_eq!(VALUE_PATTERNS.len(), VALUE_PATTERN_COUNT);
    }

    #[test]
    fn pattern_names_are_unique() {
        let mut names: Vec<&str> = VALUE_PATTERNS.iter().map(|(name, _)| *name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), VALUE_PATTERNS.len());
    }

    #[test]
    fn the_vector_is_always_the_declared_width() {
        assert_eq!(extract("", &[]).len(), FEATURE_COUNT);
        assert_eq!(extract("id", &["1"]).len(), FEATURE_COUNT);
        assert_eq!(
            extract("a_very_long_field_name", &["x"; 32]).len(),
            FEATURE_COUNT
        );
        assert_eq!(feature_names().len(), FEATURE_COUNT);
    }

    #[test]
    fn every_dimension_stays_in_range() {
        for vector in [
            extract("", &[]),
            extract("user_id", &["1", "2", "3"]),
            extract("blob", &[&"x".repeat(10_000)]),
            extract("n", &["999999999999999999999"]),
        ] {
            for (index, value) in vector.iter().enumerate() {
                assert!(
                    (0.0..=1.0).contains(value) && value.is_finite(),
                    "dimension {index} left [0,1]: {value}"
                );
            }
        }
    }

    #[test]
    fn a_keyword_matches_a_word_not_a_substring() {
        // The lesson `account_number` taught the heuristic detector.
        assert_eq!(feature("account_number", &["1"], "name.kw.count"), 0.0);
        assert_eq!(feature("total_count", &["1"], "name.kw.count"), 1.0);
        assert_eq!(feature("identifier", &["1"], "name.kw.id"), 0.0);
        assert_eq!(feature("user_id", &["1"], "name.kw.id"), 1.0);
    }

    #[test]
    fn a_pattern_feature_is_a_share_not_a_flag() {
        let all = feature("x", &["a@b.com", "c@d.org"], "pattern.email");
        let half = feature("x", &["a@b.com", "plain"], "pattern.email");
        let none = feature("x", &["plain", "text"], "pattern.email");

        assert!((all - 1.0).abs() < f32::EPSILON);
        assert!((half - 0.5).abs() < f32::EPSILON);
        assert!(none.abs() < f32::EPSILON);
    }

    #[test]
    fn no_samples_zeroes_the_value_half_without_touching_the_name_half() {
        let vector = extract("user_email", &[]);
        assert!(
            vector[..NAME_FEATURES].iter().any(|v| *v > 0.0),
            "the name still says something"
        );
        assert!(vector[NAME_FEATURES..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_shared_prefix_is_recognised() {
        assert_eq!(
            feature("x", &["file_001", "file_002", "file_003"], "value.prefix_shared"),
            1.0
        );
        assert_eq!(
            feature("x", &["alpha", "beta", "gamma"], "value.prefix_shared"),
            0.0
        );
    }

    #[test]
    fn distinctness_separates_an_enum_from_an_id() {
        let enumeration = feature("x", &["a", "b", "a", "b"], "value.distinct_ratio");
        let identifiers = feature("x", &["1", "2", "3", "4"], "value.distinct_ratio");
        assert!(enumeration < identifiers);
        assert!((identifiers - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn one_enormous_sample_cannot_dominate_the_magnitude() {
        let huge = feature("n", &["999999999999999999999999999999"], "value.magnitude");
        assert!((0.0..=1.0).contains(&huge));
    }

    #[test]
    fn names_and_dimensions_stay_in_step() {
        // The whole versioning scheme rests on this: if the printed names drift
        // from the vector, every explanation of what a model learned is wrong.
        let names = feature_names();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate dimension name");
        assert_eq!(names.len(), named("x").len());
    }
}
