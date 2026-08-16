//! Decoding a GraphQL request body into an [`async_graphql::Request`].
//!
//! Never `serde_json::from_slice::<Request>`. `serde_json/arbitrary_precision`
//! is force-enabled workspace-wide by rolldown (the `scripting` feature), and
//! under it a *non-integral* JSON number reaches `ConstValue` as a one-key map
//! holding a decimal string (`{"$serde_json::private::Number": "1.5"}`) rather
//! than a number — at any depth, inside lists and input objects too. Integers
//! survive, which is what makes this worth guarding: the failure only shows up
//! once a schema has a `Float` variable, as an argument-validation error that
//! points at the schema instead of at the encoding.
//!
//! `config::parse_har` avoids the same trap the same way: decode to
//! `serde_json::Value` first, then walk it, converting numbers explicitly.

use async_graphql::{Name, Request, Value as GqlValue, Variables};
use serde_json::Value as JsonValue;

/// Parse a GraphQL POST body (`{ query, variables?, operationName? }`).
pub fn parse_request(body: &[u8]) -> crate::Result<Request> {
    let root: JsonValue = serde_json::from_slice(body)
        .map_err(|e| crate::mp_err!("GraphQL request is not valid JSON: {e}"))?;
    request_from_json(&root)
}

/// Build a request from an already-decoded body.
pub fn request_from_json(root: &JsonValue) -> crate::Result<Request> {
    let JsonValue::Object(map) = root else {
        return Err(crate::mp_err!("GraphQL request must be a JSON object"));
    };

    let query = map
        .get("query")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| crate::mp_err!("GraphQL request is missing `query`"))?;

    let mut request = Request::new(query);

    if let Some(name) = map.get("operationName").and_then(JsonValue::as_str) {
        request = request.operation_name(name);
    }

    match map.get("variables") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Object(vars)) => {
            let converted = vars
                .iter()
                .map(|(k, v)| Ok((Name::new(k), to_gql_value(v)?)))
                .collect::<crate::Result<_>>()?;
            request = request.variables(Variables::from_value(GqlValue::Object(converted)));
        }
        Some(_) => return Err(crate::mp_err!("GraphQL `variables` must be an object")),
    }

    Ok(request)
}

/// Convert JSON to a GraphQL value, spelling out numbers rather than letting
/// serde re-derive them from an `arbitrary_precision` map.
fn to_gql_value(value: &JsonValue) -> crate::Result<GqlValue> {
    Ok(match value {
        JsonValue::Null => GqlValue::Null,
        JsonValue::Bool(b) => GqlValue::Boolean(*b),
        JsonValue::String(s) => GqlValue::String(s.clone()),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                GqlValue::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                GqlValue::Number(u.into())
            } else if let Some(f) = n.as_f64() {
                GqlValue::Number(
                    async_graphql::Number::from_f64(f)
                        .ok_or_else(|| crate::mp_err!("Non-finite number in GraphQL variables"))?,
                )
            } else {
                return Err(crate::mp_err!("Unrepresentable number in GraphQL variables"));
            }
        }
        JsonValue::Array(items) => {
            GqlValue::List(items.iter().map(to_gql_value).collect::<crate::Result<_>>()?)
        }
        JsonValue::Object(fields) => GqlValue::Object(
            fields
                .iter()
                .map(|(k, v)| Ok((Name::new(k), to_gql_value(v)?)))
                .collect::<crate::Result<_>>()?,
        ),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The guard exists for floats specifically, and `arbitrary_precision` is
    /// forced on by exactly one thing: the bundler behind `scripting`. If this
    /// ever starts failing because the naive decode produces a number, the
    /// workaround can go.
    #[cfg(feature = "scripting")]
    #[test]
    fn a_naive_decode_still_leaks_arbitrary_precision_floats() {
        let raw = br#"{"query":"{x}","variables":{"v":1.5}}"#;
        let naive: Request = serde_json::from_slice(raw).expect("naive decode succeeds");
        let GqlValue::Object(map) = naive.variables.into_value() else {
            panic!("variables should decode to an object")
        };
        assert!(
            matches!(map.get("v"), Some(GqlValue::Object(_))),
            "a float should still leak as a private-Number map; if it no longer \
             does, delete this module and decode directly"
        );
    }

    /// Holds in every build, with or without `arbitrary_precision`.
    #[test]
    fn numeric_variables_always_decode_to_numbers() {
        let request = parse_request(
            br#"{"query":"query($n: Int!){ x }","variables":{"n":42,"f":1.5,"neg":-7}}"#,
        )
        .unwrap();
        let vars = request.variables.into_value();
        let GqlValue::Object(map) = vars else {
            panic!("variables should decode to an object")
        };
        assert_eq!(map.get("n"), Some(&GqlValue::Number(42.into())));
        assert_eq!(map.get("neg"), Some(&GqlValue::Number((-7).into())));
        assert_eq!(
            map.get("f"),
            Some(&GqlValue::Number(
                async_graphql::Number::from_f64(1.5).unwrap()
            ))
        );
    }

    #[test]
    fn nested_numbers_survive() {
        let request = parse_request(
            br#"{"query":"{x}","variables":{"page":{"first":10,"tags":[1.5,2.5]}}}"#,
        )
        .unwrap();
        let GqlValue::Object(map) = request.variables.into_value() else {
            panic!("variables should decode to an object")
        };
        let GqlValue::Object(page) = map.get("page").unwrap() else {
            panic!("page should be an object")
        };
        assert_eq!(page.get("first"), Some(&GqlValue::Number(10.into())));
        assert_eq!(
            page.get("tags"),
            Some(&GqlValue::List(vec![
                GqlValue::Number(async_graphql::Number::from_f64(1.5).unwrap()),
                GqlValue::Number(async_graphql::Number::from_f64(2.5).unwrap()),
            ]))
        );
    }

    #[test]
    fn operation_name_and_missing_variables_are_optional() {
        let request = parse_request(br#"{"query":"{x}","operationName":"Q"}"#).unwrap();
        assert_eq!(request.operation_name.as_deref(), Some("Q"));
    }

    #[test]
    fn a_body_without_a_query_is_rejected() {
        assert!(parse_request(br#"{"variables":{}}"#).is_err());
    }
}
