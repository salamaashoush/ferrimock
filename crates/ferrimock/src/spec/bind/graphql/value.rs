//! Moving values between JSON (what the store holds) and GraphQL.
//!
//! Both directions convert numbers explicitly. Under the workspace-wide
//! `arbitrary_precision` this is not a style choice: serde re-emits
//! non-integral numbers as a private map, and a response built by letting
//! serde decide would carry that map out to the client.

use async_graphql::{Name, Value as GqlValue};
use serde_json::Value as JsonValue;

/// JSON to GraphQL.
pub fn to_gql(value: &JsonValue) -> GqlValue {
    match value {
        JsonValue::Null => GqlValue::Null,
        JsonValue::Bool(b) => GqlValue::Boolean(*b),
        JsonValue::String(s) => GqlValue::String(s.clone()),
        JsonValue::Number(n) => number_to_gql(n),
        JsonValue::Array(items) => GqlValue::List(items.iter().map(to_gql).collect()),
        JsonValue::Object(fields) => GqlValue::Object(
            fields
                .iter()
                .map(|(k, v)| (Name::new(k), to_gql(v)))
                .collect(),
        ),
    }
}

fn number_to_gql(number: &serde_json::Number) -> GqlValue {
    if let Some(i) = number.as_i64() {
        GqlValue::Number(i.into())
    } else if let Some(u) = number.as_u64() {
        GqlValue::Number(u.into())
    } else if let Some(f) = number.as_f64() {
        async_graphql::Number::from_f64(f).map_or(GqlValue::Null, GqlValue::Number)
    } else {
        GqlValue::Null
    }
}

/// GraphQL to JSON.
pub fn to_json(value: &GqlValue) -> JsonValue {
    match value {
        GqlValue::Null => JsonValue::Null,
        GqlValue::Boolean(b) => JsonValue::Bool(*b),
        GqlValue::String(s) => JsonValue::String(s.clone()),
        GqlValue::Enum(name) => JsonValue::String(name.to_string()),
        GqlValue::Number(n) => n
            .as_i64()
            .map(JsonValue::from)
            .or_else(|| n.as_u64().map(JsonValue::from))
            .or_else(|| {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map(JsonValue::Number)
            })
            .unwrap_or(JsonValue::Null),
        GqlValue::List(items) => JsonValue::Array(items.iter().map(to_json).collect()),
        GqlValue::Object(fields) => JsonValue::Object(
            fields
                .iter()
                .map(|(k, v)| (k.to_string(), to_json(v)))
                .collect(),
        ),
        GqlValue::Binary(bytes) => JsonValue::String(hex_encode(bytes)),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn floats_round_trip_as_numbers() {
        let json = serde_json::json!({ "price": 12.5, "count": 3, "tags": [1.5] });
        let gql = to_gql(&json);
        let GqlValue::Object(map) = &gql else {
            panic!("should be an object")
        };
        assert!(matches!(map.get("price"), Some(GqlValue::Number(_))));
        assert!(matches!(map.get("count"), Some(GqlValue::Number(_))));
        let GqlValue::List(tags) = map.get("tags").unwrap() else {
            panic!("tags should be a list")
        };
        assert!(matches!(tags[0], GqlValue::Number(_)));

        assert_eq!(to_json(&gql), json);
    }

    #[test]
    fn enums_come_back_as_strings() {
        let value = GqlValue::Enum(Name::new("PUBLISHED"));
        assert_eq!(to_json(&value), JsonValue::String("PUBLISHED".into()));
    }

    #[test]
    fn nulls_survive_both_directions() {
        assert_eq!(to_gql(&JsonValue::Null), GqlValue::Null);
        assert_eq!(to_json(&GqlValue::Null), JsonValue::Null);
    }
}
