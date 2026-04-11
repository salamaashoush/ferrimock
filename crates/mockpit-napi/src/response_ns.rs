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
