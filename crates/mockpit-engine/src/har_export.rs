//! HAR (HTTP Archive) file export from mock definitions
//!
//! Export mock definitions to HAR format for static snapshots.
//! Note: Templates and dynamic features are not preserved in HAR export.

use anyhow::Result;
use har::{Har, Spec, v1_2};
use std::sync::Arc;

use crate::{BodySource, MockDefinition, UrlPattern};

/// Export mocks to HAR format (static snapshots only)
pub fn export_mocks_to_har(mocks: &[Arc<MockDefinition>]) -> Result<Har> {
    let entries: Vec<v1_2::Entries> = mocks
        .iter()
        .enumerate()
        .map(|(idx, mock)| mock_to_har_entry(mock, idx))
        .collect();

    Ok(Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "box-dev-gate".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                comment: Some("Exported from box-dev-gate mock definitions".to_string()),
            },
            browser: None,
            pages: None,
            entries,
            comment: Some(
                "Static snapshot of mock responses. Templates/dynamic features not preserved."
                    .to_string(),
            ),
        }),
    })
}

/// Convert a single mock definition to HAR entry
fn mock_to_har_entry(mock: &MockDefinition, _index: usize) -> v1_2::Entries {
    // Extract URL from first pattern (prefer non-regex patterns)
    let url = mock
        .request
        .url_patterns
        .iter()
        .find_map(|p| match p {
            UrlPattern::Exact(url) => Some(url.clone()),
            UrlPattern::Prefix(url) => Some(url.clone()),
            _ => None,
        })
        .or(
            // Fall back to first pattern's string representation
            mock.request.url_patterns.first().map(|p| match p {
                UrlPattern::Regex(r) => format!("/{}/", r.as_str()),
                _ => String::new(),
            }),
        )
        .unwrap_or_else(|| "https://example.com/mock".to_string());

    // Extract method (use first if multiple)
    let method = mock
        .request
        .methods
        .first()
        .map(|m| m.to_string())
        .unwrap_or_else(|| "GET".to_string());

    // Convert response headers
    let response_headers: Vec<v1_2::Headers> = mock
        .response
        .headers
        .iter()
        .map(|(name, value)| v1_2::Headers {
            name: name.clone(),
            value: value.clone(),
            comment: None,
        })
        .collect();

    // Get response body
    let response_body = match &mock.response.body {
        BodySource::Inline(content) => String::from_utf8(content.to_vec())
            .unwrap_or_else(|_| format!("<binary data: {} bytes>", content.len())),
        BodySource::File(path) => format!("<file: {}>", path.display()),
        BodySource::FileCached(content) => String::from_utf8(content.to_vec())
            .unwrap_or_else(|_| format!("<binary data: {} bytes>", content.len())),
        BodySource::Template { source: tmpl, .. } => format!("<template: {}>", tmpl),
    };

    // Calculate delay in milliseconds
    let delay_ms = mock
        .response
        .delay
        .as_ref()
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);

    v1_2::Entries {
        pageref: None,
        started_date_time: chrono::Utc::now().to_rfc3339(),
        time: delay_ms,
        request: v1_2::Request {
            method,
            url,
            http_version: "HTTP/1.1".to_string(),
            cookies: vec![],
            headers: vec![],
            query_string: vec![],
            post_data: None,
            headers_size: -1,
            body_size: 0,
            comment: Some(format!("Mock ID: {}", mock.id)),
        },
        response: v1_2::Response {
            status: mock.response.status.as_u16() as i64,
            status_text: status_code_to_text(mock.response.status.as_u16()),
            http_version: "HTTP/1.1".to_string(),
            cookies: vec![],
            headers: response_headers,
            content: v1_2::Content {
                size: response_body.len() as i64,
                compression: None,
                mime_type: Some("application/json".to_string()),
                text: Some(response_body),
                encoding: None,
                comment: None,
            },
            redirect_url: Some(String::new()),
            headers_size: -1,
            body_size: 0,
            comment: Some(format!("Priority: {}", mock.priority)),
        },
        cache: v1_2::Cache {
            before_request: None,
            after_request: None,
        },
        timings: v1_2::Timings {
            blocked: None,
            dns: None,
            connect: None,
            send: 0.0,
            wait: delay_ms,
            receive: 0.0,
            ssl: None,
            comment: None,
        },
        server_ip_address: None,
        connection: None,
        comment: Some(format!("Mock: {} (enabled: {})", mock.id, mock.enabled)),
    }
}

/// Convert HTTP status code to status text
fn status_code_to_text(code: u16) -> String {
    match code {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
    .to_string()
}
