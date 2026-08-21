//! Behaviour a real service has that a schema does not describe.
//!
//! A document declares shapes and status codes; it does not declare that a
//! second `GET` with an `If-None-Match` answers 304, that a `PUT` against a
//! stale version answers 412, that a removed resource answers 410 rather than
//! 404, or that a create answers with a `Location`. A client that handles all
//! of those has no way to exercise any of them against a mock that does none.
//!
//! Everything here is **opt-in per mount** and everything here is **forced off
//! for replay**. That second rule is not a default, it is a constraint: the
//! fidelity harness replays recorded requests and scores status, shape and
//! value equality, and a 304 or a 412 fails all three — against the
//! unconsolidated baseline as well, so the attribution logic cannot tell a
//! consolidator bug from a mock behaving as designed.

use std::hash::{Hash as _, Hasher as _};

use http::StatusCode;
use serde_json::Value as JsonValue;

use crate::types::RequestContext;

/// The entity tag for one representation.
///
/// A content hash, so it is the same tag for the same bytes on any process and
/// after any restart — which is what makes a conditional request from a client
/// that cached yesterday still work.
#[must_use]
pub fn etag_of(body: &JsonValue) -> String {
    let mut hasher = rustc_hash::FxHasher::default();
    body.to_string().hash(&mut hasher);
    format!("\"{:016x}\"", hasher.finish())
}

/// Whether a tag the request named matches the one the resource has.
///
/// `*` matches anything that exists, which is how a client says "only if it is
/// there" and, on a write, "only if it is not".
#[must_use]
pub fn matches_tag(header: &str, etag: &str) -> bool {
    header
        .split(',')
        .map(str::trim)
        .any(|held| held == "*" || held == etag || held.trim_start_matches("W/") == etag)
}

/// What one request header says, ignoring case the way HTTP does.
#[must_use]
pub fn header<'a>(ctx: &'a RequestContext, name: &str) -> Option<&'a str> {
    ctx.headers
        .iter()
        .find(|(held, _)| held.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// An error the way RFC 9457 writes one.
///
/// The point is the media type and the field names: a client with a generic
/// problem-details reader gets a `title` and a `status` out of it, which it
/// cannot get out of an envelope invented per API.
#[must_use]
pub fn problem(status: StatusCode, detail: &str) -> JsonValue {
    serde_json::json!({
        "type": "about:blank",
        "title": status.canonical_reason().unwrap_or("Error"),
        "status": status.as_u16(),
        "detail": detail,
    })
}

#[cfg(test)]
mod tests;
