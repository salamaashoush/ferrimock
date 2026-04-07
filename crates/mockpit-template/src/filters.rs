//! Tera filters for template processing
//!
//! This module provides custom filters for Tera templates that are NOT already
//! built into Tera. We avoid duplicating Tera's built-in filters.
//!
//! To verify which filters are built-in, run:
//! ```bash
//! cargo run --example verify_tera_filters
//! ```

// Tera library callbacks require std::collections::HashMap - cannot use FxHashMap
#![allow(clippy::disallowed_types)]

use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use tera::{Result as TeraResult, Value as TeraValue, to_value, try_get_value};

// ============================================================================
// BASE64 FILTERS
// ============================================================================

/// Base64 encode a string
///
/// # Example
/// ```text
/// {{ "Hello World" | base64_encode }}
/// ```
pub fn b64encode(value: &TeraValue, _args: &HashMap<String, TeraValue>) -> TeraResult<TeraValue> {
    let s = try_get_value!("b64encode", "value", String, value);
    let encoded = base64::engine::general_purpose::STANDARD.encode(s.as_bytes());
    Ok(to_value(encoded)?)
}

/// Base64 decode a string
///
/// # Example
/// ```text
/// {{ "SGVsbG8gV29ybGQ=" | base64_decode }}
/// ```
pub fn b64decode(value: &TeraValue, _args: &HashMap<String, TeraValue>) -> TeraResult<TeraValue> {
    let s = try_get_value!("b64decode", "value", String, value);
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .map_err(|e| tera::Error::msg(format!("Base64 decode error: {}", e)))?;
    let result = String::from_utf8(decoded)
        .map_err(|e| tera::Error::msg(format!("UTF-8 decode error: {}", e)))?;
    Ok(to_value(result)?)
}

/// URL-safe base64 encode
///
/// # Example
/// ```text
/// {{ "Hello World" | base64_encode_urlsafe }}
/// ```
pub fn b64encode_urlsafe(
    value: &TeraValue,
    _args: &HashMap<String, TeraValue>,
) -> TeraResult<TeraValue> {
    let s = try_get_value!("b64encode_urlsafe", "value", String, value);
    let encoded = base64::engine::general_purpose::URL_SAFE.encode(s.as_bytes());
    Ok(to_value(encoded)?)
}

/// URL-safe base64 decode
///
/// # Example
/// ```text
/// {{ "SGVsbG8gV29ybGQ=" | base64_decode_urlsafe }}
/// ```
pub fn b64decode_urlsafe(
    value: &TeraValue,
    _args: &HashMap<String, TeraValue>,
) -> TeraResult<TeraValue> {
    let s = try_get_value!("b64decode_urlsafe", "value", String, value);
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(s.as_bytes())
        .map_err(|e| tera::Error::msg(format!("Base64 decode error: {}", e)))?;
    let result = String::from_utf8(decoded)
        .map_err(|e| tera::Error::msg(format!("UTF-8 decode error: {}", e)))?;
    Ok(to_value(result)?)
}

// ============================================================================
// JSON FILTERS
// ============================================================================

/// Parse a JSON string into an object
///
/// Note: Tera has built-in `json_encode` but NOT json_decode.
/// This is the logical opposite of `json_encode`.
///
/// # Example
/// ```text
/// {% set data = '{"name": "John"}' | json_decode %}
/// {{ data.name }}
/// ```
pub fn json_parse(value: &TeraValue, _args: &HashMap<String, TeraValue>) -> TeraResult<TeraValue> {
    let s = try_get_value!("json_parse", "value", String, value);
    let parsed: Value = serde_json::from_str(&s)
        .map_err(|e| tera::Error::msg(format!("JSON parse error: {}", e)))?;
    Ok(to_value(parsed)?)
}

// ============================================================================
// URL FILTERS
// ============================================================================

/// URL decode a string
///
/// Note: Tera has built-in `urlencode` but NOT urldecode
///
/// # Example
/// ```text
/// {{ "Hello%20World" | urldecode }}
/// ```
pub fn urldecode(value: &TeraValue, _args: &HashMap<String, TeraValue>) -> TeraResult<TeraValue> {
    let s = try_get_value!("urldecode", "value", String, value);
    let decoded = urlencoding::decode(&s)
        .map_err(|e| tera::Error::msg(format!("URL decode error: {}", e)))?;
    Ok(to_value(decoded.to_string())?)
}

// ============================================================================
// UTILITY FILTERS
// ============================================================================

/// Select a random element from an array
///
/// # Example
/// ```text
/// {{ ["option1", "option2", "option3"] | random_choice }}
/// ```
pub fn random_choice(
    value: &TeraValue,
    _args: &HashMap<String, TeraValue>,
) -> TeraResult<TeraValue> {
    use rand::RngExt;

    // Extract array from input value
    let values = value
        .as_array()
        .ok_or_else(|| tera::Error::msg("random_choice filter requires an array input"))?;

    if values.is_empty() {
        return Err(tera::Error::msg(
            "random_choice filter requires a non-empty array",
        ));
    }

    // Select random element
    let index = rand::rng().random_range(0..values.len());
    Ok(values[index].clone())
}

// ============================================================================
// REGISTRATION HELPER
// ============================================================================

/// Register all custom filters with a Tera instance
///
/// This only registers filters that are NOT already built into Tera.
/// Tera built-ins include: slugify, truncate, title, reverse, split, join,
/// length, default, int, float, round, json_encode, urlencode, and many more.
pub fn register_all_filters(tera: &mut tera::Tera) {
    // Base64 filters - using explicit names aligned with Tera's convention
    tera.register_filter("base64_encode", b64encode);
    tera.register_filter("base64_decode", b64decode);
    tera.register_filter("base64_encode_urlsafe", b64encode_urlsafe);
    tera.register_filter("base64_decode_urlsafe", b64decode_urlsafe);

    // JSON filters - json_decode is the opposite of json_encode (built-in)
    tera.register_filter("json_decode", json_parse);

    // URL filters
    tera.register_filter("urldecode", urldecode);

    // Utility filters
    tera.register_filter("random_choice", random_choice);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_b64encode() {
        let value = to_value("Hello World").expect("Failed to convert string to Tera value");
        let result = b64encode(&value, &HashMap::default()).expect("Failed to base64 encode value");
        assert_eq!(
            result.as_str().expect("Result should be a string"),
            "SGVsbG8gV29ybGQ="
        );
    }

    #[test]
    fn test_b64decode() {
        let value = to_value("SGVsbG8gV29ybGQ=").expect("Failed to convert string to Tera value");
        let result = b64decode(&value, &HashMap::default()).expect("Failed to base64 decode value");
        assert_eq!(
            result.as_str().expect("Result should be a string"),
            "Hello World"
        );
    }

    #[test]
    fn test_json_parse() {
        let value = to_value(r#"{"name":"John","age":30}"#)
            .expect("Failed to convert JSON string to Tera value");
        let result = json_parse(&value, &HashMap::default()).expect("Failed to parse JSON");
        assert_eq!(
            result
                .get("name")
                .expect("name field should exist")
                .as_str()
                .expect("name should be a string"),
            "John"
        );
        assert_eq!(
            result
                .get("age")
                .expect("age field should exist")
                .as_i64()
                .expect("age should be an integer"),
            30
        );
    }

    #[test]
    fn test_urldecode() {
        let value = to_value("Hello%20World").expect("Failed to convert string to Tera value");
        let result = urldecode(&value, &HashMap::default()).expect("Failed to URL decode value");
        assert_eq!(
            result.as_str().expect("Result should be a string"),
            "Hello World"
        );
    }
}
