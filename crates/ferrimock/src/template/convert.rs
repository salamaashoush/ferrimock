//! `serde_json::Value` <-> `tera::Value` at the template boundary.
//!
//! Tera 1 re-exported `serde_json::Value`, so this boundary used to be free.
//! Tera 2 has its own value type, so a conversion is unavoidable unless the
//! whole crate adopts `tera::Value` — which would couple every non-template
//! path (scripting, recorder, consolidator) to the template engine.
//!
//! Done by hand rather than with `Value::try_from_serializable`, for two
//! reasons: it skips the serde machinery, and `rolldown_common` (via the
//! `scripting` feature) turns on `serde_json/arbitrary_precision`, which Cargo
//! unifies across the build. Under that feature a `serde_json::Number`
//! serializes as `{"$serde_json::private::Number": "200"}` through any
//! non-serde_json serializer, so the serde bridge would smuggle that marker
//! into rendered output.
//!
//! Prefer [`to_tera`] (moves) over [`to_tera_ref`] (copies strings).

use serde_json::Value as Json;
use tera::Value as Tera;

/// Convert an owned JSON value, moving strings and vectors rather than copying.
pub fn to_tera(value: Json) -> Tera {
    match value {
        Json::Null => Tera::none(),
        Json::Bool(b) => Tera::from(b),
        Json::Number(n) => number(&n),
        Json::String(s) => Tera::from(s),
        Json::Array(items) => {
            let converted: Vec<Tera> = items.into_iter().map(to_tera).collect();
            Tera::from(converted.as_slice())
        }
        Json::Object(fields) => {
            let mut out = tera::value::Map::new();
            for (key, value) in fields {
                out.insert(key.into(), to_tera(value));
            }
            Tera::from(out)
        }
    }
}

/// Borrowed variant, for callers that only hold a reference.
pub fn to_tera_ref(value: &Json) -> Tera {
    match value {
        Json::Null => Tera::none(),
        Json::Bool(b) => Tera::from(*b),
        Json::Number(n) => number(n),
        Json::String(s) => Tera::from(s.as_str()),
        Json::Array(items) => {
            let converted: Vec<Tera> = items.iter().map(to_tera_ref).collect();
            Tera::from(converted.as_slice())
        }
        Json::Object(fields) => {
            let mut out = tera::value::Map::new();
            for (key, value) in fields {
                out.insert(key.clone().into(), to_tera_ref(value));
            }
            Tera::from(out)
        }
    }
}

fn number(n: &serde_json::Number) -> Tera {
    n.as_i64().map_or_else(
        || {
            n.as_u64().map_or_else(
                || n.as_f64().map_or_else(Tera::none, Tera::from),
                Tera::from,
            )
        },
        Tera::from,
    )
}

pub fn to_json(value: &Tera) -> Json {
    if value.is_none() || value.is_undefined() {
        return Json::Null;
    }
    if let Some(b) = value.as_bool() {
        return Json::Bool(b);
    }
    if let Some(s) = value.as_str() {
        return Json::String(s.to_string());
    }
    if let Some(i) = value.as_i64() {
        return Json::from(i);
    }
    if let Some(u) = value.as_u64() {
        return Json::from(u);
    }
    if let Some(f) = value.as_f64() {
        return serde_json::Number::from_f64(f).map_or(Json::Null, Json::Number);
    }
    if let Some(items) = value.as_array() {
        return Json::Array(items.iter().map(to_json).collect());
    }
    if let Some(map) = value.as_map() {
        return Json::Object(
            map.iter()
                .map(|(k, v)| (k.to_string(), to_json(v)))
                .collect(),
        );
    }
    Json::Null
}

/// Tera hands functions their named arguments as `Kwargs`; generators and
/// plugin functions take a plain JSON map.
///
/// Most generators take no arguments at all, so the empty case allocates
/// nothing.
pub fn kwargs_to_args(kwargs: &tera::Kwargs) -> crate::template::fake_data::Args {
    let mut iter = kwargs.iter().peekable();
    if iter.peek().is_none() {
        return crate::template::fake_data::Args::default();
    }
    iter.map(|(key, value)| (key.to_string(), to_json(value)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn numbers_survive_the_round_trip_without_serde_markers() {
        let json = serde_json::json!({
            "status": 200,
            "ratio": 0.5,
            "negative": -7,
            "nested": [1, {"deep": true}],
            "text": "ok",
            "nothing": null,
        });

        let tera = to_tera(json.clone());
        assert_eq!(tera.get_from_path("status").unwrap().as_i64(), Some(200));
        assert!(!format!("{tera:?}").contains("$serde_json::private"));
        assert_eq!(to_json(&tera), json);
        assert_eq!(to_tera_ref(&json), tera);
    }
}
