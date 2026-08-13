//! Tera filters for template processing
//!
//! This module provides custom filters for Tera templates that are NOT already
//! built into Tera. We avoid duplicating Tera's built-in filters.
//!
//! To verify which filters are built-in, run:
//! ```bash
//! cargo run --example verify_tera_filters
//! ```

use base64::Engine;
use tera::{Kwargs, State, TeraResult, Value as TeraValue};

// ============================================================================
// BASE64 FILTERS
// ============================================================================

/// Base64 encode a string
///
/// # Example
/// ```text
/// {{ "Hello World" | base64_encode }}
/// ```
pub fn b64encode(value: &str, _kwargs: Kwargs, _state: &State<'_>) -> String {
    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
}

/// Base64 decode a string
///
/// # Example
/// ```text
/// {{ "SGVsbG8gV29ybGQ=" | base64_decode }}
/// ```
pub fn b64decode(value: &str, _kwargs: Kwargs, _state: &State<'_>) -> TeraResult<String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value.as_bytes())
        .map_err(|e| tera::Error::message(format!("Base64 decode error: {e}")))?;
    String::from_utf8(decoded).map_err(|e| tera::Error::message(format!("UTF-8 decode error: {e}")))
}

/// URL-safe base64 encode
///
/// # Example
/// ```text
/// {{ "Hello World" | base64_encode_urlsafe }}
/// ```
pub fn b64encode_urlsafe(value: &str, _kwargs: Kwargs, _state: &State<'_>) -> String {
    base64::engine::general_purpose::URL_SAFE.encode(value.as_bytes())
}

/// URL-safe base64 decode
///
/// # Example
/// ```text
/// {{ "SGVsbG8gV29ybGQ=" | base64_decode_urlsafe }}
/// ```
pub fn b64decode_urlsafe(value: &str, _kwargs: Kwargs, _state: &State<'_>) -> TeraResult<String> {
    let decoded = base64::engine::general_purpose::URL_SAFE
        .decode(value.as_bytes())
        .map_err(|e| tera::Error::message(format!("Base64 decode error: {e}")))?;
    String::from_utf8(decoded).map_err(|e| tera::Error::message(format!("UTF-8 decode error: {e}")))
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
pub fn json_parse(value: &str, _kwargs: Kwargs, _state: &State<'_>) -> TeraResult<TeraValue> {
    let parsed: serde_json::Value = serde_json::from_str(value)
        .map_err(|e| tera::Error::message(format!("JSON parse error: {e}")))?;
    Ok(super::convert::to_tera(parsed))
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
pub fn urldecode(value: &str, _kwargs: Kwargs, _state: &State<'_>) -> TeraResult<String> {
    urlencoding::decode(value)
        .map(std::borrow::Cow::into_owned)
        .map_err(|e| tera::Error::message(format!("URL decode error: {e}")))
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
    value: &[TeraValue],
    _kwargs: Kwargs,
    _state: &State<'_>,
) -> TeraResult<TeraValue> {
    use rand::RngExt;

    if value.is_empty() {
        return Err(tera::Error::message(
            "random_choice filter requires a non-empty array",
        ));
    }

    let index = crate::fake_data::rng::rng().random_range(0..value.len());
    value
        .get(index)
        .cloned()
        .ok_or_else(|| tera::Error::message("random_choice failed to select an element"))
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Filters take `&State<'_>`, which only the VM can build, so exercise them
    /// through a rendered template instead of calling them directly.
    fn render(template: &str) -> String {
        let mut tera = tera::Tera::default();
        register_all_filters(&mut tera);
        tera.add_raw_template("t", template)
            .expect("template should compile");
        tera.render("t", &tera::Context::new())
            .expect("template should render")
    }

    #[test]
    fn test_b64encode() {
        assert_eq!(
            render(r#"{{ "Hello World" | base64_encode }}"#),
            "SGVsbG8gV29ybGQ="
        );
    }

    #[test]
    fn test_b64decode() {
        assert_eq!(
            render(r#"{{ "SGVsbG8gV29ybGQ=" | base64_decode }}"#),
            "Hello World"
        );
    }

    #[test]
    fn test_json_parse() {
        assert_eq!(
            render(
                r#"{% set d = '{"name":"John","age":30}' | json_decode %}{{ d.name }}:{{ d.age }}"#
            ),
            "John:30"
        );
    }

    #[test]
    fn test_urldecode() {
        assert_eq!(
            render(r#"{{ "Hello%20World" | urldecode }}"#),
            "Hello World"
        );
    }
}
