//! MockResponse namespace bindings: `MockResponse.json()`, `MockResponse.text()`, etc.

use crate::types::{JsHandlerResponse, JsResponseInit};
use napi_derive::napi;
use std::collections::HashMap;

fn merge_headers(
    default: HashMap<String, String>,
    custom: Option<HashMap<String, String>>,
) -> HashMap<String, String> {
    match custom {
        Some(mut h) => {
            for (k, v) in default {
                h.entry(k).or_insert(v);
            }
            h
        }
        None => default,
    }
}

/// Create a JSON response.
///
/// Sets `Content-Type: application/json` automatically.
///
/// @param data - JSON value to serialize as the response body.
/// @param init - Optional status code and headers.
#[napi(namespace = "MockResponse")]
pub fn json(data: serde_json::Value, init: Option<JsResponseInit>) -> JsHandlerResponse {
    let body = serde_json::to_string(&data).unwrap_or_default();
    let default_headers =
        HashMap::from([("content-type".to_string(), "application/json".to_string())]);

    JsHandlerResponse {
        status: init.as_ref().and_then(|i| i.status),
        headers: Some(merge_headers(
            default_headers,
            init.and_then(|i| i.headers),
        )),
        body: Some(body),
        body_json: Some(data),
    }
}

/// Create a plain text response.
///
/// Sets `Content-Type: text/plain` automatically.
///
/// @param body - Text content.
/// @param init - Optional status code and headers.
#[napi(namespace = "MockResponse")]
pub fn text(body: String, init: Option<JsResponseInit>) -> JsHandlerResponse {
    let default_headers =
        HashMap::from([("content-type".to_string(), "text/plain".to_string())]);

    JsHandlerResponse {
        status: init.as_ref().and_then(|i| i.status),
        headers: Some(merge_headers(
            default_headers,
            init.and_then(|i| i.headers),
        )),
        body: Some(body),
        body_json: None,
    }
}

/// Create an HTML response.
///
/// Sets `Content-Type: text/html` automatically.
///
/// @param body - HTML content.
/// @param init - Optional status code and headers.
#[napi(namespace = "MockResponse")]
pub fn html(body: String, init: Option<JsResponseInit>) -> JsHandlerResponse {
    let default_headers =
        HashMap::from([("content-type".to_string(), "text/html".to_string())]);

    JsHandlerResponse {
        status: init.as_ref().and_then(|i| i.status),
        headers: Some(merge_headers(
            default_headers,
            init.and_then(|i| i.headers),
        )),
        body: Some(body),
        body_json: None,
    }
}

/// Create an XML response.
///
/// Sets `Content-Type: application/xml` automatically.
///
/// @param body - XML content.
/// @param init - Optional status code and headers.
#[napi(namespace = "MockResponse")]
pub fn xml(body: String, init: Option<JsResponseInit>) -> JsHandlerResponse {
    let default_headers =
        HashMap::from([("content-type".to_string(), "application/xml".to_string())]);

    JsHandlerResponse {
        status: init.as_ref().and_then(|i| i.status),
        headers: Some(merge_headers(
            default_headers,
            init.and_then(|i| i.headers),
        )),
        body: Some(body),
        body_json: None,
    }
}

/// Create a binary response from a Buffer/ArrayBuffer.
///
/// Sets `Content-Type: application/octet-stream` automatically.
///
/// @param data - Binary data as Buffer.
/// @param init - Optional status code and headers.
#[napi(namespace = "MockResponse")]
pub fn array_buffer(data: napi::bindgen_prelude::Buffer, init: Option<JsResponseInit>) -> JsHandlerResponse {
    let default_headers = HashMap::from([(
        "content-type".to_string(),
        "application/octet-stream".to_string(),
    )]);

    // Convert binary data to string for transport through the string-based body field.
    // Loss-less for valid UTF-8; lossy for binary but sufficient for mock responses.
    let body = String::from_utf8_lossy(data.as_ref()).into_owned();

    JsHandlerResponse {
        status: init.as_ref().and_then(|i| i.status),
        headers: Some(merge_headers(
            default_headers,
            init.and_then(|i| i.headers),
        )),
        body: Some(body),
        body_json: None,
    }
}

/// Create an empty response with just a status code.
///
/// @param status - HTTP status code.
#[napi(namespace = "MockResponse")]
pub fn empty(status: u32) -> JsHandlerResponse {
    JsHandlerResponse {
        status: Some(status),
        headers: None,
        body: None,
        body_json: None,
    }
}

/// Create a network error response.
///
/// When the interceptor sees this response, it throws a `TypeError("Failed to fetch")`
/// to simulate a network failure (DNS error, connection refused, etc.).
#[napi(namespace = "MockResponse")]
pub fn error() -> JsHandlerResponse {
    let headers = HashMap::from([("x-mockpit-network-error".to_string(), "1".to_string())]);
    JsHandlerResponse {
        status: Some(0),
        headers: Some(headers),
        body: None,
        body_json: None,
    }
}
