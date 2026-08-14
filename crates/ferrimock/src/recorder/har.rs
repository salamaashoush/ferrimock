//! HAR format conversion utilities

use super::types::{RecordedInteraction, RecordedRequest, RecordedResponse};
use har::v1_2;

/// Convert RecordedInteraction to HAR Entry
pub fn to_har_entry(interaction: &RecordedInteraction) -> v1_2::Entries {
    // Build request URL (combine uri + query)
    let url = if let Some(ref query) = interaction.request.query {
        format!("{}?{}", interaction.request.uri, query)
    } else {
        interaction.request.uri.clone()
    };

    // Convert headers to HAR format
    let request_headers: Vec<v1_2::Headers> = interaction
        .request
        .headers
        .iter()
        .map(|(name, value): &(String, String)| v1_2::Headers {
            name: name.clone(),
            value: value.clone(),
            comment: None,
        })
        .collect();

    let response_headers: Vec<v1_2::Headers> = interaction
        .response
        .headers
        .iter()
        .map(|(name, value): &(String, String)| v1_2::Headers {
            name: name.clone(),
            value: value.clone(),
            comment: None,
        })
        .collect();

    // Parse query string into HAR format
    let query_string: Vec<v1_2::QueryString> = interaction
        .request
        .query
        .as_ref()
        .map(|q| {
            q.split('&')
                .filter_map(|pair| {
                    let mut parts = pair.splitn(2, '=');
                    let name = parts.next()?;
                    let value = parts.next().unwrap_or("");
                    Some(v1_2::QueryString {
                        name: name.to_string(),
                        value: value.to_string(),
                        comment: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // Calculate body sizes
    let request_body_size = interaction
        .request
        .body
        .as_ref()
        .map_or(0, |b| i64::try_from(b.len()).unwrap_or(i64::MAX));

    let response_body_size = i64::try_from(interaction.response.body.len()).unwrap_or(i64::MAX);

    // Detect if binary data (contains our sentinel string)
    let (response_text, response_encoding) =
        if interaction.response.body.starts_with("<binary data:") {
            (String::new(), Some("base64".to_string()))
        } else {
            (interaction.response.body.clone(), None)
        };

    v1_2::Entries {
        pageref: None,
        started_date_time: interaction.timestamp.to_rfc3339(),
        time: interaction.duration.as_secs_f64() * 1000.0,
        request: v1_2::Request {
            method: interaction.request.method.clone(),
            url,
            http_version: "HTTP/1.1".to_string(),
            cookies: vec![],
            headers: request_headers,
            query_string,
            post_data: interaction
                .request
                .body
                .as_ref()
                .map(|body| v1_2::PostData {
                    mime_type: "application/octet-stream".to_string(),
                    params: None,
                    text: Some(body.clone()),
                    comment: None,
                }),
            headers_size: -1,
            body_size: request_body_size,
            comment: None,
        },
        response: v1_2::Response {
            status: i64::from(interaction.response.status),
            status_text: status_code_to_text(interaction.response.status),
            http_version: "HTTP/1.1".to_string(),
            cookies: vec![],
            headers: response_headers,
            content: v1_2::Content {
                size: response_body_size,
                compression: None,
                mime_type: Some("application/json".to_string()),
                text: Some(response_text),
                encoding: response_encoding,
                comment: None,
            },
            redirect_url: Some(String::new()),
            headers_size: -1,
            body_size: response_body_size,
            comment: None,
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
            wait: interaction.duration.as_secs_f64() * 1000.0,
            receive: 0.0,
            ssl: None,
            comment: None,
        },
        server_ip_address: None,
        connection: None,
        comment: None,
    }
}

/// Convert a HAR entry back into a recorded interaction.
///
/// HAR is the only recording format that keeps the request alongside the
/// response, which makes it the only one a replay can be checked against. The
/// entry index stands in for the interaction id, since HAR carries none.
pub fn from_har_entry(index: usize, entry: &v1_2::Entries) -> RecordedInteraction {
    let (uri, query) = match entry.request.url.split_once('?') {
        Some((uri, query)) => (uri.to_string(), Some(query.to_string())),
        None => (entry.request.url.clone(), None),
    };

    let request_headers = entry
        .request
        .headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();

    let response_headers = entry
        .response
        .headers
        .iter()
        .map(|header| (header.name.clone(), header.value.clone()))
        .collect();

    let timestamp = chrono::DateTime::parse_from_rfc3339(&entry.started_date_time)
        .map_or_else(|_| chrono::Utc::now(), |ts| ts.with_timezone(&chrono::Utc));

    let duration = if entry.time.is_finite() && entry.time > 0.0 {
        std::time::Duration::from_secs_f64(entry.time / 1000.0)
    } else {
        std::time::Duration::ZERO
    };

    RecordedInteraction {
        id: format!("har-{}", index + 1),
        timestamp,
        request: RecordedRequest {
            method: entry.request.method.clone(),
            uri,
            query,
            headers: request_headers,
            body: entry
                .request
                .post_data
                .as_ref()
                .and_then(|post| post.text.clone()),
        },
        response: RecordedResponse {
            status: u16::try_from(entry.response.status).unwrap_or(0),
            headers: response_headers,
            body: entry.response.content.text.clone().unwrap_or_default(),
        },
        duration,
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
