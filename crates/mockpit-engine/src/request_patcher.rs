//! Request patcher - applies request-side patches before forwarding to upstream

use anyhow::{Context, Result};
use bytes::Bytes;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use mockpit_types::RequestPatch;

/// Applies request-side patches before forwarding to upstream
pub struct RequestPatcher {
    operations: Vec<RequestPatch>,
}

impl RequestPatcher {
    pub fn new(operations: Vec<RequestPatch>) -> Self {
        Self { operations }
    }

    /// Apply all request patches.
    /// Returns modified (headers, body, query_string).
    pub fn apply(
        &self,
        mut headers: HeaderMap,
        body: Option<Bytes>,
        query: Option<&str>,
    ) -> Result<(HeaderMap, Option<Bytes>, Option<String>)> {
        let mut json_body: Option<serde_json::Value> = None;
        let mut raw_body: Option<Bytes> = body;
        let mut query_params: Option<Vec<(String, String)>> = None;

        for op in &self.operations {
            match op {
                RequestPatch::HeaderAdd { name, value } => {
                    let header_name = HeaderName::try_from(name.as_str())
                        .with_context(|| format!("Invalid header name: {name}"))?;
                    let header_value = HeaderValue::try_from(value.as_str())
                        .with_context(|| format!("Invalid header value for {name}: {value}"))?;
                    headers.insert(header_name, header_value);
                }
                RequestPatch::HeaderRemove { name } => {
                    if let Ok(header_name) = HeaderName::try_from(name.as_str()) {
                        headers.remove(header_name);
                    }
                }
                RequestPatch::QueryAdd { name, value } => {
                    let params =
                        query_params.get_or_insert_with(|| parse_query_string(query.unwrap_or("")));
                    // Remove existing key, then add
                    params.retain(|(k, _)| k != name);
                    params.push((name.clone(), value.clone()));
                }
                RequestPatch::QueryRemove { name } => {
                    let params =
                        query_params.get_or_insert_with(|| parse_query_string(query.unwrap_or("")));
                    params.retain(|(k, _)| k != name);
                }
                RequestPatch::JsonPath { path, value } => {
                    let body_val = json_body.get_or_insert_with(|| {
                        raw_body
                            .as_ref()
                            .and_then(|b| serde_json::from_slice(b).ok())
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::default()))
                    });
                    // Use a simple JSONPath setter (navigates dotted paths)
                    set_jsonpath_value(body_val, path, value.clone()).with_context(|| {
                        format!("Failed to set JSONPath '{path}' on request body")
                    })?;
                }
                RequestPatch::JsonPatch(patch) => {
                    let body_val = json_body.get_or_insert_with(|| {
                        raw_body
                            .as_ref()
                            .and_then(|b| serde_json::from_slice(b).ok())
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::default()))
                    });
                    json_patch::patch(body_val, patch)
                        .context("Failed to apply JSON Patch to request body")?;
                }
                RequestPatch::RegexReplace {
                    pattern,
                    replacement,
                } => {
                    // If we have a pending json_body modification, serialize it first
                    if let Some(json_val) = json_body.take() {
                        let serialized = serde_json::to_string(&json_val)
                            .context("Failed to serialize JSON body")?;
                        raw_body = Some(Bytes::from(serialized));
                    }
                    let body_str = raw_body
                        .as_ref()
                        .map(|b| String::from_utf8_lossy(b).into_owned())
                        .unwrap_or_default();
                    let result = pattern.replace_all(&body_str, replacement.as_str());
                    raw_body = Some(Bytes::from(result.into_owned()));
                }
            }
        }

        // Rebuild body from json_body if modified via JSONPath/JsonPatch
        let final_body = if let Some(json_val) = json_body {
            let serialized = serde_json::to_vec(&json_val)
                .context("Failed to serialize patched request body")?;
            // Update Content-Length header
            headers.insert(
                http::header::CONTENT_LENGTH,
                HeaderValue::from(serialized.len()),
            );
            Some(Bytes::from(serialized))
        } else {
            raw_body
        };

        // Rebuild query string if modified
        let final_query = if let Some(params) = query_params {
            if params.is_empty() {
                None
            } else {
                Some(serialize_query_params(&params))
            }
        } else {
            query.map(String::from)
        };

        Ok((headers, final_body, final_query))
    }
}

/// Parse a query string into key-value pairs
fn parse_query_string(query: &str) -> Vec<(String, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            if let Some((key, value)) = pair.split_once('=') {
                (
                    urlencoding::decode(key)
                        .unwrap_or_else(|_| key.into())
                        .into_owned(),
                    urlencoding::decode(value)
                        .unwrap_or_else(|_| value.into())
                        .into_owned(),
                )
            } else {
                (
                    urlencoding::decode(pair)
                        .unwrap_or_else(|_| pair.into())
                        .into_owned(),
                    String::new(),
                )
            }
        })
        .collect()
}

/// Serialize key-value pairs back into a query string
fn serialize_query_params(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                urlencoding::encode(k).into_owned()
            } else {
                format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Simple JSONPath value setter
///
/// Supports paths like:
/// - `$.field` - root-level field
/// - `$.parent.child` - nested field
/// - `$.array[0]` - array index
/// - `$.array[*]` - all array elements (set each)
fn set_jsonpath_value(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<()> {
    // Strip leading "$." if present
    let path = path
        .strip_prefix("$.")
        .unwrap_or(path.strip_prefix('$').unwrap_or(path));

    if path.is_empty() {
        *root = value;
        return Ok(());
    }

    // Split by '.' but handle array notation
    let segments: Vec<&str> = path.split('.').collect();
    set_value_at_path(root, &segments, value)
}

fn set_value_at_path(
    current: &mut serde_json::Value,
    segments: &[&str],
    value: serde_json::Value,
) -> Result<()> {
    if segments.is_empty() {
        *current = value;
        return Ok(());
    }

    let Some(&segment) = segments.first() else {
        *current = value;
        return Ok(());
    };
    let remaining = segments.get(1..).unwrap_or_default();

    // Check for array index notation: field[0], field[*]
    if let Some(bracket_pos) = segment.find('[') {
        let field_name = segment.get(..bracket_pos).unwrap_or_default();
        let index_str = segment
            .get(bracket_pos + 1..segment.len().saturating_sub(1))
            .unwrap_or_default(); // strip [ and ]

        // Navigate to the field first
        let field = if field_name.is_empty() {
            current
        } else {
            current
                .as_object_mut()
                .ok_or_else(|| anyhow::anyhow!("Expected object at '{field_name}'"))?
                .entry(field_name)
                .or_insert(serde_json::Value::Array(vec![]))
        };

        let arr = field
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected array at '{segment}'"))?;

        if index_str == "*" {
            // Apply to all elements
            for item in arr.iter_mut() {
                set_value_at_path(item, remaining, value.clone())?;
            }
        } else {
            let idx: usize = index_str
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid array index: {index_str}"))?;
            if let Some(elem) = arr.get_mut(idx) {
                set_value_at_path(elem, remaining, value)?;
            } else {
                return Err(anyhow::anyhow!(
                    "Array index {} out of bounds (length {})",
                    idx,
                    arr.len()
                ));
            }
        }
    } else {
        // Simple field navigation
        let obj = current
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Expected object at '{segment}'"))?;

        if remaining.is_empty() {
            // Set the value
            obj.insert(segment.to_string(), value);
        } else {
            // Navigate deeper
            let next = obj
                .entry(segment)
                .or_insert(serde_json::Value::Object(serde_json::Map::default()));
            set_value_at_path(next, remaining, value)?;
        }
    }

    Ok(())
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
    use http::HeaderMap;
    use http::header::HeaderValue;

    #[test]
    fn test_header_add() {
        let patcher = RequestPatcher::new(vec![RequestPatch::HeaderAdd {
            name: "x-custom".to_string(),
            value: "test-value".to_string(),
        }]);

        let (headers, _, _) = patcher.apply(HeaderMap::new(), None, None).unwrap();
        assert_eq!(headers.get("x-custom").unwrap(), "test-value");
    }

    #[test]
    fn test_header_remove() {
        let mut headers = HeaderMap::new();
        headers.insert("x-remove-me", HeaderValue::from_static("bye"));
        headers.insert("x-keep-me", HeaderValue::from_static("stay"));

        let patcher = RequestPatcher::new(vec![RequestPatch::HeaderRemove {
            name: "x-remove-me".to_string(),
        }]);

        let (headers, _, _) = patcher.apply(headers, None, None).unwrap();
        assert!(headers.get("x-remove-me").is_none());
        assert_eq!(headers.get("x-keep-me").unwrap(), "stay");
    }

    #[test]
    fn test_query_add() {
        let patcher = RequestPatcher::new(vec![
            RequestPatch::QueryAdd {
                name: "debug".to_string(),
                value: "true".to_string(),
            },
            RequestPatch::QueryAdd {
                name: "source".to_string(),
                value: "mock".to_string(),
            },
        ]);

        let (_, _, query) = patcher
            .apply(HeaderMap::new(), None, Some("existing=yes"))
            .unwrap();
        let query = query.unwrap();
        assert!(query.contains("existing=yes"));
        assert!(query.contains("debug=true"));
        assert!(query.contains("source=mock"));
    }

    #[test]
    fn test_query_remove() {
        let patcher = RequestPatcher::new(vec![RequestPatch::QueryRemove {
            name: "sensitive".to_string(),
        }]);

        let (_, _, query) = patcher
            .apply(
                HeaderMap::new(),
                None,
                Some("keep=yes&sensitive=secret&also_keep=true"),
            )
            .unwrap();
        let query = query.unwrap();
        assert!(query.contains("keep=yes"));
        assert!(!query.contains("sensitive"));
        assert!(query.contains("also_keep=true"));
    }

    #[test]
    fn test_jsonpath_body_patch() {
        let body = Bytes::from(r#"{"user": {"name": "old"}, "count": 0}"#);

        let patcher = RequestPatcher::new(vec![
            RequestPatch::JsonPath {
                path: "$.user.name".to_string(),
                value: serde_json::Value::String("new".to_string()),
            },
            RequestPatch::JsonPath {
                path: "$.count".to_string(),
                value: serde_json::json!(42),
            },
        ]);

        let (_, body, _) = patcher.apply(HeaderMap::new(), Some(body), None).unwrap();
        let body: serde_json::Value = serde_json::from_slice(body.as_ref().unwrap()).unwrap();
        assert_eq!(body["user"]["name"], "new");
        assert_eq!(body["count"], 42);
    }

    #[test]
    fn test_query_string_roundtrip() {
        let params = parse_query_string("foo=bar&baz=qux&empty=");
        assert_eq!(params.len(), 3);
        let serialized = serialize_query_params(&params);
        assert!(serialized.contains("foo=bar"));
        assert!(serialized.contains("baz=qux"));
    }

    #[test]
    fn test_empty_query_string() {
        let params = parse_query_string("");
        assert!(params.is_empty());
    }
}
