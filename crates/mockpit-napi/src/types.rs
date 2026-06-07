//! NAPI interop types for JS <-> Rust conversion.

use napi_derive::napi;
use std::collections::HashMap;

/// Response returned from JS handler functions.
///
/// Return `null`/`undefined` from a handler to signal passthrough.
#[napi(object)]
pub struct JsHandlerResponse {
    /// HTTP status code (default: 200)
    pub status: Option<u32>,
    /// Response headers
    pub headers: Option<HashMap<String, String>>,
    /// Response body as string
    pub body: Option<String>,
    /// Response body as JSON (takes precedence over `body` if both set)
    pub body_json: Option<serde_json::Value>,
    /// Raw binary response body (takes precedence over `body`/`body_json`).
    /// Set by `MockResponse.arrayBuffer()` for binary-safe responses.
    pub body_bytes: Option<napi::bindgen_prelude::Uint8Array>,
}

/// Options for response construction.
#[napi(object)]
#[derive(Clone)]
pub struct JsResponseInit {
    /// HTTP status code
    pub status: Option<u32>,
    /// Response headers
    pub headers: Option<HashMap<String, String>>,
}


impl From<JsHandlerResponse> for mockpit::types::DynamicResponse {
    fn from(resp: JsHandlerResponse) -> Self {
        let status = resp
            .status
            .and_then(|s| u16::try_from(s).ok())
            .and_then(|s| http::StatusCode::from_u16(s).ok());

        let headers = resp.headers.map(|h| h.into_iter().collect());

        // Precedence: raw bytes (binary-safe) > body_json > body string.
        let body = if let Some(bytes) = resp.body_bytes {
            bytes::Bytes::from(bytes.to_vec())
        } else if let Some(json) = resp.body_json {
            bytes::Bytes::from(serde_json::to_vec(&json).unwrap_or_default())
        } else if let Some(text) = resp.body {
            bytes::Bytes::from(text)
        } else {
            bytes::Bytes::new()
        };

        mockpit::types::DynamicResponse {
            status,
            headers,
            body,
        }
    }
}
