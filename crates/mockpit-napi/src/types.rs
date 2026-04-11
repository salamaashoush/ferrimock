//! NAPI interop types for JS <-> Rust conversion.

use napi_derive::napi;
use std::collections::HashMap;

/// Request context passed to JS handler functions.
///
/// Contains all HTTP request information plus captured path parameters.
#[napi(object)]
#[derive(Clone)]
pub struct JsRequestContext {
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Full URI including query string
    pub uri: String,
    /// Request path (without query string)
    pub path: String,
    /// Path parameter captures from `:param` patterns
    pub params: HashMap<String, String>,
    /// Parsed query parameters
    pub query: HashMap<String, String>,
    /// Request headers
    pub headers: HashMap<String, String>,
    /// Request body as string (if UTF-8)
    pub body: Option<String>,
    /// Request body parsed as JSON (if valid JSON)
    pub body_json: Option<serde_json::Value>,
}

/// Response returned from JS handler functions.
///
/// Return `null`/`undefined` from a handler to signal passthrough.
#[napi(object)]
#[derive(Clone)]
pub struct JsHandlerResponse {
    /// HTTP status code (default: 200)
    pub status: Option<u32>,
    /// Response headers
    pub headers: Option<HashMap<String, String>>,
    /// Response body as string
    pub body: Option<String>,
    /// Response body as JSON (takes precedence over `body` if both set)
    pub body_json: Option<serde_json::Value>,
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

impl JsRequestContext {
    /// Minimal construction -- avoids cloning headers and query for the common case
    /// where handlers only need params and body.
    pub fn from_context_minimal(ctx: &mockpit::types::RequestContext) -> Self {
        Self {
            method: ctx.method.clone(),
            uri: ctx.uri.clone(),
            path: ctx.path.clone(),
            params: ctx.captures.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            // Only include query/headers if non-empty (avoids HashMap allocation)
            query: if ctx.query.is_empty() {
                HashMap::new()
            } else {
                ctx.query.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            },
            headers: if ctx.headers.is_empty() {
                HashMap::new()
            } else {
                ctx.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            },
            body: ctx.body.clone(),
            body_json: ctx.body_json.clone(),
        }
    }
}

impl From<&mockpit::types::RequestContext> for JsRequestContext {
    fn from(ctx: &mockpit::types::RequestContext) -> Self {
        Self::from_context_minimal(ctx)
    }
}

impl From<JsHandlerResponse> for mockpit::types::DynamicResponse {
    fn from(resp: JsHandlerResponse) -> Self {
        let status = resp
            .status
            .and_then(|s| u16::try_from(s).ok())
            .and_then(|s| http::StatusCode::from_u16(s).ok());

        let headers = resp.headers.map(|h| h.into_iter().collect());

        // body_json takes precedence over body
        let body = if let Some(json) = resp.body_json {
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
