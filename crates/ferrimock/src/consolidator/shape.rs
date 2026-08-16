//! Response compatibility: which recordings describe the same kind of answer.
//!
//! Grouping brings candidates together by request shape, which is deliberately
//! loose -- `/v2/files/1` and `/v2/files/999` look identical from the request
//! side. But one of those recorded a file and the other recorded a 404 error,
//! and templating them together produces a mock that answers both wrongly:
//! fields from each shape get invented into the other, and the status of
//! whichever won gets served to both.
//!
//! So a group is partitioned by what its members actually *answered* before any
//! of it is templated. Two responses belong together when they carry the same
//! status and the same structure.

use crate::config::MockConfig;
use serde_json::Value as JsonValue;

/// Deepest nesting level compared before two values are assumed compatible.
/// Real payloads are far shallower; the cap only bounds pathological input.
const MAX_DEPTH: usize = 16;

/// Split a group into subsets whose members answered the same way.
///
/// Subsets keep the input order, and the first subset is the largest, so callers
/// can treat it as the majority case and fall back to exact matches for the
/// rest.
pub fn partition_by_response(group: &[MockConfig]) -> Vec<Vec<MockConfig>> {
    let mut partitions: Vec<Vec<MockConfig>> = Vec::new();

    'next_mock: for mock in group {
        for partition in &mut partitions {
            let Some(representative) = partition.first() else {
                continue;
            };
            if responses_compatible(representative, mock) {
                partition.push(mock.clone());
                continue 'next_mock;
            }
        }
        partitions.push(vec![mock.clone()]);
    }

    // Stable sort by descending size: the majority partition earns the pattern,
    // and equal-sized partitions keep the order they were recorded in.
    partitions.sort_by_key(|partition| std::cmp::Reverse(partition.len()));
    partitions
}

/// Whether two recorded mocks answered the same kind of thing.
pub fn responses_compatible(left: &MockConfig, right: &MockConfig) -> bool {
    let left_status = left
        .response_config
        .as_ref()
        .and_then(crate::config::ResponseConfig::status);
    let right_status = right
        .response_config
        .as_ref()
        .and_then(crate::config::ResponseConfig::status);
    if left_status != right_status {
        return false;
    }

    let left_body = left.response_config.as_ref().and_then(|rc| rc.body());
    let right_body = right.response_config.as_ref().and_then(|rc| rc.body());

    match (left_body, right_body) {
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
        (Some(left_body), Some(right_body)) => {
            let left_json = serde_json::from_str::<JsonValue>(left_body).ok();
            let right_json = serde_json::from_str::<JsonValue>(right_body).ok();
            match (left_json, right_json) {
                (Some(left_json), Some(right_json)) => {
                    values_compatible(&left_json, &right_json, 0, false)
                }
                // Two opaque bodies say nothing to distinguish them; one opaque
                // and one JSON say everything.
                (None, None) => true,
                _ => false,
            }
        }
    }
}

/// Structural compatibility of two JSON values.
///
/// `null` is compatible with anything: a field the API nulls out in one
/// recording is the same field, not a different shape. Integers and floats are
/// one kind here -- JSON numeric drift across recordings of the same endpoint is
/// routine and splitting on it would fragment groups for nothing.
///
/// `listed` marks values reached through an array. Inside a list the rule
/// relaxes: only the first element of each side is compared, and two documents
/// in a search result are not the same document. One carrying a field the other
/// lacks says the field is optional, not that the endpoint answered a different
/// kind of thing -- and splitting a paginated listing on which document happened
/// to be first leaves two mocks claiming one URL, where the second answers
/// nothing.
fn values_compatible(left: &JsonValue, right: &JsonValue, depth: usize, listed: bool) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }

    match (left, right) {
        // Matching scalar kinds agree, and a null on either side agrees with
        // everything -- see the doc comment.
        (JsonValue::Null, _)
        | (_, JsonValue::Null)
        | (JsonValue::Bool(_), JsonValue::Bool(_))
        | (JsonValue::Number(_), JsonValue::Number(_))
        | (JsonValue::String(_), JsonValue::String(_)) => true,
        (JsonValue::Array(left_items), JsonValue::Array(right_items)) => {
            // An empty array carries no element shape to disagree about.
            match (left_items.first(), right_items.first()) {
                (Some(left_item), Some(right_item)) => {
                    values_compatible(left_item, right_item, depth + 1, true)
                }
                _ => true,
            }
        }
        (JsonValue::Object(left_map), JsonValue::Object(right_map)) => {
            // Outside a list, a key one side has and the other lacks is the
            // whole problem: merge them and the template invents that key into
            // responses that never carried it.
            if !listed && left_map.len() != right_map.len() {
                return false;
            }
            left_map.iter().all(|(key, left_value)| {
                match right_map.get(key) {
                    Some(right_value) => {
                        values_compatible(left_value, right_value, depth + 1, listed)
                    }
                    // Only reachable inside a list, where the key counts were
                    // not required to agree in the first place.
                    None => listed,
                }
            })
        }
        _ => false,
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
    use rustc_hash::FxHashMap;

    fn mock(id: &str, url: &str, status: u16, body: &str) -> MockConfig {
        MockConfig {
            id: id.into(),
            description: None,
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec!["GET".to_string()],
                urls: vec![url.to_string()],
                ..Default::default()
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(status),
                headers: FxHashMap::default(),
                body: Some(body.to_string()),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            patch: None,
            delay: None,
            network_error: None,
            sse: None,
            ws: None,
        }
    }

    fn compatible(left: &str, right: &str) -> bool {
        let left: JsonValue = serde_json::from_str(left).unwrap();
        let right: JsonValue = serde_json::from_str(right).unwrap();
        values_compatible(&left, &right, 0, false)
    }

    #[test]
    fn two_pages_of_a_listing_are_the_same_answer() {
        // Documents in a search result differ from one another; comparing the
        // first of one page against the first of another compares two unrelated
        // records. Splitting on that leaves two mocks claiming one URL, and the
        // second answers nothing.
        assert!(compatible(
            r#"{"count":2,"results":[{"id":1,"doc":{"a":1,"b":2}}]}"#,
            r#"{"count":2,"results":[{"id":2,"doc":{"a":1}}]}"#,
        ));
    }

    #[test]
    fn a_field_missing_from_the_top_level_is_still_a_different_answer() {
        // The protection this module exists for: outside a list, a key one side
        // lacks means the template would invent it into responses that never
        // carried it.
        assert!(!compatible(r#"{"id":1,"error":"nope"}"#, r#"{"id":1}"#,));
    }

    #[test]
    fn the_same_shape_with_different_values_is_compatible() {
        assert!(compatible(
            r#"{"id":"1","name":"Ann"}"#,
            r#"{"id":"2","name":"Bob"}"#
        ));
    }

    #[test]
    fn a_key_only_one_side_carries_is_incompatible() {
        assert!(!compatible(r#"{"id":"1"}"#, r#"{"id":"1","extra":true}"#));
    }

    #[test]
    fn a_renamed_key_is_incompatible_even_at_equal_arity() {
        assert!(!compatible(r#"{"a":1}"#, r#"{"b":1}"#));
    }

    #[test]
    fn null_stands_in_for_any_shape() {
        assert!(compatible(r#"{"a":null}"#, r#"{"a":{"deep":1}}"#));
        assert!(compatible(r#"{"a":[1]}"#, r#"{"a":null}"#));
    }

    #[test]
    fn integers_and_floats_are_one_kind() {
        assert!(compatible(r#"{"n":1}"#, r#"{"n":1.5}"#));
    }

    #[test]
    fn an_empty_array_agrees_with_a_populated_one() {
        assert!(compatible(r#"{"items":[]}"#, r#"{"items":[{"id":1}]}"#));
    }

    #[test]
    fn arrays_of_different_element_shapes_are_incompatible() {
        assert!(!compatible(
            r#"{"items":[{"id":1}]}"#,
            r#"{"items":["plain"]}"#
        ));
    }

    #[test]
    fn nesting_is_compared_all_the_way_down() {
        assert!(!compatible(
            r#"{"a":{"b":{"c":1}}}"#,
            r#"{"a":{"b":{"d":1}}}"#
        ));
    }

    #[test]
    fn a_different_status_splits_regardless_of_body() {
        let ok = mock("a", "/x/1", 200, r#"{"id":"1"}"#);
        let created = mock("b", "/x/2", 201, r#"{"id":"2"}"#);
        assert!(!responses_compatible(&ok, &created));
    }

    #[test]
    fn an_error_partition_separates_from_the_resource_partition() {
        let group = vec![
            mock("a", "/files/1", 200, r#"{"type":"file","id":"1"}"#),
            mock("b", "/files/2", 200, r#"{"type":"file","id":"2"}"#),
            mock(
                "c",
                "/files/999",
                404,
                r#"{"type":"error","code":"not_found"}"#,
            ),
            mock("d", "/files/3", 200, r#"{"type":"file","id":"3"}"#),
        ];

        let partitions = partition_by_response(&group);
        assert_eq!(partitions.len(), 2);
        assert_eq!(partitions[0].len(), 3, "the resources are the majority");
        assert_eq!(partitions[1].len(), 1);
        assert_eq!(partitions[1][0].id.as_str(), "c");
    }

    #[test]
    fn a_uniform_group_stays_whole() {
        let group = vec![
            mock("a", "/users/1", 200, r#"{"id":"1"}"#),
            mock("b", "/users/2", 200, r#"{"id":"2"}"#),
            mock("c", "/users/3", 200, r#"{"id":"3"}"#),
        ];
        let partitions = partition_by_response(&group);
        assert_eq!(partitions.len(), 1);
        assert_eq!(partitions[0].len(), 3);
    }

    #[test]
    fn two_api_versions_with_different_payloads_split() {
        let group = vec![
            mock("a", "/api/2/users/1", 200, r#"{"v":2,"id":"1"}"#),
            mock(
                "b",
                "/api/3/users/1",
                200,
                r#"{"v":3,"id":"1","extra":true}"#,
            ),
            mock("c", "/api/2/users/2", 200, r#"{"v":2,"id":"2"}"#),
            mock(
                "d",
                "/api/3/users/2",
                200,
                r#"{"v":3,"id":"2","extra":true}"#,
            ),
        ];
        let partitions = partition_by_response(&group);
        assert_eq!(partitions.len(), 2);
        assert!(partitions.iter().all(|p| p.len() == 2));
    }
}
