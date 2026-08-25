//! Turning a browser's headers into an upstream's, and back.

use http::{HeaderMap, HeaderName, HeaderValue, header};
use std::net::IpAddr;

use super::config::RouteConfig;

/// Headers that describe a single hop and must never cross one.
///
/// RFC 9110 section 7.6.1. Forwarding any of them is wrong on HTTP/1.1 and
/// fatal on HTTP/2, where hyper rejects the message rather than sending a
/// connection-specific field.
const HOP_BY_HOP: [HeaderName; 8] = [
    header::CONNECTION,
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
    HeaderName::from_static("keep-alive"),
];

/// Remove every hop-by-hop header, including the ones `Connection` names.
///
/// `Connection: x-custom` makes `x-custom` hop-by-hop for this message only,
/// so the list has to be read before it is itself removed.
pub fn strip_hop_by_hop(headers: &mut HeaderMap) {
    // Collected first: removing `Connection` while iterating what it names
    // would drop the list out from under the loop.
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|token| HeaderName::try_from(token.trim()).ok())
        .collect();

    for name in &HOP_BY_HOP {
        headers.remove(name);
    }
    for name in named {
        headers.remove(&name);
    }
}

/// Tell the upstream who the browser thinks it is talking to.
///
/// Default: `Host` becomes the target's authority, because a dev server that
/// compares `Host` against `Origin` reads a mismatched pair as cross-origin
/// and refuses. `preserve_host` inverts it and sends the browser's own `Host`,
/// which is what a backend generating absolute URLs needs; `X-Forwarded-Host`
/// and `X-Forwarded-Proto` travel either way so the upstream can always
/// reconstruct the URL the browser actually used.
pub fn apply_forwarding_identity(
    headers: &mut HeaderMap,
    route: &RouteConfig,
    client_host: Option<&HeaderValue>,
    client_scheme: &str,
    client_ip: Option<IpAddr>,
) {
    if let Some(host) = client_host {
        headers.insert(X_FORWARDED_HOST, host.clone());
    }
    if let Ok(scheme) = HeaderValue::from_str(client_scheme) {
        headers.insert(X_FORWARDED_PROTO, scheme);
    }
    if let Some(ip) = client_ip {
        append_forwarded_for(headers, ip);
    }

    if route.preserve_host {
        return;
    }

    // Removing it is not enough to be safe on its own: hyper only fills a
    // missing `Host` from the URI on HTTP/1.1, so it is set explicitly.
    match HeaderValue::from_str(route.target.authority.as_str()) {
        Ok(authority) => {
            headers.insert(header::HOST, authority);
        }
        Err(_) => {
            headers.remove(header::HOST);
        }
    }
}

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// Append this hop to `X-Forwarded-For` rather than replacing it: the header
/// is a trail, and overwriting it hides every proxy before this one.
fn append_forwarded_for(headers: &mut HeaderMap, ip: IpAddr) {
    let existing = headers
        .get(&X_FORWARDED_FOR)
        .and_then(|value| value.to_str().ok());

    let combined = match existing {
        Some(prior) => format!("{prior}, {ip}"),
        None => ip.to_string(),
    };

    if let Ok(value) = HeaderValue::from_str(&combined) {
        headers.insert(X_FORWARDED_FOR, value);
    }
}

/// Ask the upstream for an unencoded body.
///
/// Set only when the proxy is going to read the body: a patch operates on
/// JSON and the recorder stores text, and neither can do anything with gzip.
/// Requests the proxy merely forwards keep the browser's own
/// `Accept-Encoding`, so the compressed bytes pass through untouched and
/// nothing pays to decode them.
pub fn request_identity_encoding(headers: &mut HeaderMap) {
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
}

/// Clean an upstream response for return to the browser.
///
/// Strips hop-by-hop fields, then drops `Content-Length` when the body length
/// is about to change. Framing is hyper's to decide from the body it is
/// handed; a stale length here is how a response truncates.
pub fn clean_response_headers(headers: &mut HeaderMap, body_length_changed: bool) {
    strip_hop_by_hop(headers);
    if body_length_changed {
        headers.remove(header::CONTENT_LENGTH);
        headers.remove(header::CONTENT_ENCODING);
    }
}

/// Whether this response should be allowed to stream frame by frame rather
/// than be collected.
///
/// Content type is the only honest signal available at header time: an
/// `Content-Length` says nothing about how long the body takes to arrive, and
/// an SSE stream that gets collected never delivers a first event.
pub fn is_streaming_response(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| {
            let content_type = content_type.trim_start();
            content_type.starts_with("text/event-stream")
                || content_type.starts_with("application/x-ndjson")
                || content_type.starts_with("application/stream+json")
                || content_type.starts_with("multipart/x-mixed-replace")
                || content_type.starts_with("video/")
                || content_type.starts_with("audio/")
        })
}

/// Whether the request is a WebSocket handshake.
///
/// All three conditions are required: `Connection: Upgrade` alone appears on
/// HTTP/2 cleartext negotiation, and an `Upgrade: websocket` without a key is
/// a client that will fail the handshake anyway.
pub fn is_websocket_upgrade(method: &http::Method, headers: &HeaderMap) -> bool {
    method == http::Method::GET
        && headers
            .get(header::UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        && headers
            .get(header::CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(',')
                    .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
            })
        && headers.contains_key("sec-websocket-key")
}

/// The scheme name a client reached this listener over.
pub fn client_scheme(tls: bool) -> &'static str {
    if tls { "https" } else { "http" }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::proxy::config::{RouteConfig, Target};

    fn route(preserve_host: bool) -> RouteConfig {
        let route = RouteConfig::new("/", Target::parse("http://upstream.test:8080").unwrap());
        if preserve_host {
            route.preserving_host()
        } else {
            route
        }
    }

    #[test]
    fn hop_by_hop_headers_do_not_cross_the_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        headers.insert(header::UPGRADE, HeaderValue::from_static("h2c"));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));

        strip_hop_by_hop(&mut headers);

        assert!(!headers.contains_key(header::CONNECTION));
        assert!(!headers.contains_key(header::TRANSFER_ENCODING));
        assert!(!headers.contains_key(header::UPGRADE));
        assert!(!headers.contains_key("keep-alive"));
        assert!(headers.contains_key(header::CONTENT_TYPE));
    }

    #[test]
    fn a_header_named_by_connection_is_hop_by_hop_too() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("upgrade, x-custom-hop"),
        );
        headers.insert("x-custom-hop", HeaderValue::from_static("1"));
        headers.insert("x-end-to-end", HeaderValue::from_static("1"));

        strip_hop_by_hop(&mut headers);

        assert!(!headers.contains_key("x-custom-hop"));
        assert!(headers.contains_key("x-end-to-end"));
    }

    #[test]
    fn host_becomes_the_targets_authority_by_default() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3010"));

        apply_forwarding_identity(
            &mut headers,
            &route(false),
            Some(&HeaderValue::from_static("localhost:3010")),
            "http",
            None,
        );

        assert_eq!(headers[header::HOST], "upstream.test:8080");
        assert_eq!(headers[&X_FORWARDED_HOST], "localhost:3010");
        assert_eq!(headers[&X_FORWARDED_PROTO], "http");
    }

    #[test]
    fn preserve_host_keeps_the_browsers_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3010"));

        apply_forwarding_identity(
            &mut headers,
            &route(true),
            Some(&HeaderValue::from_static("localhost:3010")),
            "https",
            None,
        );

        assert_eq!(headers[header::HOST], "localhost:3010");
        assert_eq!(headers[&X_FORWARDED_PROTO], "https");
    }

    #[test]
    fn forwarded_for_appends_rather_than_replaces() {
        let mut headers = HeaderMap::new();
        headers.insert(&X_FORWARDED_FOR, HeaderValue::from_static("203.0.113.7"));

        append_forwarded_for(&mut headers, "198.51.100.4".parse().unwrap());

        assert_eq!(headers[&X_FORWARDED_FOR], "203.0.113.7, 198.51.100.4");
    }

    #[test]
    fn a_patched_response_loses_its_stale_length_and_encoding() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("120"));
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
        headers.insert(header::CONNECTION, HeaderValue::from_static("keep-alive"));

        clean_response_headers(&mut headers, true);

        assert!(!headers.contains_key(header::CONTENT_LENGTH));
        assert!(!headers.contains_key(header::CONTENT_ENCODING));
        assert!(!headers.contains_key(header::CONNECTION));
    }

    #[test]
    fn an_untouched_response_keeps_its_length() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("120"));

        clean_response_headers(&mut headers, false);

        assert_eq!(headers[header::CONTENT_LENGTH], "120");
    }

    #[test]
    fn event_streams_are_recognised_as_streaming() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        );
        assert!(is_streaming_response(&headers));

        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        assert!(!is_streaming_response(&headers));
    }

    #[test]
    fn a_websocket_handshake_needs_all_three_signals() {
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        assert!(!is_websocket_upgrade(&http::Method::GET, &headers));

        headers.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        assert!(!is_websocket_upgrade(&http::Method::GET, &headers));

        headers.insert(
            "sec-websocket-key",
            HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
        );
        assert!(is_websocket_upgrade(&http::Method::GET, &headers));
        assert!(!is_websocket_upgrade(&http::Method::POST, &headers));
    }

    #[test]
    fn a_connection_list_with_extra_tokens_still_reads_as_an_upgrade() {
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("keep-alive, Upgrade"),
        );
        headers.insert(
            "sec-websocket-key",
            HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
        );
        assert!(is_websocket_upgrade(&http::Method::GET, &headers));
    }
}
