//! Why a request matched — or didn't.
//!
//! [`MockMatcher::explain`] re-runs a request against every mock, criterion by
//! criterion, and reports which ones passed. Unmatched requests get a ranked
//! list of near misses: the mocks that came closest and the exact criterion
//! that rejected them.
//!
//! Every criterion is evaluated through the same predicates the matcher uses on
//! the hot path, so an explanation can never disagree with a real match.

use crate::types::{
    BodyMatcher, GraphQLMatcher, HeaderMatchPattern, MockDefinition, QueryMatchPattern, UrlPattern,
};
use http::{HeaderMap, Method};
use std::fmt;
use std::sync::Arc;

use super::matcher::MockMatcher;

/// Longest body/value excerpt quoted in an outcome before truncation.
const MAX_EXCERPT: usize = 120;

/// A single matching criterion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Criterion {
    /// HTTP method.
    Method,
    /// URL pattern.
    Url,
    /// One header matcher, by header name.
    Header(String),
    /// One query parameter matcher, by parameter name.
    Query(String),
    /// Request body matcher.
    Body,
    /// GraphQL operation matcher.
    GraphQl,
}

impl fmt::Display for Criterion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Method => f.write_str("method"),
            Self::Url => f.write_str("url"),
            Self::Header(name) => write!(f, "header {name}"),
            Self::Query(name) => write!(f, "query {name}"),
            Self::Body => f.write_str("body"),
            Self::GraphQl => f.write_str("graphql"),
        }
    }
}

/// The verdict on one criterion, with both sides of the comparison so a reader
/// can see the difference without re-deriving it.
#[derive(Debug, Clone)]
pub struct CriterionOutcome {
    /// Which criterion this is.
    pub criterion: Criterion,
    /// Whether the request satisfied it.
    pub passed: bool,
    /// What the mock requires.
    pub expected: String,
    /// What the request carried.
    pub actual: String,
}

impl fmt::Display for CriterionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.passed {
            write!(f, "{} matches {}", self.criterion, self.expected)
        } else {
            write!(
                f,
                "{} expected {}, got {}",
                self.criterion, self.expected, self.actual
            )
        }
    }
}

/// One mock evaluated against the request.
#[derive(Debug, Clone)]
pub struct MatchAttempt {
    /// Mock id.
    pub mock_id: String,
    /// Match priority (higher wins).
    pub priority: u32,
    /// Whether the mock is currently enabled. A disabled mock never matches,
    /// however well its criteria line up — a consumed `once` mock lands here.
    pub enabled: bool,
    /// Every criterion the mock declares, in evaluation order.
    pub outcomes: Vec<CriterionOutcome>,
    /// URL captures, populated only when every criterion passed.
    pub captures: rustc_hash::FxHashMap<String, String>,
}

impl MatchAttempt {
    /// Whether this mock would serve the request.
    #[must_use]
    pub fn matched(&self) -> bool {
        self.enabled && self.outcomes.iter().all(|o| o.passed)
    }

    /// How many criteria the request satisfied.
    #[must_use]
    pub fn passed_count(&self) -> usize {
        self.outcomes.iter().filter(|o| o.passed).count()
    }

    /// The criteria that rejected the request.
    pub fn failures(&self) -> impl Iterator<Item = &CriterionOutcome> {
        self.outcomes.iter().filter(|o| !o.passed)
    }
}

impl fmt::Display for MatchAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (priority {})", self.mock_id, self.priority)?;
        if !self.enabled {
            f.write_str(" [disabled]")?;
        }
        let failures: Vec<String> = self.failures().map(ToString::to_string).collect();
        if failures.is_empty() {
            if self.enabled {
                f.write_str(": all criteria match")
            } else {
                f.write_str(": all criteria match but the mock is disabled")
            }
        } else {
            write!(f, ": {}", failures.join("; "))
        }
    }
}

/// The full evaluation of one request against the registry.
#[derive(Debug, Clone)]
pub struct MatchReport {
    /// The request line, for rendering.
    pub request: String,
    /// Every mock considered, in the order the matcher would consider them.
    pub attempts: Vec<MatchAttempt>,
}

impl MatchReport {
    /// The mock that serves this request, if any.
    #[must_use]
    pub fn matched(&self) -> Option<&MatchAttempt> {
        self.attempts.iter().find(|a| a.matched())
    }

    /// The mocks that came closest, best first: fewest failed criteria, then
    /// most satisfied, then highest priority. Only non-matching mocks appear.
    #[must_use]
    pub fn near_misses(&self, limit: usize) -> Vec<&MatchAttempt> {
        let mut candidates: Vec<&MatchAttempt> =
            self.attempts.iter().filter(|a| !a.matched()).collect();
        candidates.sort_by_key(|a| {
            (
                a.failures().count(),
                std::cmp::Reverse(a.passed_count()),
                std::cmp::Reverse(a.priority),
            )
        });
        candidates.truncate(limit);
        candidates
    }

    /// A one-line explanation suitable for a log or a 404 body.
    #[must_use]
    pub fn summary(&self) -> String {
        if let Some(matched) = self.matched() {
            return format!("{} matched mock {}", self.request, matched.mock_id);
        }
        if self.attempts.is_empty() {
            return format!("{} matched no mock (registry is empty)", self.request);
        }
        match self.near_misses(1).first() {
            Some(closest) => format!("{} matched no mock; closest: {closest}", self.request),
            None => format!("{} matched no mock", self.request),
        }
    }
}

fn excerpt(value: &str) -> String {
    let trimmed: String = value.chars().take(MAX_EXCERPT).collect();
    if trimmed.len() < value.len() {
        format!("{trimmed}…")
    } else {
        trimmed
    }
}

fn quote(value: &str) -> String {
    format!("\"{}\"", excerpt(value))
}

fn describe_url_pattern(pattern: &UrlPattern) -> String {
    match pattern {
        UrlPattern::Exact(s) => format!("exact {}", quote(s)),
        UrlPattern::Prefix(s) => format!("prefix {}", quote(s)),
        UrlPattern::Suffix(s) => format!("suffix {}", quote(s)),
        UrlPattern::Regex(re) | UrlPattern::HrefRegex(re) => {
            format!("regex {}", quote(re.as_str()))
        }
        UrlPattern::Glob(g) => format!("glob {}", quote(g.glob().glob())),
    }
}

fn describe_header_pattern(pattern: &HeaderMatchPattern) -> String {
    match pattern {
        HeaderMatchPattern::Exact(value) => quote(value),
        HeaderMatchPattern::Regex(re) => format!("regex {}", quote(re.as_str())),
        HeaderMatchPattern::Present => "any value".to_string(),
        HeaderMatchPattern::Absent => "no value".to_string(),
    }
}

fn describe_query_pattern(pattern: &QueryMatchPattern) -> String {
    match pattern {
        QueryMatchPattern::Exact(value) => quote(value),
        QueryMatchPattern::Regex(re) => format!("regex {}", quote(re.as_str())),
        QueryMatchPattern::Present => "any value".to_string(),
        QueryMatchPattern::Absent => "no value".to_string(),
    }
}

fn describe_body_matcher(matcher: &BodyMatcher) -> String {
    match matcher {
        BodyMatcher::Contains(s) => format!("contains {}", quote(s)),
        BodyMatcher::Regex(re) => format!("regex {}", quote(re.as_str())),
        BodyMatcher::JsonPath { path, value } => {
            format!(
                "json path {} = {}",
                quote(path),
                excerpt(&value.to_string())
            )
        }
        BodyMatcher::JsonEquals(value) => format!("json equals {}", excerpt(&value.to_string())),
    }
}

fn describe_graphql_matcher(matcher: &GraphQLMatcher) -> String {
    if matcher.match_any {
        return "any operation".to_string();
    }
    let mut parts = Vec::new();
    if let Some(name) = &matcher.operation_name {
        parts.push(format!("operation {}", quote(name)));
    }
    if let Some(re) = &matcher.operation_name_regex {
        parts.push(format!("operation regex {}", quote(re.as_str())));
    }
    if let Some(kind) = matcher.operation_type {
        parts.push(format!("type {kind:?}").to_lowercase());
    }
    if !matcher.variable_matchers.is_empty() {
        let mut names: Vec<&str> = matcher
            .variable_matchers
            .keys()
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        parts.push(format!("variables {}", names.join(", ")));
    }
    if matcher.introspection_matcher.is_some() {
        parts.push("introspection".to_string());
    }
    if parts.is_empty() {
        "any operation".to_string()
    } else {
        parts.join(" + ")
    }
}

fn header_value(headers: &HeaderMap, name: &http::HeaderName) -> String {
    headers.get(name).map_or_else(
        || "(absent)".to_string(),
        |value| {
            value
                .to_str()
                .map_or_else(|_| "(non-utf8)".to_string(), quote)
        },
    )
}

fn body_excerpt(body: Option<&[u8]>) -> String {
    match body {
        None => "(no body)".to_string(),
        Some(bytes) => std::str::from_utf8(bytes)
            .map_or_else(|_| format!("({} bytes, non-utf8)", bytes.len()), quote),
    }
}

impl MockMatcher {
    /// Evaluate a request against every mock and report criterion by criterion.
    ///
    /// Diagnostics only: unlike [`MockMatcher::find_match`] this never consumes
    /// a `once` mock, records a call, or touches the match cache.
    #[must_use]
    pub fn explain(
        &self,
        method: &Method,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) -> MatchReport {
        let request = match query {
            Some(q) if !q.is_empty() => format!("{method} {path}?{q}"),
            _ => format!("{method} {path}"),
        };

        let registry = self.registry();
        let mut candidates = registry.get_enabled_mocks();
        let mut disabled: Vec<Arc<MockDefinition>> = registry
            .get_all_mocks()
            .into_iter()
            .filter(|m| !m.enabled)
            .collect();
        disabled.sort_by_key(|m| std::cmp::Reverse(m.priority));
        candidates.extend(disabled);

        let parsed_query = crate::types::QueryMatcher::parse_query(query);
        let parsed_body_json =
            body.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok());

        let attempts = candidates
            .iter()
            .map(|mock| {
                self.explain_mock(
                    mock,
                    method,
                    path,
                    query,
                    headers,
                    body,
                    &parsed_query,
                    parsed_body_json.as_ref(),
                )
            })
            .collect();

        MatchReport { request, attempts }
    }

    #[allow(clippy::too_many_arguments)]
    fn explain_mock(
        &self,
        mock: &MockDefinition,
        method: &Method,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
        body: Option<&[u8]>,
        parsed_query: &rustc_hash::FxHashMap<String, String>,
        parsed_body_json: Option<&serde_json::Value>,
    ) -> MatchAttempt {
        let mut outcomes = Vec::new();

        outcomes.push(CriterionOutcome {
            criterion: Criterion::Method,
            passed: self.matches_method(mock, method),
            expected: if mock.request.methods.is_empty() {
                "any method".to_string()
            } else {
                mock.request
                    .methods
                    .iter()
                    .map(Method::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            actual: method.to_string(),
        });

        outcomes.push(CriterionOutcome {
            criterion: Criterion::Url,
            passed: self.matches_url(mock, path, query, Some(headers), None),
            expected: if mock.request.url_patterns.is_empty() {
                "any url".to_string()
            } else {
                mock.request
                    .url_patterns
                    .iter()
                    .map(describe_url_pattern)
                    .collect::<Vec<_>>()
                    .join(" or ")
            },
            actual: quote(path),
        });

        for matcher in &mock.request.header_matchers {
            outcomes.push(CriterionOutcome {
                criterion: Criterion::Header(matcher.name.to_string()),
                passed: matcher.matches(headers),
                expected: describe_header_pattern(&matcher.pattern),
                actual: header_value(headers, &matcher.name),
            });
        }

        for matcher in &mock.request.query_matchers {
            outcomes.push(CriterionOutcome {
                criterion: Criterion::Query(matcher.name.clone()),
                passed: matcher.matches_parsed(parsed_query),
                expected: describe_query_pattern(&matcher.pattern),
                actual: parsed_query
                    .get(&matcher.name)
                    .map_or_else(|| "(absent)".to_string(), |v| quote(v)),
            });
        }

        if let Some(graphql) = &mock.request.graphql_matcher {
            let operation = parsed_body_json
                .and_then(|json| json.get("query"))
                .and_then(serde_json::Value::as_str)
                .and_then(Self::operation_name_from_query)
                .map_or_else(|| "(no operation)".to_string(), quote);
            outcomes.push(CriterionOutcome {
                criterion: Criterion::GraphQl,
                passed: self.matches_graphql(mock, body, parsed_body_json),
                expected: describe_graphql_matcher(graphql),
                actual: operation,
            });
        }

        if let Some(matcher) = &mock.request.body_matcher {
            outcomes.push(CriterionOutcome {
                criterion: Criterion::Body,
                passed: self.matches_body(mock, body, parsed_body_json),
                expected: describe_body_matcher(matcher),
                actual: body_excerpt(body),
            });
        }

        let captures = if outcomes.iter().all(|o| o.passed) {
            self.extract_url_captures(mock, path)
        } else {
            rustc_hash::FxHashMap::default()
        };

        MatchAttempt {
            mock_id: mock.id.to_string(),
            priority: mock.priority,
            enabled: mock.enabled,
            outcomes,
            captures,
        }
    }
}
