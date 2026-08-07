//! Emitting a `serde_json::Value` as YAML.
//!
//! `rolldown_common` (via the `scripting` feature) turns on
//! `serde_json/arbitrary_precision`, and Cargo unifies that across the whole
//! build. Under it `serde_json::Number` serializes as
//! `{"$serde_json::private::Number": "..."}` through any serializer that is not
//! serde_json's own — which would put that marker map straight into generated
//! mock files. This wrapper serializes numbers as numbers instead.

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::Value;

/// Wraps a `serde_json::Value` so it serializes as plain data.
pub struct Yamlable<'a>(pub &'a Value);

impl Serialize for Yamlable<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::String(s) => serializer.serialize_str(s),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    serializer.serialize_i64(i)
                } else if let Some(u) = n.as_u64() {
                    serializer.serialize_u64(u)
                } else {
                    serializer.serialize_f64(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            Value::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&Yamlable(item))?;
                }
                seq.end()
            }
            Value::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    map.serialize_entry(key, &Yamlable(value))?;
                }
                map.end()
            }
        }
    }
}

/// Serialize a JSON value as YAML.
pub fn to_yaml(value: &Value) -> Result<String, crate::FerrimockError> {
    serde_yaml_ng::to_string(&Yamlable(value))
        .map_err(|e| crate::FerrimockError::Message(format!("YAML serialization failed: {e}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn numbers_do_not_leak_the_serde_json_marker() {
        let value = serde_json::json!({"status": 200, "ratio": 0.5, "name": "ok"});
        let yaml = to_yaml(&value).unwrap();
        assert!(!yaml.contains("$serde_json::private"), "{yaml}");
        assert!(yaml.contains("status: 200"), "{yaml}");
    }
}
