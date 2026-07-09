//! HttpResponse namespace bindings: `HttpResponse.json()`, `HttpResponse.text()`, etc.
//!
//! These builders produce the plain `HandlerResponse` shape that crosses
//! the NAPI boundary without a Response round-trip — the fastest path.
//! @mockpit/core exports its own `HttpResponse` (a real `Response`
//! subclass) under the same name and call signatures; handlers written
//! against either work unchanged.

use crate::types::{HandlerResponse, HandlerResponseInit};
use napi_derive::napi;
use std::collections::HashMap;

fn merge_headers(
    default_content_type: Option<&str>,
    custom: Option<HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    match (default_content_type, custom) {
        (Some(ct), Some(mut headers)) => {
            // Case-insensitive default: don't duplicate a caller-provided
            // Content-Type that differs only in casing.
            if !headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("content-type"))
            {
                headers.insert("content-type".to_string(), ct.to_string());
            }
            Some(headers)
        }
        (Some(ct), None) => Some(HashMap::from([(
            "content-type".to_string(),
            ct.to_string(),
        )])),
        (None, custom) => custom,
    }
}

fn build(
    init: Option<HandlerResponseInit>,
    default_content_type: Option<&str>,
    body: Option<String>,
    body_json: Option<serde_json::Value>,
    body_bytes: Option<napi::bindgen_prelude::Uint8Array>,
) -> HandlerResponse {
    let (status, status_text, headers) = match init {
        Some(init) => (init.status, init.status_text, init.headers),
        None => (None, None, None),
    };
    HandlerResponse {
        status,
        status_text,
        headers: merge_headers(default_content_type, headers),
        body,
        body_json,
        body_bytes,
    }
}

/// Create a JSON response.
///
/// Sets `Content-Type: application/json` automatically.
///
/// @param data - JSON value to serialize as the response body.
/// @param init - Optional status code, status text, and headers.
#[napi(namespace = "HttpResponse")]
pub fn json(data: serde_json::Value, init: Option<HandlerResponseInit>) -> HandlerResponse {
    build(init, Some("application/json"), None, Some(data), None)
}

/// Create a plain text response.
///
/// Sets `Content-Type: text/plain` automatically.
#[napi(namespace = "HttpResponse")]
pub fn text(body: String, init: Option<HandlerResponseInit>) -> HandlerResponse {
    build(init, Some("text/plain"), Some(body), None, None)
}

/// Create an HTML response.
///
/// Sets `Content-Type: text/html` automatically.
#[napi(namespace = "HttpResponse")]
pub fn html(body: String, init: Option<HandlerResponseInit>) -> HandlerResponse {
    build(init, Some("text/html"), Some(body), None, None)
}

/// Create an XML response.
///
/// Sets `Content-Type: text/xml` automatically (matches MSW).
#[napi(namespace = "HttpResponse")]
pub fn xml(body: String, init: Option<HandlerResponseInit>) -> HandlerResponse {
    build(init, Some("text/xml"), Some(body), None, None)
}

/// Create a binary response from a Buffer/ArrayBuffer.
///
/// Sets `Content-Type: application/octet-stream` automatically.
/// Binary-safe: bytes pass through untouched.
#[napi(namespace = "HttpResponse")]
pub fn array_buffer(
    data: napi::bindgen_prelude::Uint8Array,
    init: Option<HandlerResponseInit>,
) -> HandlerResponse {
    build(
        init,
        Some("application/octet-stream"),
        None,
        None,
        Some(data),
    )
}

/// Create a redirect response (default 302) with a Location header.
#[napi(namespace = "HttpResponse")]
pub fn redirect(url: String, status: Option<u32>) -> HandlerResponse {
    build(
        Some(HandlerResponseInit {
            status: Some(status.unwrap_or(302)),
            status_text: None,
            headers: Some(HashMap::from([("location".to_string(), url)])),
        }),
        None,
        None,
        None,
        None,
    )
}

/// Create a network error response.
///
/// When the interceptor sees this response, it throws a `TypeError("Failed to fetch")`
/// to simulate a network failure (DNS error, connection refused, etc.).
#[napi(namespace = "HttpResponse")]
pub fn error() -> HandlerResponse {
    build(
        Some(HandlerResponseInit {
            status: Some(0),
            status_text: None,
            headers: Some(HashMap::from([(
                mockpit::types::NETWORK_ERROR_HEADER.to_string(),
                "1".to_string(),
            )])),
        }),
        None,
        None,
        None,
        None,
    )
}
