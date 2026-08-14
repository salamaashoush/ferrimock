//! A rendered body must never be able to seize the response.
//!
//! A template renders to text, and that text is inspected to see whether it
//! describes the response (`{status, headers, body}`) or *is* the response.
//! Getting that wrong is not cosmetic: an ordinary payload with a top-level
//! `status` field once hijacked the mock's status code and swallowed its body.
//!
//! The rule this pins: only an object built solely out of envelope keys is an
//! envelope. Anything else is a payload, whatever fields it happens to carry.

#![no_main]

use ferrimock::types::DynamicResponse;
use libfuzzer_sys::fuzz_target;
use serde_json::Value as JsonValue;

const ENVELOPE_KEYS: [&str; 3] = ["status", "headers", "body"];

fuzz_target!(|rendered: String| {
    let response = DynamicResponse::from_rendered_string(rendered.clone());

    let Ok(parsed) = serde_json::from_str::<JsonValue>(rendered.trim()) else {
        // Not JSON, so it can only ever have been a body.
        assert!(
            response.status.is_none() && response.headers.is_none(),
            "a non-JSON render set response fields: {rendered:?}"
        );
        assert_eq!(response.body.as_ref(), rendered.as_bytes());
        return;
    };

    let is_envelope = parsed.as_object().is_some_and(|object| {
        !object.is_empty()
            && object
                .keys()
                .all(|key| ENVELOPE_KEYS.contains(&key.as_str()))
    });

    if !is_envelope {
        assert!(
            response.status.is_none(),
            "a payload dictated the HTTP status: {rendered:?}"
        );
        assert!(
            response.headers.is_none(),
            "a payload dictated response headers: {rendered:?}"
        );
    }
});
