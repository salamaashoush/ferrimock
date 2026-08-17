//! Reading value hints out of a field's description.
//!
//! A schema says a field is a `String`. Its description often says what kind
//! of string — `(e.g., "text/plain", "image/png")`, `value is always
//! ADD_METADATA`, `one of ACTIVE, INACTIVE`. That is the only place a
//! schema-only pipeline can learn a domain vocabulary, and it is worth mining:
//! in a production schema of 2455 string fields, 1312 carry a description.
//!
//! Everything here is a heuristic on prose, so it errs toward silence: a hint
//! is only taken when the extracted values look like values — short, unbroken
//! tokens — rather than the surrounding sentence.

use lean_string::LeanString;

use crate::type_detector::{FieldType, TimestampFormat};

/// The longest a mined value may be before it reads as prose rather than a
/// value.
const MAX_VALUE_LEN: usize = 48;

/// What a description reveals about a field's values.
#[derive(Debug, Clone, PartialEq)]
pub enum DescriptionHint {
    /// The description states the value outright.
    Constant(LeanString),
    /// The description enumerates the values.
    OneOf(Vec<LeanString>),
    /// The description names a kind of value the detector already knows.
    Semantic(FieldType),
}

/// Mine a description for a value hint.
#[must_use]
pub fn hint(description: &str) -> Option<DescriptionHint> {
    let text = description.trim();
    if text.is_empty() {
        return None;
    }

    if let Some(value) = constant_value(text) {
        return Some(DescriptionHint::Constant(value));
    }
    if let Some(values) = quoted_examples(text) {
        return Some(DescriptionHint::OneOf(values));
    }
    if let Some(values) = enumerated_values(text) {
        return Some(DescriptionHint::OneOf(values));
    }
    // Only the leading sentence describes the field; a later one mentioning a
    // URL is talking about something else. `Folder.id` says "...for the URL
    // https://app.example.com/folders/123 the folder_id is 123" — prose about
    // where to find the id, not a claim that the id is a URL.
    semantic_phrase(leading_sentence(text)).map(DescriptionHint::Semantic)
}

/// The first sentence, which is the one describing the field itself.
fn leading_sentence(text: &str) -> &str {
    text.split_once(". ").map_or(text, |(first, _)| first)
}

/// `value is always ADD_METADATA` — the description states the answer.
fn constant_value(text: &str) -> Option<LeanString> {
    let lowered = text.to_ascii_lowercase();
    let marker = ["value is always ", "always set to ", "is always "]
        .iter()
        .find_map(|marker| lowered.find(marker).map(|at| (at, marker.len())))?;

    let (at, len) = marker;
    let rest = text.get(at + len..)?;
    let token = rest
        .split(['.', ',', ';'])
        .next()?
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`');

    is_value_like(token).then(|| LeanString::from(token))
}

/// `(e.g., "text/plain", "image/png")` — quoted values after an example
/// marker. The quotes are what make this safe: prose is rarely quoted.
fn quoted_examples(text: &str) -> Option<Vec<LeanString>> {
    let lowered = text.to_ascii_lowercase();
    let at = [
        "e.g.",
        "eg.",
        "for example",
        "such as",
        "example:",
        "examples:",
    ]
    .iter()
    .filter_map(|marker| lowered.find(marker))
    .min()?;

    let tail = text.get(at..)?;
    let values: Vec<LeanString> = quoted_runs(tail)
        .into_iter()
        .filter(|value| is_value_like(value))
        .map(LeanString::from)
        .collect();

    (!values.is_empty()).then_some(values)
}

/// Every `"..."` in a fragment, including escaped ones a repaired description
/// carries.
fn quoted_runs(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current: Option<String> = None;
    let mut escaped = false;

    for ch in text.chars() {
        match (ch, &mut current) {
            ('\\', _) => escaped = !escaped,
            ('"', Some(open)) if !escaped => {
                values.push(std::mem::take(open));
                current = None;
            }
            ('"', None) if !escaped => current = Some(String::new()),
            (c, Some(open)) => {
                escaped = false;
                open.push(c);
            }
            _ => escaped = false,
        }
    }
    values
}

/// `one of ACTIVE, INACTIVE` / `possible values: a, b` / `either x or y`.
fn enumerated_values(text: &str) -> Option<Vec<LeanString>> {
    let lowered = text.to_ascii_lowercase();
    let (at, len) = [
        "one of:",
        "one of",
        "possible values:",
        "possible values are",
        "either",
    ]
    .iter()
    .find_map(|marker| lowered.find(marker).map(|at| (at, marker.len())))?;

    let tail = text.get(at + len..)?;
    let tail = tail.split(['.', ';']).next()?.replace(" or ", ",");

    let values: Vec<LeanString> = tail
        .split(',')
        .map(|part| {
            part.trim()
                .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        })
        .filter(|part| is_value_like(part))
        .map(LeanString::from)
        .collect();

    // One value is not an enumeration; it is a sentence that happened to
    // contain the word "either".
    (values.len() > 1).then_some(values)
}

/// A description that names a kind of value rather than listing values.
fn semantic_phrase(text: &str) -> Option<FieldType> {
    let lowered = text.to_ascii_lowercase();
    let has = |needle: &str| lowered.contains(needle);

    if has("mime type") || has("content type") || has("media type") {
        return Some(FieldType::MimeType);
    }
    if has("iso 8601") || has("iso8601") || has("rfc 3339") || has("rfc3339") {
        return Some(FieldType::Timestamp {
            format: TimestampFormat::Rfc3339Utc,
        });
    }
    if has("uuid") || has("guid") {
        return Some(FieldType::Uuid);
    }
    if has("email address") || has("e-mail") {
        return Some(FieldType::Email);
    }
    if has("url") || has("uri") || has("web address") {
        return Some(FieldType::Url);
    }
    if has("ip address") {
        return Some(FieldType::IpAddress);
    }
    if has("phone number") {
        return Some(FieldType::PhoneNumber);
    }
    if has("file name") || has("filename") {
        return Some(FieldType::FileName);
    }
    None
}

/// Whether a mined token reads as a value rather than as prose.
fn is_value_like(token: &str) -> bool {
    if token.is_empty() || token.len() > MAX_VALUE_LEN {
        return false;
    }
    // A value may contain a space ("New York") but not a sentence's worth, and
    // never sentence punctuation.
    if token.split_whitespace().count() > 3 {
        return false;
    }
    !token.contains(['.', ';', ':', '(', ')']) || token.contains('/') || token.contains('-')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn quoted_examples_become_a_vocabulary() {
        let hint = hint(r#"The MIME type of the content (e.g., "text/plain", "image/png")."#);
        let Some(DescriptionHint::OneOf(values)) = hint else {
            panic!("should mine the quoted values, got {hint:?}")
        };
        assert_eq!(values, ["text/plain", "image/png"]);
    }

    #[test]
    fn a_single_quoted_example_still_counts() {
        let Some(DescriptionHint::OneOf(values)) = hint(r#"Source of the session (e.g., "hubs")"#)
        else {
            panic!("should mine one value")
        };
        assert_eq!(values, ["hubs"]);
    }

    #[test]
    fn a_stated_constant_is_taken_literally() {
        let Some(DescriptionHint::Constant(value)) =
            hint("Type of the outcome, value is always ADD_METADATA")
        else {
            panic!("should read the constant")
        };
        assert_eq!(value.as_str(), "ADD_METADATA");
    }

    #[test]
    fn an_enumeration_becomes_a_vocabulary() {
        let Some(DescriptionHint::OneOf(values)) =
            hint("The state, one of ACTIVE, INACTIVE, ERROR")
        else {
            panic!("should read the enumeration")
        };
        assert_eq!(values, ["ACTIVE", "INACTIVE", "ERROR"]);
    }

    #[test]
    fn either_or_is_an_enumeration() {
        let Some(DescriptionHint::OneOf(values)) = hint("Either draft or published") else {
            panic!("should read both branches")
        };
        assert_eq!(values, ["draft", "published"]);
    }

    #[test]
    fn named_kinds_map_to_the_detector() {
        assert_eq!(
            hint("An ISO 8601 timestamp"),
            Some(DescriptionHint::Semantic(FieldType::Timestamp {
                format: TimestampFormat::Rfc3339Utc
            }))
        );
        assert_eq!(
            hint("The URL of the avatar"),
            Some(DescriptionHint::Semantic(FieldType::Url))
        );
    }

    #[test]
    fn prose_yields_nothing_rather_than_a_guess() {
        assert_eq!(hint("The name of the thing"), None);
        assert_eq!(hint(""), None);
        assert_eq!(
            hint("Specifies whether the current user can create workflows for the folder."),
            None
        );
    }

    #[test]
    fn a_sentence_after_e_g_is_not_mistaken_for_a_value() {
        // Nothing quoted and nothing short: there is no value here to take.
        assert_eq!(
            hint(
                "Set this when the caller is trusted, e.g. when the request originates \
                  from an internal service that has already checked permissions"
            ),
            None
        );
    }

    #[test]
    fn one_branch_is_not_an_enumeration() {
        assert_eq!(hint("Either of the two forms is accepted"), None);
    }
}
