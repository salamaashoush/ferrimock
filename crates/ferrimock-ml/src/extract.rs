//! Pulling reviewable examples out of a real recording.
//!
//! The ship gate will not pass a model measured only on generated data, which
//! makes a corpus of real traffic the bottleneck. This is the shovel: point it
//! at a HAR or a recording session and it produces one example per distinct
//! field, with every value that field was seen holding.
//!
//! What comes out is **unlabelled**. The detector's guess is attached as a
//! suggestion so a reviewer has somewhere to start, but it is written to a
//! separate field and never to `label`. Promoting a suggestion to a label
//! without reading it would recreate exactly the circularity this whole crate
//! exists to avoid -- a corpus labelled by the detector can only ever confirm
//! the detector.

use crate::corpus::{Corpus, Example, Provenance};
use crate::label::FieldLabel;
use ferrimock::recorder::RecordedInteraction;
use ferrimock::type_detector::TypeDetector;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A field seen in a recording, waiting for someone to say what it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    /// Where the field sits in the response, as a JSON pointer.
    pub pointer: String,
    /// The field's own name -- the last segment of the pointer.
    pub field_name: String,
    /// Distinct values it was seen holding.
    pub values: Vec<String>,
    /// How many responses carried it.
    pub occurrences: usize,
    /// What the built-in detector thinks. A starting point for review, never a
    /// label: a corpus labelled this way could only ever confirm the detector.
    pub suggestion: Option<FieldLabel>,
    /// Left empty by extraction. A reviewer fills it in, and only then does the
    /// row become a training example.
    #[serde(default)]
    pub label: Option<FieldLabel>,
}

impl Candidate {
    /// Turn a reviewed candidate into a training example.
    pub fn into_example(self) -> Option<Example> {
        let label = self.label?;
        Some(Example::new(
            self.field_name,
            self.values,
            label,
            Provenance::Reviewed,
        ))
    }
}

/// How extraction is bounded.
#[derive(Debug, Clone, Copy)]
pub struct ExtractOptions {
    /// Distinct values kept per field. Enough to see the shape; not so many that
    /// a review file becomes unreadable.
    pub max_values: usize,
    /// Deepest nesting followed.
    pub max_depth: usize,
    /// Fields seen fewer times than this are dropped as noise.
    pub min_occurrences: usize,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            max_values: 8,
            max_depth: 6,
            min_occurrences: 1,
        }
    }
}

/// Collect every distinct field across a recording's responses.
pub fn from_interactions(
    interactions: &[RecordedInteraction],
    options: &ExtractOptions,
) -> Vec<Candidate> {
    let mut seen: FxHashMap<String, (Vec<String>, usize)> = FxHashMap::default();

    for interaction in interactions {
        let Ok(body) = serde_json::from_str::<JsonValue>(&interaction.response.body) else {
            continue;
        };
        let mut fields: FxHashMap<String, Vec<String>> = FxHashMap::default();
        walk(&body, &mut String::new(), 0, options.max_depth, &mut fields);

        for (pointer, values) in fields {
            let entry = seen.entry(pointer).or_insert_with(|| (Vec::new(), 0));
            entry.1 += 1;
            for value in values {
                if entry.0.len() >= options.max_values {
                    break;
                }
                if !entry.0.contains(&value) {
                    entry.0.push(value);
                }
            }
        }
    }

    let detector = TypeDetector::new();
    let mut candidates: Vec<Candidate> = seen
        .into_iter()
        .filter(|(_, (_, occurrences))| *occurrences >= options.min_occurrences)
        .map(|(pointer, (values, occurrences))| {
            let field_name = pointer
                .rsplit('/')
                .next()
                .unwrap_or(&pointer)
                .to_string();

            let json: Vec<JsonValue> = values
                .iter()
                .map(|value| {
                    serde_json::from_str(value)
                        .ok()
                        .filter(|parsed: &JsonValue| parsed.is_number() || parsed.is_boolean())
                        .unwrap_or_else(|| JsonValue::String(value.clone()))
                })
                .collect();
            let refs: Vec<&JsonValue> = json.iter().collect();
            let (field_type, _) = detector.detect_type(&field_name, &refs);

            Candidate {
                pointer,
                field_name,
                values,
                occurrences,
                suggestion: FieldLabel::from_field_type(&field_type),
                label: None,
            }
        })
        .collect();

    // Most-seen first: a reviewer's time is better spent on the field that
    // appears in every response than on one that appeared once.
    candidates.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.pointer.cmp(&b.pointer))
    });
    candidates
}

/// Every reviewed candidate, as a corpus.
pub fn reviewed_corpus(candidates: Vec<Candidate>) -> Corpus {
    Corpus::new(
        candidates
            .into_iter()
            .filter_map(Candidate::into_example)
            .collect(),
    )
}

/// Collect `pointer -> every value seen at it` within one response.
///
/// A pointer maps to a list rather than a value because an array contributes
/// several: `items[].type` is one field that a single response answers three
/// times, and keeping only the last would hide exactly the variation a reviewer
/// needs to see.
fn walk(
    value: &JsonValue,
    pointer: &mut String,
    depth: usize,
    max_depth: usize,
    out: &mut FxHashMap<String, Vec<String>>,
) {
    if depth > max_depth {
        return;
    }

    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(key);
                walk(child, pointer, depth + 1, max_depth, out);
                pointer.truncate(mark);
            }
        }
        JsonValue::Array(items) => {
            // Elements collapse to one pointer with `[]`: `entries[0].type` and
            // `entries[1].type` are the same field asked twice, and splitting
            // them would flood a review file with duplicates.
            let mark = pointer.len();
            pointer.push_str("[]");
            for item in items.iter().take(4) {
                walk(item, pointer, depth + 1, max_depth, out);
            }
            pointer.truncate(mark);
        }
        JsonValue::Null => {}
        scalar => {
            let rendered = scalar
                .as_str()
                .map_or_else(|| scalar.to_string(), std::string::ToString::to_string);
            let values = out.entry(pointer.clone()).or_default();
            if !values.contains(&rendered) {
                values.push(rendered);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ferrimock::recorder::{RecordedRequest, RecordedResponse};
    use std::time::Duration;

    fn interaction(body: &str) -> RecordedInteraction {
        RecordedInteraction {
            id: "i".to_string(),
            timestamp: Utc::now(),
            request: RecordedRequest {
                method: "GET".to_string(),
                uri: "/v2/things".to_string(),
                query: None,
                headers: vec![],
                body: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: vec![],
                body: body.to_string(),
            },
            duration: Duration::from_millis(1),
        }
    }

    fn extract(bodies: &[&str]) -> Vec<Candidate> {
        let interactions: Vec<RecordedInteraction> =
            bodies.iter().map(|b| interaction(b)).collect();
        from_interactions(&interactions, &ExtractOptions::default())
    }

    #[test]
    fn a_field_collects_the_values_it_was_seen_holding() {
        let candidates = extract(&[
            r#"{"email":"a@b.com"}"#,
            r#"{"email":"c@d.org"}"#,
            r#"{"email":"a@b.com"}"#,
        ]);

        let email = candidates.iter().find(|c| c.pointer == "/email").unwrap();
        assert_eq!(email.occurrences, 3);
        assert_eq!(email.values.len(), 2, "duplicates are not worth reviewing");
        assert_eq!(email.field_name, "email");
    }

    #[test]
    fn extraction_never_writes_a_label() {
        // The whole point: a corpus labelled by the detector can only confirm
        // the detector.
        let candidates = extract(&[r#"{"email":"a@b.com","id":"12345678901"}"#]);
        assert!(candidates.iter().all(|c| c.label.is_none()));
        assert!(
            candidates.iter().any(|c| c.suggestion.is_some()),
            "a reviewer still gets somewhere to start"
        );
    }

    #[test]
    fn only_reviewed_candidates_become_examples() {
        let mut candidates = extract(&[r#"{"email":"a@b.com","other":"x"}"#]);
        assert_eq!(reviewed_corpus(candidates.clone()).len(), 0);

        if let Some(first) = candidates.first_mut() {
            first.label = Some(FieldLabel::Email);
        }
        let corpus = reviewed_corpus(candidates);
        assert_eq!(corpus.len(), 1);
        assert_eq!(
            corpus.examples.first().map(|e| e.provenance),
            Some(Provenance::Reviewed)
        );
    }

    #[test]
    fn list_elements_collapse_to_one_field() {
        let candidates = extract(&[
            r#"{"items":[{"type":"a"},{"type":"b"},{"type":"c"}]}"#,
        ]);

        let types: Vec<&Candidate> = candidates
            .iter()
            .filter(|c| c.field_name == "type")
            .collect();
        assert_eq!(types.len(), 1, "one field, not one per element");
        assert_eq!(types[0].pointer, "/items[]/type");
        assert_eq!(types[0].values.len(), 3);
    }

    #[test]
    fn nested_objects_are_addressed_by_pointer() {
        let candidates = extract(&[r#"{"owner":{"login":"ada"}}"#]);
        assert!(candidates.iter().any(|c| c.pointer == "/owner/login"));
    }

    #[test]
    fn the_most_common_fields_come_first() {
        let candidates = extract(&[
            r#"{"always":"1","sometimes":"x"}"#,
            r#"{"always":"2"}"#,
            r#"{"always":"3"}"#,
        ]);
        assert_eq!(
            candidates.first().map(|c| c.pointer.as_str()),
            Some("/always"),
            "a reviewer's time goes to the field that appears everywhere"
        );
    }

    #[test]
    fn a_non_json_response_is_skipped_rather_than_guessed_at() {
        let candidates = extract(&["not json at all", r#"{"ok":"1"}"#]);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn nulls_carry_nothing_to_review() {
        let candidates = extract(&[r#"{"present":"x","absent":null}"#]);
        assert!(candidates.iter().all(|c| c.pointer != "/absent"));
    }
}
