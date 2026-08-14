//! Telling apart recordings that share a request line but not a request body.
//!
//! A recorded `POST /v2/search` says nothing about *what* was searched for: the
//! request line is identical for every query. Converted naively, three different
//! searches become three identical matchers, and no request can ever select the
//! right one -- the first recording answers all three.
//!
//! Sequencing them with `once` (see `har::sequence_repeated_requests`) is the
//! right answer for a request that genuinely repeated and was answered
//! differently as a session progressed -- a job that is pending and then done.
//! It is the wrong answer here: these are distinct requests that merely share a
//! URL, and replaying them in recording order only works if the app under test
//! happens to ask in the same order.
//!
//! So before falling back to sequencing, look for something in the request body
//! that tells the recordings apart and pin it.

use super::MockConfig;
use super::har::{MatchKey, match_key};
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt::Write;
use serde_json::Value as JsonValue;

/// Longest request body pinned verbatim when no single field discriminates.
const MAX_VERBATIM_BODY: usize = 2048;

/// Deepest JSON nesting searched for a discriminating field. Discriminators
/// deeper than this are unlikely to be the semantic subject of the request, and
/// the search cost grows with every level.
const MAX_DEPTH: usize = 8;

/// Pin the request bodies of recordings that are otherwise indistinguishable.
///
/// `request_bodies` is parallel to `mocks`. Returns how many mocks gained a body
/// matcher.
pub fn discriminate_by_request_body(
    mocks: &mut [MockConfig],
    request_bodies: &[Option<String>],
) -> usize {
    let mut groups: FxHashMap<MatchKey, Vec<usize>> = FxHashMap::default();
    for (index, mock) in mocks.iter().enumerate() {
        if let Some(key) = match_key(mock) {
            groups.entry(key).or_default().push(index);
        }
    }

    let mut pinned = 0;
    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }

        let bodies: Vec<&str> = indices
            .iter()
            .map(|&index| {
                request_bodies
                    .get(index)
                    .and_then(Option::as_deref)
                    .unwrap_or_default()
            })
            .collect();

        let Some(first) = bodies.first() else {
            continue;
        };
        // Identical bodies are genuinely the same request repeated. Leave them
        // to sequencing, which is what that case wants.
        if bodies.iter().all(|body| body == first) {
            continue;
        }

        let Some(matchers) = discriminators(&bodies) else {
            continue;
        };

        for (&index, matcher) in indices.iter().zip(matchers) {
            if let Some(mock) = mocks.get_mut(index)
                && let Some(match_config) = mock.match_config.as_mut()
            {
                match_config.body = matcher;
                pinned += 1;
            }
        }
    }

    pinned
}

/// Build one body matcher per recording, or `None` when nothing tells them apart.
fn discriminators(bodies: &[&str]) -> Option<Vec<FxHashMap<String, JsonValue>>> {
    if let Some(matchers) = discriminate_by_field(bodies) {
        return Some(matchers);
    }
    discriminate_verbatim(bodies)
}

/// Find one JSON field whose value is different in every recording.
///
/// Pinning a single field keeps the mock usable: the app under test only has to
/// send the same search term, not byte-identical JSON with the same key order
/// and whitespace.
fn discriminate_by_field(bodies: &[&str]) -> Option<Vec<FxHashMap<String, JsonValue>>> {
    let parsed: Vec<JsonValue> = bodies
        .iter()
        .map(|body| serde_json::from_str::<JsonValue>(body).ok())
        .collect::<Option<Vec<_>>>()?;

    let flattened: Vec<FxHashMap<String, JsonValue>> =
        parsed.iter().map(flatten_scalars).collect();

    let first = flattened.first()?;
    let mut candidates: Vec<&String> = first
        .keys()
        .filter(|path| flattened.iter().all(|leaves| leaves.contains_key(*path)))
        .collect();

    // Shallowest first, then alphabetical, so the choice is stable across runs
    // and lands on the field a human would have picked.
    candidates.sort_by(|a, b| {
        let depth = |path: &str| path.matches(['.', '[']).count();
        depth(a).cmp(&depth(b)).then_with(|| a.cmp(b))
    });

    for path in candidates {
        let values: Vec<&JsonValue> = flattened
            .iter()
            .filter_map(|leaves| leaves.get(path))
            .collect();
        if values.len() != flattened.len() {
            continue;
        }

        let distinct: FxHashSet<String> = values.iter().map(ToString::to_string).collect();
        if distinct.len() != values.len() {
            continue;
        }

        return Some(
            values
                .into_iter()
                .map(|value| {
                    let mut matcher = FxHashMap::default();
                    matcher.insert(format!("$.{path}"), value.clone());
                    matcher
                })
                .collect(),
        );
    }

    None
}

/// Pin each body verbatim as a substring match.
///
/// The fallback for bodies that are not JSON, or JSON with no single
/// discriminating field. Refuses when one body contains another, since a
/// substring match would then select both.
fn discriminate_verbatim(bodies: &[&str]) -> Option<Vec<FxHashMap<String, JsonValue>>> {
    if bodies
        .iter()
        .any(|body| body.is_empty() || body.len() > MAX_VERBATIM_BODY)
    {
        return None;
    }

    for (index, body) in bodies.iter().enumerate() {
        for (other_index, other) in bodies.iter().enumerate() {
            if index != other_index && other.contains(*body) {
                return None;
            }
        }
    }

    Some(
        bodies
            .iter()
            .map(|body| {
                let mut matcher = FxHashMap::default();
                matcher.insert(format!("@{body}"), JsonValue::Bool(true));
                matcher
            })
            .collect(),
    )
}

/// Flatten a JSON value to `jsonpath -> scalar`, in the dialect
/// [`crate::types::json_path_lookup`] reads (`a.b[0].c`).
fn flatten_scalars(value: &JsonValue) -> FxHashMap<String, JsonValue> {
    let mut out = FxHashMap::default();
    collect(value, &mut String::new(), 0, &mut out);
    out
}

fn collect(
    value: &JsonValue,
    path: &mut String,
    depth: usize,
    out: &mut FxHashMap<String, JsonValue>,
) {
    if depth > MAX_DEPTH {
        return;
    }

    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                // A key carrying path punctuation cannot be addressed in this
                // JSONPath dialect, so it can never be pinned.
                if key.contains(['.', '[', ']']) {
                    continue;
                }
                let mark = path.len();
                if !path.is_empty() {
                    path.push('.');
                }
                path.push_str(key);
                collect(child, path, depth + 1, out);
                path.truncate(mark);
            }
        }
        JsonValue::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let mark = path.len();
                let _ = write!(path, "[{index}]");
                collect(child, path, depth + 1, out);
                path.truncate(mark);
            }
        }
        JsonValue::Null => {}
        scalar => {
            if !path.is_empty() {
                out.insert(path.clone(), scalar.clone());
            }
        }
    }
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
    use crate::config::{MatchConfig, ReturnConfig};

    fn post(id: &str, url: &str, body: &str) -> (MockConfig, Option<String>) {
        (
            MockConfig {
                id: id.into(),
                description: None,
                priority: 100,
                enabled: true,
                once: false,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec![format!("exact:{url}")],
                    ..Default::default()
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(200),
                    headers: FxHashMap::default(),
                    body: Some("{}".to_string()),
                    template: None,
                    file: None,
                    template_file: None,
                    json: Box::new(JsonValue::Null),
                }),
                patch: None,
                delay: None,
                network_error: None,
                sse: None,
                ws: None,
            },
            Some(body.to_string()),
        )
    }

    fn run(pairs: Vec<(MockConfig, Option<String>)>) -> (Vec<MockConfig>, usize) {
        let (mut mocks, bodies): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        let pinned = discriminate_by_request_body(&mut mocks, &bodies);
        (mocks, pinned)
    }

    fn body_matcher(mock: &MockConfig) -> Vec<(String, JsonValue)> {
        let mut entries: Vec<(String, JsonValue)> = mock
            .match_config
            .as_ref()
            .map(|m| {
                m.body
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    #[test]
    fn a_single_differing_field_becomes_the_discriminator() {
        let (mocks, pinned) = run(vec![
            post("a", "/v2/search", r#"{"query":"invoices","limit":10}"#),
            post("b", "/v2/search", r#"{"query":"contracts","limit":10}"#),
            post("c", "/v2/search", r#"{"query":"receipts","limit":10}"#),
        ]);

        assert_eq!(pinned, 3);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("$.query".to_string(), JsonValue::from("invoices"))]
        );
        assert_eq!(
            body_matcher(&mocks[2]),
            vec![("$.query".to_string(), JsonValue::from("receipts"))]
        );
    }

    #[test]
    fn a_field_shared_by_every_recording_is_not_a_discriminator() {
        // `limit` is identical everywhere, so it cannot select between them.
        let (mocks, _) = run(vec![
            post("a", "/s", r#"{"query":"x","limit":10}"#),
            post("b", "/s", r#"{"query":"y","limit":10}"#),
        ]);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("$.query".to_string(), JsonValue::from("x"))]
        );
    }

    #[test]
    fn a_nested_field_is_addressed_in_the_matcher_dialect() {
        let (mocks, pinned) = run(vec![
            post("a", "/s", r#"{"filter":{"kind":"file"}}"#),
            post("b", "/s", r#"{"filter":{"kind":"folder"}}"#),
        ]);
        assert_eq!(pinned, 2);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("$.filter.kind".to_string(), JsonValue::from("file"))]
        );
    }

    #[test]
    fn the_shallowest_discriminator_wins() {
        let (mocks, _) = run(vec![
            post("a", "/s", r#"{"q":"x","deep":{"also":"1"}}"#),
            post("b", "/s", r#"{"q":"y","deep":{"also":"2"}}"#),
        ]);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("$.q".to_string(), JsonValue::from("x"))]
        );
    }

    #[test]
    fn identical_bodies_are_left_to_sequencing() {
        let (mocks, pinned) = run(vec![
            post("a", "/s", r#"{"query":"x"}"#),
            post("b", "/s", r#"{"query":"x"}"#),
        ]);
        assert_eq!(pinned, 0);
        assert!(body_matcher(&mocks[0]).is_empty());
    }

    #[test]
    fn non_json_bodies_fall_back_to_a_verbatim_match() {
        let (mocks, pinned) = run(vec![
            post("a", "/s", "term=invoices&page=1"),
            post("b", "/s", "term=contracts&page=1"),
        ]);
        assert_eq!(pinned, 2);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("@term=invoices&page=1".to_string(), JsonValue::Bool(true))]
        );
    }

    #[test]
    fn a_body_contained_in_another_is_refused_rather_than_mismatched() {
        // "a=1" is a substring of "a=1&b=2", so a contains-match would select
        // both and answering either would be a coin flip.
        let (mocks, pinned) = run(vec![post("a", "/s", "a=1"), post("b", "/s", "a=1&b=2")]);
        assert_eq!(pinned, 0);
        assert!(body_matcher(&mocks[0]).is_empty());
    }

    #[test]
    fn recordings_on_different_urls_are_never_compared() {
        let (mocks, pinned) = run(vec![
            post("a", "/one", r#"{"query":"x"}"#),
            post("b", "/two", r#"{"query":"y"}"#),
        ]);
        assert_eq!(pinned, 0);
        assert!(body_matcher(&mocks[0]).is_empty());
    }

    #[test]
    fn repeated_values_across_recordings_disqualify_a_field() {
        // `kind` repeats, so it cannot single any recording out; `id` can.
        let (mocks, _) = run(vec![
            post("a", "/s", r#"{"kind":"file","id":"1"}"#),
            post("b", "/s", r#"{"kind":"file","id":"2"}"#),
            post("c", "/s", r#"{"kind":"folder","id":"3"}"#),
        ]);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("$.id".to_string(), JsonValue::from("1"))]
        );
    }

    #[test]
    fn a_key_carrying_path_punctuation_is_never_chosen() {
        // `a.b` would parse as two segments and pin the wrong thing.
        let (mocks, pinned) = run(vec![
            post("a", "/s", r#"{"a.b":"1","ok":"x"}"#),
            post("b", "/s", r#"{"a.b":"2","ok":"y"}"#),
        ]);
        assert_eq!(pinned, 2);
        assert_eq!(
            body_matcher(&mocks[0]),
            vec![("$.ok".to_string(), JsonValue::from("x"))]
        );
    }
}
