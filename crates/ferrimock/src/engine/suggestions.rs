//! What to widen so a corpus covers the requests it missed.
//!
//! A recorded mock matches the one URL it was recorded from, so the next run —
//! a different folder id, a different `format` — misses and the corpus has to
//! be re-recorded. This module reads the two reports the registry already
//! keeps, [`MockRegistry::coverage`](crate::engine::MockRegistry::coverage) and
//! [`MockRegistry::unmatched_requests`](crate::engine::MockRegistry::unmatched_requests),
//! and names the mocks that were one generalization away from serving a miss.
//!
//! Only widenings that are safe to propose are reported: a suggestion is made
//! when a single criterion rejected the request and the mock is otherwise
//! addressing the same endpoint. A request that needs a *new* mock produces no
//! suggestion — `/folder/0/items` is not a widening of `/folder/0`, and saying
//! so would send a reader to edit the wrong file.
//!
//! # Bodies and headers
//!
//! The unmatched log keeps request lines, not payloads, so a mock rejected on a
//! header or body matcher cannot be re-evaluated here and is never suggested
//! against. Post the request line to the inspector for that.

use crate::consolidator::pattern::PatternDetector;
use crate::engine::diagnostics::Criterion;
use crate::engine::matcher::MockMatcher;
use crate::engine::registry::UnmatchedRequest;
use crate::types::{QueryMatchPattern, UrlPattern};
use http::{HeaderMap, Method};
use serde::Serialize;
use std::collections::BTreeMap;

/// How many near misses are considered per unmatched request. A request that
/// is a widening away from something is a widening away from one of the very
/// closest candidates; going deeper only invents edits to unrelated mocks.
const CANDIDATES_PER_REQUEST: usize = 3;

/// The kind of edit being proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SuggestionKind {
    /// Replace a literal URL with a parameterised one, so ids vary freely.
    ParameterizeUrl,
    /// Relax a query matcher that pinned the value it was recorded with.
    RelaxQuery,
    /// Drop the query string a recording baked into the URL pattern, so the
    /// mock matches its endpoint rather than the one call that was captured.
    DropRecordedQuery,
}

impl SuggestionKind {
    /// Lowercase spelling, as used in reports.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ParameterizeUrl => "parameterize-url",
            Self::RelaxQuery => "relax-query",
            Self::DropRecordedQuery => "drop-recorded-query",
        }
    }
}

/// One edit that would let an existing mock serve requests it currently misses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// Mock to edit.
    pub mock_id: String,
    /// File it was loaded from, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// What kind of edit this is.
    pub kind: SuggestionKind,
    /// What the mock declares today.
    pub current: String,
    /// What it would declare instead.
    pub proposed: String,
    /// Request lines the edit would newly serve, most frequent first.
    pub covers: Vec<String>,
    /// Requests the edit would newly serve, counting repeats.
    pub request_count: u64,
}

/// Suggestions for a set of unmatched requests, best first.
///
/// "Best" is the number of requests an edit would recover, so the first entry
/// is the single change that buys the most coverage.
#[must_use]
pub fn suggest(matcher: &MockMatcher, unmatched: &[UnmatchedRequest]) -> Vec<Suggestion> {
    let mut draft: BTreeMap<(String, SuggestionKind, String), DraftSuggestion> = BTreeMap::new();
    let headers = HeaderMap::new();

    for request in unmatched {
        let Ok(method) = request.method.parse::<Method>() else {
            continue;
        };
        let report = matcher.explain(
            &method,
            &request.path,
            request.query.as_deref(),
            &headers,
            None,
        );

        for attempt in report.near_misses(CANDIDATES_PER_REQUEST) {
            // A mock that is switched off is not one edit away from serving
            // anything; re-enabling it is the fix, and it is already reported
            // as disabled in the coverage report.
            if !attempt.enabled {
                continue;
            }
            let Some(mock) = matcher.registry().get_mock(&attempt.mock_id) else {
                continue;
            };

            let failed: Vec<&Criterion> = attempt.failures().map(|o| &o.criterion).collect();
            let Some(proposal) = propose(&failed, &mock, &request.path) else {
                continue;
            };

            let entry = draft
                .entry((
                    attempt.mock_id.clone(),
                    proposal.kind,
                    proposal.proposed.clone(),
                ))
                .or_insert_with(|| DraftSuggestion {
                    source_file: mock.source_file.clone(),
                    current: proposal.current,
                    covers: Vec::new(),
                    request_count: 0,
                });
            entry.covers.push(request_line(request));
            entry.request_count += request.count;

            // One widening per missed request: the closest candidate that can
            // absorb it is the edit to make, and letting a request vote for
            // several mocks would inflate every one of their counts.
            break;
        }
    }

    let mut suggestions: Vec<Suggestion> = draft
        .into_iter()
        .map(|((mock_id, kind, proposed), d)| Suggestion {
            mock_id,
            source_file: d.source_file,
            kind,
            current: d.current,
            proposed,
            covers: d.covers,
            request_count: d.request_count,
        })
        .collect();

    suggestions.sort_by(|a, b| {
        b.request_count
            .cmp(&a.request_count)
            .then_with(|| b.covers.len().cmp(&a.covers.len()))
            .then_with(|| a.mock_id.cmp(&b.mock_id))
    });
    suggestions
}

struct DraftSuggestion {
    source_file: Option<String>,
    current: String,
    covers: Vec<String>,
    request_count: u64,
}

struct Proposal {
    kind: SuggestionKind,
    current: String,
    proposed: String,
}

fn request_line(request: &UnmatchedRequest) -> String {
    match request.query.as_deref() {
        Some(q) if !q.is_empty() => format!("{} {}?{}", request.method, request.path, q),
        _ => format!("{} {}", request.method, request.path),
    }
}

fn propose(
    failed: &[&Criterion],
    mock: &crate::types::MockDefinition,
    path: &str,
) -> Option<Proposal> {
    // Exactly one kind of criterion may have failed. A request rejected on both
    // its url and its query needs two edits, and proposing either alone would
    // not actually make it match.
    if failed.is_empty() {
        return None;
    }
    if failed.iter().any(|c| {
        matches!(
            c,
            Criterion::Method | Criterion::Header(_) | Criterion::Body | Criterion::GraphQl
        )
    }) {
        return None;
    }

    let url_failed = failed.iter().any(|c| matches!(c, Criterion::Url));
    let query_failed: Vec<&str> = failed
        .iter()
        .filter_map(|c| match c {
            Criterion::Query(name) => Some(name.as_str()),
            _ => None,
        })
        .collect();

    match (url_failed, query_failed.is_empty()) {
        (true, true) => propose_parameterized_url(mock, path),
        (false, false) => propose_relaxed_query(mock, &query_failed),
        _ => None,
    }
}

/// Propose a widening when the mock's literal URL and the missed path address
/// the same endpoint.
///
/// Two shapes, in order of how often a recorded corpus produces them: the mock
/// carries the query string it was captured with, or it carries an id that has
/// since changed.
fn propose_parameterized_url(mock: &crate::types::MockDefinition, path: &str) -> Option<Proposal> {
    for pattern in &mock.request.url_patterns {
        let literal = match pattern {
            UrlPattern::Exact(s) => s.as_str(),
            // A prefix or suffix already matches a family; if it still missed,
            // widening the ids is not what was wrong.
            _ => continue,
        };

        // A recording writes the whole request line into the pattern, so a mock
        // for this very endpoint stops matching the moment any parameter moves
        // — a paginating marker, a reordered id list. Serving the endpoint is
        // what the mock was recorded for.
        if let Some((literal_path, _)) = literal.split_once('?')
            && literal_path == path
        {
            return Some(Proposal {
                kind: SuggestionKind::DropRecordedQuery,
                current: literal.to_string(),
                proposed: literal_path.to_string(),
            });
        }

        let detector = PatternDetector::new();
        let target = detector.normalize_path_for_grouping(path);
        // Nothing varied, so the paths genuinely address different endpoints
        // and a parameterised pattern would not bring them together.
        if target == path {
            continue;
        }
        if detector.normalize_path_for_grouping(literal) == target {
            return Some(Proposal {
                kind: SuggestionKind::ParameterizeUrl,
                current: literal.to_string(),
                proposed: to_express_pattern(&target),
            });
        }
    }
    None
}

/// Propose relaxing the query matchers that pinned recorded values.
fn propose_relaxed_query(
    mock: &crate::types::MockDefinition,
    failed_names: &[&str],
) -> Option<Proposal> {
    let mut current = Vec::new();
    let mut proposed = Vec::new();

    for matcher in &mock.request.query_matchers {
        if !failed_names.contains(&matcher.name.as_str()) {
            continue;
        }
        match &matcher.pattern {
            // Only a pinned value is safe to widen. `Absent` failing means the
            // request carried a parameter the mock exists to exclude, and
            // relaxing that would break what it was written for.
            QueryMatchPattern::Exact(value) => {
                current.push(format!("{}={value}", matcher.name));
                proposed.push(format!("{}=<any>", matcher.name));
            }
            QueryMatchPattern::Regex(_)
            | QueryMatchPattern::Present
            | QueryMatchPattern::Absent => {
                return None;
            }
        }
    }

    if proposed.is_empty() {
        return None;
    }
    Some(Proposal {
        kind: SuggestionKind::RelaxQuery,
        current: current.join(", "),
        proposed: proposed.join(", "),
    })
}

/// `/api/users/{id}` -> `/api/users/:id`, the spelling mock files use.
fn to_express_pattern(normalized: &str) -> String {
    let mut out = String::with_capacity(normalized.len());
    let mut rest = normalized;

    // Split rather than index: a placeholder is delimited by braces, and byte
    // offsets into a path that may hold multi-byte characters are not.
    while let Some((before, after_open)) = rest.split_once('{') {
        // An unclosed brace is not a placeholder. Leave the remainder as it
        // was written, including the brace.
        let Some((name, after_close)) = after_open.split_once('}') else {
            break;
        };
        out.push_str(before);
        out.push(':');
        out.push_str(name);
        rest = after_close;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn express_conversion_handles_several_placeholders() {
        assert_eq!(to_express_pattern("/api/users/{id}"), "/api/users/:id");
        assert_eq!(
            to_express_pattern("/orgs/{id1}/users/{id2}"),
            "/orgs/:id1/users/:id2"
        );
        assert_eq!(to_express_pattern("/files/{uuid}/x"), "/files/:uuid/x");
        assert_eq!(to_express_pattern("/api/health"), "/api/health");
    }

    #[test]
    fn express_conversion_leaves_an_unclosed_brace_alone() {
        assert_eq!(to_express_pattern("/api/{broken"), "/api/{broken");
    }
}
