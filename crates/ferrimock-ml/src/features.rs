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
///
/// Version 2 added the dimensions that separate the pairs a corpus covering
/// seventeen families of API convention showed the first layout could not tell
/// apart: an image URL from a plain one, a quoted ETag from an opaque handle, a
/// ten-digit identifier from a ten-digit epoch.
pub const FEATURE_LAYOUT_VERSION: u32 = 2;

/// Width of the vector [`extract`] produces.
pub const FEATURE_COUNT: usize = NAME_FEATURES + VALUE_FEATURES + KIND_FEATURES;

const NAME_FEATURES: usize = 8 + NAME_KEYWORDS.len() + NAME_ROLE_FEATURES + EXTRA_KEYWORDS.len();
const VALUE_FEATURES: usize =
    12 + CHARACTER_CLASSES + VALUE_PATTERN_COUNT + 6 + VALUE_STRUCTURE_FEATURES;
const CHARACTER_CLASSES: usize = 6;
/// Number of entries in [`VALUE_PATTERNS`]. Stated rather than derived, because
/// the regexes are built lazily and a const cannot count them; a test holds the
/// two in step.
const VALUE_PATTERN_COUNT: usize = 26;
/// Dimensions describing what a name's last word makes the field.
const NAME_ROLE_FEATURES: usize = 10;
/// Dimensions describing how a value is built rather than what it matches.
const VALUE_STRUCTURE_FEATURES: usize = 41;
/// Dimensions describing the JSON kind the values were recorded as. Kept apart
/// from the value block because they survive a field having no samples at all.
const KIND_FEATURES: usize = 2;

/// Words in a field name that hint at what it holds. Order is part of the
/// layout: each contributes one dimension at its own index.
const NAME_KEYWORDS: [&str; 24] = [
    "id", "uuid", "guid", "key", "token", "secret", "hash", "email", "mail", "phone", "name",
    "user", "login", "url", "uri", "link", "href", "path", "file", "type", "date", "time", "at",
    "count",
];

/// More of the same, added with the second layout. Kept as a separate list so
/// the first twenty-four keep their indices and an artifact trained under either
/// version still lines up with the names printed for it.
const EXTRA_KEYWORDS: [&str; 20] = [
    "image",
    "avatar",
    "thumbnail",
    "icon",
    "photo",
    "logo",
    "zip",
    "postal",
    "currency",
    "locale",
    "lang",
    "timezone",
    "status",
    "size",
    "total",
    "amount",
    "price",
    "version",
    "etag",
    "cursor",
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
        // A day-first date, which shares its punctuation with a semantic
        // version: `17.03.2024` and `1.2.3` are the same three dotted numbers
        // until someone looks at how many digits each part has.
        compile("dotted_date", r"^\d{1,2}[./]\d{1,2}[./]\d{4}$"),
        compile("compact_date", r"^(19|20)\d{6}$"),
    ]
});

/// A field rendered as numbers, all in `[0, 1]`.
pub fn extract(field: &crate::Field<'_>) -> Vec<f32> {
    let mut features = Vec::with_capacity(FEATURE_COUNT);
    push_name_features(field.name, &mut features);
    push_value_features(field.values, &mut features);
    push_kind_features(field.kind, &mut features);
    debug_assert_eq!(features.len(), FEATURE_COUNT);
    features
}

/// Whether the values were quoted in the JSON they came from.
///
/// The only evidence that separates a count from a numeric string id, and it is
/// not in the text at all.
fn push_kind_features(kind: crate::corpus::ValueKind, out: &mut Vec<f32>) {
    out.push(flag(kind == crate::corpus::ValueKind::Number));
    out.push(flag(kind == crate::corpus::ValueKind::Boolean));
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
            "name.ends_at",
            "name.ends_id",
            "name.ends_url",
            "name.ends_count",
            "name.ends_name",
            "name.ends_code",
            "name.ends_type",
            "name.ends_size",
            "name.flag_prefix",
            "name.time_word",
        ]
        .iter()
        .map(|n| (*n).to_string()),
    );
    names.extend(EXTRA_KEYWORDS.iter().map(|k| format!("name.kw.{k}")));

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
    names.extend(
        [
            "value.quoted",
            "value.weak_etag",
            "value.image_extension",
            "value.file_extension",
            "value.scheme_http",
            "value.scheme_data",
            "value.scheme_urn_or_arn",
            "value.starts_with_slash",
            "value.ends_with_slash",
            "value.single_at",
            "value.cdn_host",
            "value.digits_only",
            "value.hex_only",
            "value.base62_only",
            "value.crockford_only",
            "value.url_safe_only",
            "value.leading_zero_digits",
            "value.epoch_seconds_window",
            "value.epoch_millis_window",
            "value.epoch_micros_window",
            "value.dash_density",
            "value.dot_density",
            "value.slash_density",
            "value.colon_density",
            "value.underscore_density",
            "value.jwt_shape",
            "value.prefixed_id",
            "value.non_ascii_ratio",
            "value.words",
            "value.all_caps",
            "value.title_case",
            "value.distinct_length_ratio",
            "value.percent_encoded",
            "value.shape_agreement",
            "value.placeholder_ratio",
            "value.two_letter_upper",
            "value.three_letter_upper",
            "value.known_currency_code",
            "value.known_country_code",
            "value.known_timezone",
            "value.known_media_type",
            "value.json_number",
            "value.json_boolean",
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
    out.push(flag(chars.windows(2).any(
        |pair| matches!(pair, [first, second] if first.is_lowercase() && second.is_uppercase()),
    )));
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

    // What the *last* word is, which is usually what the field is. `avatar_url`
    // is a URL and `url_count` is a number, and the keyword flags above cannot
    // tell those apart because both carry both words.
    let last = words.last().map_or("", String::as_str);
    let first = words.first().map_or("", String::as_str);
    out.push(flag(last == "at"));
    out.push(flag(last == "id" || last == "ids"));
    out.push(flag(matches!(last, "url" | "uri" | "link" | "href")));
    out.push(flag(matches!(last, "count" | "total" | "num")));
    out.push(flag(last == "name"));
    out.push(flag(last == "code"));
    out.push(flag(last == "type"));
    out.push(flag(matches!(last, "size" | "length" | "bytes")));
    out.push(flag(matches!(
        first,
        "is" | "has" | "can" | "allow" | "should" | "enable" | "use"
    )));
    out.push(flag(words.iter().any(|word| {
        matches!(
            word.as_str(),
            "ts" | "time"
                | "date"
                | "at"
                | "expires"
                | "created"
                | "updated"
                | "modified"
                | "seen"
                | "timestamp"
                | "epoch"
        )
    })));

    for keyword in EXTRA_KEYWORDS {
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

    let numeric: Vec<f64> = values
        .iter()
        .filter_map(|v| v.parse::<f64>().ok())
        .collect();
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
        numeric.len() == values.len()
            && numeric
                .windows(2)
                .all(|pair| matches!(pair, [earlier, later] if later > earlier)),
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
        if let Some(slot) = counts.get_mut(class) {
            *slot += 1;
        }
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

    push_structure_features(values, &lengths, out);
}

/// How the values are *built*, as opposed to what they match.
///
/// Every dimension here exists because the first layout confused a specific pair
/// on a corpus wide enough to contain both halves of it. The pattern flags above
/// say a value looks like a URL; these say whether it ends in `.png`, which is
/// the whole difference between a link and an image.
#[allow(clippy::cast_precision_loss, clippy::too_many_lines)] // sample counts are tiny; one push per dimension
fn push_structure_features(values: &[&str], lengths: &[usize], out: &mut Vec<f32>) {
    let total = values.len() as f32;
    let share = |count: usize| count as f32 / total;
    let of = |predicate: &dyn Fn(&str) -> bool| -> f32 {
        share(values.iter().filter(|value| predicate(value)).count())
    };

    out.push(of(&|value| {
        value.len() >= 2 && value.starts_with('"') && value.ends_with('"')
    }));
    out.push(of(&|value| value.starts_with("W/\"")));
    out.push(of(&|value| has_extension(value, IMAGE_EXTENSIONS)));
    out.push(of(&|value| extension_of(value).is_some()));
    out.push(of(&|value| {
        value.starts_with("https://") || value.starts_with("http://")
    }));
    out.push(of(&|value| value.starts_with("data:")));
    out.push(of(&|value| {
        value.starts_with("urn:") || value.starts_with("arn:")
    }));
    out.push(of(&|value| value.starts_with('/')));
    out.push(of(&|value| value.len() > 1 && value.ends_with('/')));
    out.push(of(&|value| value.matches('@').count() == 1));
    out.push(of(&|value| host_looks_like_a_cdn(value)));

    // Which alphabet a value is closed over. A ULID is not merely "some letters
    // and digits" -- it is Crockford base32 and nothing else, and that is the
    // only thing separating it from a base62 key of similar length.
    out.push(of(&|value| {
        !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
    }));
    out.push(of(&|value| {
        value.len() >= 6 && value.chars().all(|c| c.is_ascii_hexdigit())
    }));
    out.push(of(&|value| {
        value.len() >= 8 && value.chars().all(|c| c.is_ascii_alphanumeric())
    }));
    out.push(of(&|value| {
        value.len() >= 20
            && value.chars().all(|c| CROCKFORD_ALPHABET.contains(c))
            && value.chars().any(|c| c.is_ascii_uppercase())
    }));
    out.push(of(&|value| {
        value.len() >= 8
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }));
    // A leading zero is why an identifier is text rather than a number: parse it
    // and the zero is gone, along with the identifier.
    out.push(of(&|value| {
        value.len() > 1 && value.starts_with('0') && value.chars().all(|c| c.is_ascii_digit())
    }));

    // The three windows a bare number falls in when it is a moment rather than a
    // count. Telling a ten-digit id from a ten-digit epoch is the hardest call in
    // the label space, and length alone cannot make it.
    out.push(of(&|value| {
        in_epoch_window(value, 1_000_000_000, 2_000_000_000)
    }));
    out.push(of(&|value| {
        in_epoch_window(value, 1_000_000_000_000, 2_000_000_000_000)
    }));
    out.push(of(&|value| {
        in_epoch_window(value, 1_000_000_000_000_000, 2_000_000_000_000_000)
    }));

    for separator in ['-', '.', '/', ':', '_'] {
        let density: f32 = values
            .iter()
            .map(|value| {
                let length = value.chars().count();
                if length == 0 {
                    0.0
                } else {
                    value.matches(separator).count() as f32 / length as f32
                }
            })
            .sum::<f32>()
            / total;
        out.push(density.clamp(0.0, 1.0));
    }

    out.push(of(&|value| looks_like_a_jwt(value)));
    out.push(of(&|value| looks_like_a_prefixed_id(value)));

    let non_ascii: f32 = values
        .iter()
        .map(|value| {
            let length = value.chars().count();
            if length == 0 {
                0.0
            } else {
                value.chars().filter(|c| !c.is_ascii()).count() as f32 / length as f32
            }
        })
        .sum::<f32>()
        / total;
    out.push(non_ascii.clamp(0.0, 1.0));

    let words: f32 = values
        .iter()
        .map(|value| (value.split_whitespace().count() as f32 / 12.0).clamp(0.0, 1.0))
        .sum::<f32>()
        / total;
    out.push(words);

    out.push(of(&|value| {
        let letters = value.chars().filter(|c| c.is_alphabetic()).count();
        letters >= 2
            && value
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(char::is_uppercase)
    }));
    out.push(of(&|value| {
        let words: Vec<&str> = value.split_whitespace().collect();
        words.len() >= 2
            && words
                .iter()
                .all(|word| word.chars().next().is_some_and(char::is_uppercase))
    }));

    // Whether the samples are all the same length, measured as how many distinct
    // lengths there were. An identifier column has one; a description has as
    // many as it has samples.
    let mut distinct_lengths: Vec<usize> = lengths.to_vec();
    distinct_lengths.sort_unstable();
    distinct_lengths.dedup();
    out.push((distinct_lengths.len() as f32 / total).clamp(0.0, 1.0));

    out.push(of(&is_percent_encoded));

    // The single most useful thing that can be said about a set of samples: do
    // they agree on a shape at all.
    #[allow(clippy::cast_possible_truncation)] // agreement is in [0,1]
    out.push(crate::shape::agreement(values) as f32);

    // What share of the samples are not values at all. A field is not opaque
    // because a proxy blanked two of its five samples, and without this the
    // model has no way to discount them.
    out.push(of(&is_placeholder));

    out.push(of(&|value| {
        value.chars().count() == 2 && value.chars().all(|c| c.is_ascii_uppercase())
    }));
    out.push(of(&|value| {
        value.chars().count() == 3 && value.chars().all(|c| c.is_ascii_uppercase())
    }));

    // Membership of the closed vocabularies the world actually has. Three
    // upper-case letters is a currency code and it is also `GMT`, and no amount
    // of shape will ever separate those two -- but ISO 4217 will.
    out.push(of(&|value| CURRENCY_CODES.contains(&value)));
    out.push(of(&|value| {
        COUNTRY_CODES.contains(&value.to_ascii_uppercase().as_str())
    }));
    out.push(of(&is_iana_timezone));
    out.push(of(&has_known_media_type));
}

/// ISO 4217, in the codes that reach an API.
const CURRENCY_CODES: [&str; 32] = [
    "USD", "EUR", "GBP", "JPY", "CHF", "AUD", "CAD", "SEK", "NOK", "DKK", "PLN", "CZK", "HUF",
    "RON", "TRY", "RUB", "BRL", "MXN", "ARS", "CLP", "COP", "INR", "CNY", "HKD", "SGD", "KRW",
    "TWD", "THB", "IDR", "ILS", "ZAR", "NZD",
];

/// ISO 3166-1 alpha-2, in the countries that reach an API.
const COUNTRY_CODES: [&str; 40] = [
    "US", "GB", "DE", "FR", "ES", "IT", "NL", "SE", "NO", "DK", "FI", "PL", "CZ", "AT", "CH", "BE",
    "IE", "PT", "GR", "TR", "RU", "UA", "EG", "IL", "SA", "AE", "IN", "TH", "JP", "CN", "KR", "TW",
    "SG", "AU", "NZ", "BR", "MX", "AR", "CA", "ZA",
];

/// The regions an IANA timezone name starts with.
const TIMEZONE_REGIONS: [&str; 11] = [
    "Africa",
    "America",
    "Antarctica",
    "Arctic",
    "Asia",
    "Atlantic",
    "Australia",
    "Europe",
    "Indian",
    "Pacific",
    "Etc",
];

/// The top-level types a media type can have.
const MEDIA_TYPES: [&str; 9] = [
    "application",
    "text",
    "image",
    "video",
    "audio",
    "font",
    "multipart",
    "model",
    "message",
];

fn is_iana_timezone(value: &str) -> bool {
    if matches!(value, "UTC" | "GMT" | "Z") {
        return true;
    }
    value
        .split_once('/')
        .is_some_and(|(region, rest)| TIMEZONE_REGIONS.contains(&region) && !rest.is_empty())
}

fn has_known_media_type(value: &str) -> bool {
    let without_parameters = value.split(';').next().unwrap_or(value).trim();
    without_parameters
        .split_once('/')
        .is_some_and(|(top, subtype)| MEDIA_TYPES.contains(&top) && !subtype.is_empty())
}

/// The Crockford base32 alphabet, which drops I, L, O and U.
const CROCKFORD_ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

const IMAGE_EXTENSIONS: [&str; 10] = [
    "png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp", "ico", "heic",
];

/// The extension at the end of a value's path, if it has one.
///
/// The query string is cut off first: `/a/b.png?w=64` is a PNG, and reading the
/// extension off the whole string would find `64`.
fn extension_of(value: &str) -> Option<&str> {
    let path = value.split(['?', '#']).next().unwrap_or(value);
    let last_segment = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let extension = last_segment.rsplit_once('.')?.1;
    let usable = !extension.is_empty()
        && extension.len() <= 6
        && extension.chars().all(|c| c.is_ascii_alphanumeric());
    usable.then_some(extension)
}

fn has_extension(value: &str, allowed: [&str; 10]) -> bool {
    extension_of(value)
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| allowed.contains(&extension.as_str()))
}

/// Whether a URL's host is the sort of host images are served from.
fn host_looks_like_a_cdn(value: &str) -> bool {
    let Some((_, rest)) = value.split_once("://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let lowered = host.to_lowercase();
    [
        "cdn", "img", "image", "static", "media", "assets", "photo", "avatar",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

/// Whether a bare number falls inside a window of epoch values.
fn in_epoch_window(value: &str, low: i64, high: i64) -> bool {
    value
        .parse::<i64>()
        .is_ok_and(|parsed| (low..high).contains(&parsed))
}

/// Three base64url segments and nothing else.
fn looks_like_a_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            part.len() >= 8
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
}

/// A short lower-case type prefix, a separator, and an opaque body.
fn looks_like_a_prefixed_id(value: &str) -> bool {
    let Some((prefix, body)) = value.split_once(['_', '-']) else {
        return false;
    };
    let prefix_ok =
        (2..=10).contains(&prefix.len()) && prefix.chars().all(|c| c.is_ascii_alphabetic());
    // The body may itself carry a separator -- `cus_live_9s2Kf3` -- so only its
    // last segment has to be the opaque part.
    let tail = body.rsplit(['_', '-']).next().unwrap_or(body);
    let body_ok = tail.len() >= 8 && tail.chars().all(|c| c.is_ascii_alphanumeric());
    prefix_ok && body_ok
}

fn is_percent_encoded(value: &str) -> bool {
    let bytes: Vec<char> = value.chars().collect();
    bytes.windows(3).any(|window| {
        matches!(window, ['%', first, second] if first.is_ascii_hexdigit() && second.is_ascii_hexdigit())
    })
}

/// Whether a sample stands in for a value rather than being one.
fn is_placeholder(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lowered = trimmed.to_lowercase();
    matches!(
        lowered.as_str(),
        "n/a" | "-" | "--" | "unknown" | "null" | "none" | "tbd" | "not set"
    ) || trimmed.starts_with('*')
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('<') && trimmed.ends_with('>'))
}

#[derive(Clone, Copy)]
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
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    fn named(name: &str) -> Vec<f32> {
        extract(&crate::Field::new(name, &["a"]))
    }

    fn of(name: &str, values: &[&str]) -> Vec<f32> {
        extract(&crate::Field::new(name, values))
    }

    fn feature(name: &str, values: &[&str], dimension: &str) -> f32 {
        let names = feature_names();
        let index = names
            .iter()
            .position(|n| n == dimension)
            .unwrap_or_else(|| panic!("no dimension {dimension}"));
        of(name, values)[index]
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
        assert_eq!(of("", &[]).len(), FEATURE_COUNT);
        assert_eq!(of("id", &["1"]).len(), FEATURE_COUNT);
        assert_eq!(
            of("a_very_long_field_name", &["x"; 32]).len(),
            FEATURE_COUNT
        );
        assert_eq!(feature_names().len(), FEATURE_COUNT);
    }

    #[test]
    fn every_dimension_stays_in_range() {
        for vector in [
            of("", &[]),
            of("user_id", &["1", "2", "3"]),
            of("blob", &[&"x".repeat(10_000)]),
            of("n", &["999999999999999999999"]),
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
        let vector = of("user_email", &[]);
        assert!(
            vector[..NAME_FEATURES].iter().any(|v| *v > 0.0),
            "the name still says something"
        );
        assert!(vector[NAME_FEATURES..].iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_shared_prefix_is_recognised() {
        assert_eq!(
            feature(
                "x",
                &["file_001", "file_002", "file_003"],
                "value.prefix_shared"
            ),
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
