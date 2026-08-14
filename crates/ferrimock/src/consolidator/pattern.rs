//! URL pattern detection and analysis for mock consolidation

use crate::config::MockConfig;
use crate::profile::{ConsolidationProfile, Placeholder, SegmentContext};
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::{Arc, LazyLock};
use url::Url;

/// Stand-in for a group whose sibling values are not known.
const NO_SIBLINGS: [&str; 0] = [];

/// Ask the profile about each segment before the built-in rules see the path.
///
/// The profile goes first because it is the only thing that can know an API's
/// conventions -- that `/v2/` is a version and not an id, that an 11-digit run
/// is a document id.
///
/// `siblings` holds, per position, every value the group had there, which is
/// what lets a profile distinguish a segment that varies from one that merely
/// looks variable. Pass an empty slice when the group is not known.
///
/// Returns the rewritten segments alongside a flag per segment saying whether
/// the profile settled it. The flag is the point: a segment pinned as
/// [`Placeholder::Literal`] reads identically to one the profile never saw, and
/// without the flag the built-in numeric rule would claim it right back.
fn apply_profile_normalizers<'a>(
    segments: &[&'a str],
    path: &str,
    profile: &dyn ConsolidationProfile,
    siblings: &[Vec<&'a str>],
) -> (Vec<String>, Vec<bool>) {
    let mut rewritten = Vec::with_capacity(segments.len());
    let mut settled = vec![false; segments.len()];

    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() || segment.starts_with('{') {
            rewritten.push((*segment).to_string());
            continue;
        }

        let ctx = SegmentContext {
            segment,
            index,
            previous: index.checked_sub(1).and_then(|i| segments.get(i)).copied(),
            next: segments.get(index + 1).copied(),
            path,
            siblings: siblings
                .get(index)
                .map_or(&NO_SIBLINGS[..], std::vec::Vec::as_slice),
        };

        match profile.normalize_segment(&ctx) {
            // A name carrying a slash would desync the segment alignment every
            // caller depends on, so it is refused rather than trusted.
            Some(Placeholder::Named(name)) if !name.contains('/') => {
                rewritten.push(format!("{{{name}}}"));
                if let Some(slot) = settled.get_mut(index) {
                    *slot = true;
                }
            }
            Some(Placeholder::Literal) => {
                rewritten.push((*segment).to_string());
                if let Some(slot) = settled.get_mut(index) {
                    *slot = true;
                }
            }
            Some(Placeholder::Named(_)) | None => rewritten.push((*segment).to_string()),
        }
    }

    (rewritten, settled)
}

// Paths are classified one segment at a time rather than by scanning the whole
// string. That ordering is what lets a profile's answer stand: a segment it
// pinned as a literal is never offered to the numeric rule afterwards.
#[allow(clippy::expect_used)] // Static regex literals -- panic on invalid pattern is correct
static UUID_SEGMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$")
        .expect("Failed to compile UUID segment pattern")
});
#[allow(clippy::expect_used)]
static ISO_DATE_SEGMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("Failed to compile ISO date segment pattern")
});

/// Analysis of query parameter variations in a group
#[derive(Debug)]
pub struct QueryParamAnalysis {
    pub has_variations: bool,
    pub has_common_base_path: bool,
    #[allow(dead_code)]
    pub varying_params: Vec<String>,
    #[allow(dead_code)]
    pub constant_params: Vec<String>,
    #[allow(dead_code)]
    pub variation_count: usize,
}

/// Pattern detection engine for grouping and analyzing mocks
pub struct PatternDetector {
    profile: Arc<dyn ConsolidationProfile>,
}

impl Default for PatternDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternDetector {
    /// A detector with only the built-in rules.
    pub fn new() -> Self {
        Self {
            profile: crate::profile::default_profile(),
        }
    }

    /// A detector that asks `profile` before applying the built-in rules.
    pub fn with_profile(profile: Arc<dyn ConsolidationProfile>) -> Self {
        Self { profile }
    }

    /// Group mocks by similar URL patterns
    pub fn group_similar_mocks(&self, mocks: &[MockConfig]) -> Vec<Vec<MockConfig>> {
        let mut groups: FxHashMap<String, Vec<MockConfig>> = FxHashMap::default();

        for mock in mocks {
            let key = self.extract_pattern_key(mock);
            groups.entry(key).or_default().push(mock.clone());
        }

        groups.into_values().collect()
    }

    /// Extract a pattern key for grouping similar requests
    /// Groups by: method + normalized_path + resource + priority_tier + enabled_state
    fn extract_pattern_key(&self, mock: &MockConfig) -> String {
        let Some(match_config) = mock.match_config.as_ref() else {
            return "unknown".to_string();
        };

        let url_pattern = match_config
            .urls
            .first()
            .or(match_config.url.as_ref())
            .map_or("", std::string::String::as_str);

        let url = url_pattern.strip_prefix("exact:").unwrap_or(url_pattern);

        // Determine priority tier to prevent mixing different priority mocks
        let priority_tier = match mock.priority {
            0..=99 => "low",
            100..=499 => "normal",
            _ => "high",
        };

        // Include enabled state in grouping
        let enabled_state = if mock.enabled { "enabled" } else { "disabled" };

        // Extract GraphQL grouping key if this is a GraphQL request
        let graphql_key = if Self::is_graphql_request(match_config) {
            Self::extract_graphql_grouping_key(match_config.graphql.as_ref())
        } else {
            "rest".to_string()
        };

        // Parse URL flexibly: try as absolute URL first, then as relative
        let parsed_path = Url::parse(url)
            .map(|u| u.path().to_string())
            .or_else(|_| Url::parse(&format!("http://dummy{url}")).map(|u| u.path().to_string()));

        if let Ok(path) = parsed_path {
            let method = match_config
                .methods
                .first()
                .or(match_config.method.as_ref())
                .map_or("GET", std::string::String::as_str);

            // A profile can keep resources apart that normalize alike -- two
            // endpoints reachable at the same shape of path but answering about
            // different things.
            let resource = self
                .profile
                .resource_key(&path)
                .unwrap_or(std::borrow::Cow::Borrowed(""));

            format!(
                "{}:{}:{}:{}:{}:{}",
                method,
                self.normalize_path_for_grouping(&path),
                resource,
                graphql_key,
                priority_tier,
                enabled_state
            )
        } else {
            format!(
                "{}:{}:{}:{}:{}",
                match_config
                    .methods
                    .first()
                    .or(match_config.method.as_ref())
                    .map_or("GET", std::string::String::as_str),
                url,
                graphql_key,
                priority_tier,
                enabled_state
            )
        }
    }

    /// Check if a mock uses GraphQL matching
    fn is_graphql_request(match_config: &crate::config::MatchConfig) -> bool {
        match_config.graphql.is_some()
    }

    /// Extract GraphQL-specific grouping key for separating different GraphQL operations
    ///
    /// This ensures that different GraphQL operations are grouped separately:
    /// - Query GetUser → "gql:query:GetUser"
    /// - Query GetPost → "gql:query:GetPost"
    /// - Mutation CreateUser → "gql:mutation:CreateUser"
    /// - Introspection __schema → "gql:introspection:schema"
    fn extract_graphql_grouping_key(
        graphql_config: Option<&crate::config::GraphQLMatchConfig>,
    ) -> String {
        use crate::config::matcher::{GraphQLMatchConfig, IntrospectionMatchConfig};

        match graphql_config {
            None => "rest".to_string(),

            // Boolean syntax: match.graphql = true (introspection)
            Some(GraphQLMatchConfig::Boolean(true)) => "gql:introspection:any".to_string(),
            Some(GraphQLMatchConfig::Boolean(false)) => "gql:invalid".to_string(),

            // Simple string syntax: match.graphql = "GetUser" or "query" or "*"
            Some(GraphQLMatchConfig::Simple(s)) => match s.as_str() {
                "*" => "gql:any".to_string(),
                "query" => "gql:query:*".to_string(),
                "mutation" => "gql:mutation:*".to_string(),
                "subscription" => "gql:subscription:*".to_string(),
                operation_name => format!("gql:op:{operation_name}"),
            },

            // Structured syntax with operation details
            Some(GraphQLMatchConfig::Structured {
                query,
                mutation,
                subscription,
                introspection,
                operation,
                ..
            }) => {
                // Priority: specific type fields > introspection > operation field
                if let Some(query_name) = query {
                    format!("gql:query:{query_name}")
                } else if let Some(mutation_name) = mutation {
                    format!("gql:mutation:{mutation_name}")
                } else if let Some(subscription_name) = subscription {
                    format!("gql:subscription:{subscription_name}")
                } else if let Some(intro) = introspection {
                    // Parse introspection type
                    let intro_type = match intro {
                        IntrospectionMatchConfig::Bool(true) => "any",
                        IntrospectionMatchConfig::Bool(false) => "none",
                        IntrospectionMatchConfig::String(s) => match s.as_str() {
                            "true" | "*" => "any",
                            "schema" => "schema",
                            "type" => "type",
                            "typename" => "typename",
                            _ => "unknown",
                        },
                    };
                    format!("gql:introspection:{intro_type}")
                } else if let Some(operation_name) = operation {
                    format!("gql:op:{operation_name}")
                } else {
                    // Has GraphQL config but no specific operation - group by variables existence
                    "gql:generic".to_string()
                }
            }
        }
    }

    /// Normalize a path for grouping, collapsing ids, UUIDs and dates into
    /// placeholders so that requests for different instances of one resource
    /// land in the same group.
    ///
    /// Repeated kinds are numbered -- `/orgs/{id}/users/{id2}` -- so a path with
    /// two ids stays distinguishable from one with a single id.
    pub fn normalize_path_for_grouping(&self, path: &str) -> String {
        self.normalize_path_with_siblings(path, &[])
    }

    /// [`Self::normalize_path_for_grouping`], told what the rest of the group
    /// had in each position so a profile can judge whether a segment varies.
    fn normalize_path_with_siblings(&self, path: &str, siblings: &[Vec<&str>]) -> String {
        let segments: Vec<&str> = path.split('/').collect();

        // The profile speaks first: only it can know that `/v2/` is a version
        // rather than an id, and a segment it settles is never reconsidered.
        let (rewritten, settled) =
            apply_profile_normalizers(&segments, path, self.profile.as_ref(), siblings);

        let mut counters: FxHashMap<&'static str, usize> = FxHashMap::default();
        let normalized: Vec<String> = rewritten
            .into_iter()
            .enumerate()
            .map(|(index, segment)| {
                if settled.get(index).copied().unwrap_or(false) {
                    return segment;
                }

                let Some(kind) = Self::builtin_segment_kind(&segment) else {
                    return segment;
                };

                let counter = counters.entry(kind).or_insert(0);
                *counter += 1;
                if *counter == 1 {
                    format!("{{{kind}}}")
                } else {
                    format!("{{{kind}{counter}}}")
                }
            })
            .collect();

        normalized.join("/")
    }

    /// The placeholder kind the built-in rules give a segment, if any.
    fn builtin_segment_kind(segment: &str) -> Option<&'static str> {
        if UUID_SEGMENT.is_match(segment) {
            Some("uuid")
        } else if ISO_DATE_SEGMENT.is_match(segment) {
            Some("date")
        } else if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
            Some("id")
        } else {
            None
        }
    }

    /// Generate a smart URL pattern based on the URLs in the group
    /// Returns clean URLs without prefixes - system will auto-detect matching strategy
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: callers ensure non-empty group; `.windows(2)` guarantees 2-element slices
    pub fn generate_smart_url_pattern(&self, group: &[MockConfig]) -> String {
        let base_path = Self::extract_base_path(&group[0]);

        let query_analysis = Self::analyze_query_param_variations(group);
        if query_analysis.has_variations && query_analysis.has_common_base_path {
            // Query param variations - just use base path (will be prefix match)
            return base_path;
        }

        if Self::all_urls_identical(group) {
            let first_url = group[0]
                .match_config
                .as_ref()
                .and_then(|mc| mc.urls.first().or(mc.url.as_ref()))
                .map_or("", std::string::String::as_str);
            // Return clean URL without any prefix - will be exact match
            return first_url
                .strip_prefix("exact:")
                .unwrap_or(first_url)
                .to_string();
        }

        self.generate_evidence_pattern(group).unwrap_or(base_path)
    }

    /// Build a URL pattern from what the group's paths actually vary.
    ///
    /// The grouping normalizer is deliberately loose -- it has to pull
    /// candidates together before anything is known about them, so it turns
    /// every numeric segment into `{id}`. A *pattern* must be the opposite:
    /// `/api/2/users/1` and `/api/2/users/2` differ only in the last segment, so
    /// the API version stays literal and only the id becomes a placeholder.
    /// Deriving that from observed variation rather than from a regex is what
    /// keeps two versions of an endpoint from claiming the same pattern.
    ///
    /// Returns `None` when the paths do not have a common segment count, which
    /// leaves no sensible segment-wise alignment.
    pub fn generate_evidence_pattern(&self, group: &[MockConfig]) -> Option<String> {
        let paths: Vec<String> = group.iter().map(Self::extract_base_path).collect();
        let segment_count = paths.first()?.split('/').count();
        if paths
            .iter()
            .any(|path| path.split('/').count() != segment_count)
        {
            return None;
        }

        // What every recording had in each position. The profile sees this, so
        // it can answer about a segment knowing whether the group varied there.
        let siblings: Vec<Vec<&str>> = (0..segment_count)
            .map(|index| {
                paths
                    .iter()
                    .filter_map(|path| path.split('/').nth(index))
                    .collect()
            })
            .collect();

        let first_path = paths.first()?;
        let first_segments: Vec<&str> = first_path.split('/').collect();
        let (rewritten, settled) = apply_profile_normalizers(
            &first_segments,
            first_path,
            self.profile.as_ref(),
            &siblings,
        );

        let mut counters: FxHashMap<&'static str, usize> = FxHashMap::default();
        let mut segments = Vec::with_capacity(segment_count);

        for index in 0..segment_count {
            if settled.get(index).copied().unwrap_or(false) {
                segments.push(rewritten.get(index).cloned().unwrap_or_default());
                continue;
            }

            let Some(values) = siblings.get(index) else {
                continue;
            };
            let Some(first) = values.first().copied() else {
                continue;
            };

            // A position every recording agreed on is a literal, whatever it
            // looks like. This is what keeps an API version out of `{id}`.
            if values.iter().all(|value| *value == first) {
                segments.push(first.to_string());
                continue;
            }

            let kind = Self::classify_segment_values(values);
            let counter = counters.entry(kind).or_insert(0);
            *counter += 1;
            segments.push(if *counter == 1 {
                format!("{{{kind}}}")
            } else {
                format!("{{{kind}{counter}}}")
            });
        }

        Some(segments.join("/"))
    }

    /// Name the placeholder a set of varying segment values deserves.
    ///
    /// `id` is the fallback rather than something more honest like `seg` because
    /// template generation binds response `id` fields to `captures.id`; renaming
    /// it would silently stop those bindings resolving.
    fn classify_segment_values(values: &[&str]) -> &'static str {
        if values.iter().all(|value| UUID_SEGMENT.is_match(value)) {
            "uuid"
        } else if values.iter().all(|value| ISO_DATE_SEGMENT.is_match(value)) {
            "date"
        } else {
            "id"
        }
    }

    /// Check if all URLs in group are identical
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.len() < 2` returns early
    fn all_urls_identical(group: &[MockConfig]) -> bool {
        if group.len() < 2 {
            return true;
        }

        let first_url = group[0]
            .match_config
            .as_ref()
            .and_then(|mc| mc.urls.first().or(mc.url.as_ref()))
            .map_or("", std::string::String::as_str);

        group.iter().skip(1).all(|mock| {
            let mock_url = mock
                .match_config
                .as_ref()
                .and_then(|mc| mc.urls.first().or(mc.url.as_ref()))
                .map_or("", std::string::String::as_str);
            mock_url == first_url
        })
    }

    /// Analyze query parameter variations across a group of mocks
    pub fn analyze_query_param_variations(group: &[MockConfig]) -> QueryParamAnalysis {
        let mut base_paths = FxHashSet::default();
        let mut all_params = FxHashMap::<String, FxHashSet<String>>::default();

        for mock in group {
            if let Some(ref match_config) = mock.match_config {
                let url_patterns = if !match_config.urls.is_empty() {
                    &match_config.urls
                } else if let Some(ref url) = match_config.url {
                    &vec![url.clone()]
                } else {
                    continue;
                };

                for url_pattern in url_patterns {
                    let url = url_pattern.strip_prefix("exact:").unwrap_or(url_pattern);

                    // Parse flexibly: absolute URL first, then relative
                    let parsed =
                        Url::parse(url).or_else(|_| Url::parse(&format!("http://dummy{url}")));
                    if let Ok(parsed) = parsed {
                        base_paths.insert(parsed.path().to_string());

                        for (key, value) in parsed.query_pairs() {
                            all_params
                                .entry(key.to_string())
                                .or_default()
                                .insert(value.to_string());
                        }
                    }
                }
            }
        }

        let has_variations = all_params.values().any(|values| values.len() > 1);
        let has_common_base_path = base_paths.len() == 1;

        let varying_params: Vec<String> = all_params
            .iter()
            .filter(|(_, values)| values.len() > 1)
            .map(|(key, _)| key.clone())
            .collect();

        let constant_params: Vec<String> = all_params
            .iter()
            .filter(|(_, values)| values.len() == 1)
            .map(|(key, _)| key.clone())
            .collect();

        let variation_count = varying_params.len();

        QueryParamAnalysis {
            has_variations,
            has_common_base_path,
            varying_params,
            constant_params,
            variation_count,
        }
    }

    /// Extract base path from URL (without query params).
    /// Handles both absolute URLs (`https://api.example.com/v2/users/me`)
    /// and relative paths (`/v2/users/me`).
    pub fn extract_base_path(mock: &MockConfig) -> String {
        let url = mock
            .match_config
            .as_ref()
            .and_then(|mc| mc.urls.first().or(mc.url.as_ref()))
            .map_or("", std::string::String::as_str);
        let cleaned = url.strip_prefix("exact:").unwrap_or(url);

        // Try parsing as an absolute URL - if it has a host, return just the path
        if let Ok(parsed) = Url::parse(cleaned)
            && parsed.host_str().is_some()
        {
            return parsed.path().to_string();
        }

        // Relative URL - strip query params
        if let Some(query_pos) = cleaned.find('?') {
            cleaned.get(..query_pos).unwrap_or(cleaned).to_string()
        } else {
            cleaned.to_string()
        }
    }

    /// Whether every mock in the group is interchangeable with the first.
    ///
    /// Collapsing a group to one member discards everything the others said, so
    /// the comparison has to cover everything a request can be selected on --
    /// method, URL, and the header, query and body matchers -- as well as the
    /// whole answer. Comparing only URL, status and body let a group collapse
    /// onto a member with different response headers, or merged a `GET` mock
    /// with a `GET, HEAD` one and lost the `HEAD`.
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.len() < 2` returns early
    pub fn are_duplicates(group: &[MockConfig]) -> bool {
        if group.len() < 2 {
            return false;
        }

        let first = DuplicateKey::of(&group[0]);
        group
            .iter()
            .skip(1)
            .all(|mock| DuplicateKey::of(mock) == first)
    }
}

/// Everything that has to agree before two recordings are interchangeable.
#[derive(PartialEq, Eq)]
struct DuplicateKey {
    methods: Vec<String>,
    urls: Vec<String>,
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    body_match: Vec<(String, String)>,
    graphql: Option<String>,
    status: Option<u16>,
    response_body: Option<String>,
    response_headers: Vec<(String, String)>,
}

impl DuplicateKey {
    fn of(mock: &MockConfig) -> Self {
        let match_config = mock.match_config.as_ref();

        let mut methods = match_config.map_or_else(Vec::new, |m| {
            if m.methods.is_empty() {
                m.method.clone().into_iter().collect()
            } else {
                m.methods.clone()
            }
        });
        methods.sort_unstable();

        let mut urls = match_config.map_or_else(Vec::new, |m| {
            if m.urls.is_empty() {
                m.url.clone().into_iter().collect()
            } else {
                m.urls.clone()
            }
        });
        urls.sort_unstable();

        let mut headers: Vec<(String, String)> = match_config.map_or_else(Vec::new, |m| {
            m.headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        serde_json::to_string(value).unwrap_or_default(),
                    )
                })
                .collect()
        });
        headers.sort_unstable();

        let mut query: Vec<(String, String)> = match_config.map_or_else(Vec::new, |m| {
            m.query
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect()
        });
        query.sort_unstable();

        let mut body_match: Vec<(String, String)> = match_config.map_or_else(Vec::new, |m| {
            m.body
                .iter()
                .map(|(name, value)| (name.clone(), value.to_string()))
                .collect()
        });
        body_match.sort_unstable();

        let graphql = match_config
            .and_then(|m| m.graphql.as_ref())
            .map(|graphql| serde_json::to_string(graphql).unwrap_or_default());

        let response = mock.response_config.as_ref();
        let mut response_headers: Vec<(String, String)> = response
            .and_then(crate::config::ResponseConfig::headers)
            .map(|headers| {
                headers
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        response_headers.sort_unstable();

        Self {
            methods,
            urls,
            headers,
            query,
            body_match,
            graphql,
            status: response.and_then(crate::config::ResponseConfig::status),
            response_body: response.and_then(|r| r.body().cloned()),
            response_headers,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice
)]
mod tests {
    use super::*;
    use crate::config::matcher::{GraphQLMatchConfig, MatchConfig};

    // Helper function to create a test MockConfig with GraphQL config
    fn create_graphql_mock(id: &str, graphql: GraphQLMatchConfig) -> MockConfig {
        MockConfig {
            id: id.into(),
            description: None,
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec!["POST".to_string()],
                urls: vec!["/graphql".to_string()],
                graphql: Some(graphql),
                ..Default::default()
            }),
            request: None,
            response_config: None,
            patch: None,
            delay: None,
            network_error: None,
            sse: None,
            ws: None,
        }
    }

    // Helper function to create a test MockConfig for REST endpoints
    fn create_rest_mock(id: &str, method: &str, url: &str) -> MockConfig {
        MockConfig {
            id: id.into(),
            description: None,
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec![method.to_string()],
                urls: vec![url.to_string()],
                graphql: None,
                ..Default::default()
            }),
            request: None,
            response_config: None,
            patch: None,
            delay: None,
            network_error: None,
            sse: None,
            ws: None,
        }
    }

    #[test]
    fn test_normalize_path_numeric_id() {
        assert_eq!(
            PatternDetector::new().normalize_path_for_grouping("/users/123"),
            "/users/{id}"
        );
        assert_eq!(
            PatternDetector::new().normalize_path_for_grouping("/api/files/456/download"),
            "/api/files/{id}/download"
        );
    }

    #[test]
    fn test_normalize_path_uuid() {
        let path = "/files/550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            PatternDetector::new().normalize_path_for_grouping(path),
            "/files/{uuid}"
        );
    }

    #[test]
    fn test_normalize_path_before_regex_generation() {
        let path_with_id = "/api/file-info/10000000002/";
        let normalized = PatternDetector::new().normalize_path_for_grouping(path_with_id);
        assert_eq!(normalized, "/api/file-info/{id}/");

        let path_with_query = "/api/file-info/10000000003";
        let normalized2 = PatternDetector::new().normalize_path_for_grouping(path_with_query);
        assert_eq!(normalized2, "/api/file-info/{id}");
    }

    // GraphQL Grouping Tests

    #[test]
    fn test_graphql_grouping_separate_operations() {
        // Create two different GraphQL query operations
        let mock1 = create_graphql_mock(
            "get-user",
            GraphQLMatchConfig::Structured {
                query: Some("GetUser".to_string()),
                mutation: None,
                subscription: None,
                introspection: None,
                operation: None,
                variables: FxHashMap::default(),
            },
        );

        let mock2 = create_graphql_mock(
            "get-post",
            GraphQLMatchConfig::Structured {
                query: Some("GetPost".to_string()),
                mutation: None,
                subscription: None,
                introspection: None,
                operation: None,
                variables: FxHashMap::default(),
            },
        );

        let key1 = PatternDetector::new().extract_pattern_key(&mock1);
        let key2 = PatternDetector::new().extract_pattern_key(&mock2);

        // Different operations should have different grouping keys
        assert_ne!(key1, key2);
        assert!(key1.contains("gql:query:GetUser"));
        assert!(key2.contains("gql:query:GetPost"));
    }

    #[test]
    fn test_graphql_grouping_query_vs_mutation() {
        let query_mock = create_graphql_mock(
            "get-user-query",
            GraphQLMatchConfig::Structured {
                query: Some("GetUser".to_string()),
                mutation: None,
                subscription: None,
                introspection: None,
                operation: None,
                variables: FxHashMap::default(),
            },
        );

        let mutation_mock = create_graphql_mock(
            "create-user-mutation",
            GraphQLMatchConfig::Structured {
                query: None,
                mutation: Some("CreateUser".to_string()),
                subscription: None,
                introspection: None,
                operation: None,
                variables: FxHashMap::default(),
            },
        );

        let query_key = PatternDetector::new().extract_pattern_key(&query_mock);
        let mutation_key = PatternDetector::new().extract_pattern_key(&mutation_mock);

        // Query and mutation should have different keys
        assert_ne!(query_key, mutation_key);
        assert!(query_key.contains("gql:query:GetUser"));
        assert!(mutation_key.contains("gql:mutation:CreateUser"));
    }

    #[test]
    fn test_graphql_grouping_introspection() {
        use crate::config::matcher::IntrospectionMatchConfig;

        let introspection_mock = create_graphql_mock(
            "introspection",
            GraphQLMatchConfig::Structured {
                query: None,
                mutation: None,
                subscription: None,
                introspection: Some(IntrospectionMatchConfig::String("schema".to_string())),
                operation: None,
                variables: FxHashMap::default(),
            },
        );

        let key = PatternDetector::new().extract_pattern_key(&introspection_mock);
        assert!(key.contains("gql:introspection:schema"));
    }

    #[test]
    fn test_graphql_grouping_simple_syntax() {
        let mock = create_graphql_mock(
            "get-user-simple",
            GraphQLMatchConfig::Simple("GetUser".to_string()),
        );

        let key = PatternDetector::new().extract_pattern_key(&mock);
        assert!(key.contains("gql:op:GetUser"));
    }

    #[test]
    fn test_graphql_vs_rest_grouping() {
        let graphql_mock = create_graphql_mock(
            "graphql-user",
            GraphQLMatchConfig::Simple("GetUser".to_string()),
        );

        let rest_mock = create_rest_mock("rest-user", "GET", "/api/users");

        let graphql_key = PatternDetector::new().extract_pattern_key(&graphql_mock);
        let rest_key = PatternDetector::new().extract_pattern_key(&rest_mock);

        // GraphQL and REST should have different keys
        assert_ne!(graphql_key, rest_key);
        assert!(graphql_key.contains("gql:op:GetUser"));
        assert!(rest_key.contains("rest"));
    }

    #[test]
    fn test_graphql_grouping_same_operation_grouped_together() {
        // Two mocks with same GraphQL operation should have same grouping key
        let mock1 = create_graphql_mock(
            "get-user-1",
            GraphQLMatchConfig::Structured {
                query: Some("GetUser".to_string()),
                mutation: None,
                subscription: None,
                introspection: None,
                operation: None,
                variables: {
                    let mut vars = FxHashMap::default();
                    vars.insert(
                        "id".to_string(),
                        serde_json::Value::String("123".to_string()),
                    );
                    vars
                },
            },
        );

        let mock2 = create_graphql_mock(
            "get-user-2",
            GraphQLMatchConfig::Structured {
                query: Some("GetUser".to_string()),
                mutation: None,
                subscription: None,
                introspection: None,
                operation: None,
                variables: {
                    let mut vars = FxHashMap::default();
                    vars.insert(
                        "id".to_string(),
                        serde_json::Value::String("456".to_string()),
                    );
                    vars
                },
            },
        );

        let key1 = PatternDetector::new().extract_pattern_key(&mock1);
        let key2 = PatternDetector::new().extract_pattern_key(&mock2);

        // Same operation with different variables should have SAME grouping key
        // (variables are not part of the grouping key - they'll be analyzed separately)
        assert_eq!(key1, key2);
        assert!(key1.contains("gql:query:GetUser"));
    }

    // -- Absolute URL handling --

    #[test]
    fn test_extract_base_path_absolute_url() {
        let mock = create_rest_mock("abs-1", "GET", "exact:https://api.example.com/v2/users/me");
        assert_eq!(PatternDetector::extract_base_path(&mock), "/v2/users/me");
    }

    #[test]
    fn test_extract_base_path_absolute_url_with_query() {
        let mock = create_rest_mock(
            "abs-2",
            "GET",
            "exact:https://api.example.com/v2/folders/0/items?fields=name&limit=100",
        );
        assert_eq!(
            PatternDetector::extract_base_path(&mock),
            "/v2/folders/0/items"
        );
    }

    #[test]
    fn test_extract_base_path_relative_url() {
        let mock = create_rest_mock("rel-1", "GET", "exact:/v2/users/me");
        assert_eq!(PatternDetector::extract_base_path(&mock), "/v2/users/me");
    }

    #[test]
    fn test_grouping_mixed_absolute_and_relative() {
        // An absolute URL and relative URL for the same path should group together
        let mock_abs = create_rest_mock("abs", "GET", "exact:https://api.example.com/v2/users/123");
        let mock_rel = create_rest_mock("rel", "GET", "exact:/v2/users/456");

        let key_abs = PatternDetector::new().extract_pattern_key(&mock_abs);
        let key_rel = PatternDetector::new().extract_pattern_key(&mock_rel);

        // Both should normalize to the same grouping key (path with {id})
        assert_eq!(key_abs, key_rel);
    }

    // -- Duplicate detection --

    fn answered(id: &str, method: &str, url: &str, status: u16, body: &str) -> MockConfig {
        let mut mock = create_rest_mock(id, method, url);
        mock.response_config = Some(crate::config::ReturnConfig::Structured {
            status: Some(status),
            headers: FxHashMap::default(),
            body: Some(body.to_string()),
            template: None,
            file: None,
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        });
        mock
    }

    #[test]
    fn identical_recordings_are_duplicates() {
        let group = vec![
            answered("a", "GET", "/x", 200, r#"{"id":1}"#),
            answered("b", "GET", "/x", 200, r#"{"id":1}"#),
        ];
        assert!(PatternDetector::are_duplicates(&group));
    }

    #[test]
    fn differing_response_headers_are_not_duplicates() {
        let mut group = vec![
            answered("a", "GET", "/x", 200, r#"{"id":1}"#),
            answered("b", "GET", "/x", 200, r#"{"id":1}"#),
        ];
        if let Some(crate::config::ReturnConfig::Structured { headers, .. }) =
            group[1].response_config.as_mut()
        {
            headers.insert("x-cache".to_string(), "HIT".to_string());
        }
        assert!(
            !PatternDetector::are_duplicates(&group),
            "collapsing these would silently drop the x-cache header"
        );
    }

    #[test]
    fn a_wider_method_set_is_not_a_duplicate_of_a_narrower_one() {
        let mut group = vec![
            answered("a", "GET", "/x", 200, r#"{"id":1}"#),
            answered("b", "GET", "/x", 200, r#"{"id":1}"#),
        ];
        if let Some(match_config) = group[1].match_config.as_mut() {
            match_config.methods = vec!["GET".to_string(), "HEAD".to_string()];
        }
        assert!(
            !PatternDetector::are_duplicates(&group),
            "collapsing onto the GET-only mock would lose HEAD"
        );
    }

    #[test]
    fn a_pinned_request_body_is_not_a_duplicate_of_an_unpinned_one() {
        let mut group = vec![
            answered("a", "POST", "/search", 200, r#"{"hits":1}"#),
            answered("b", "POST", "/search", 200, r#"{"hits":1}"#),
        ];
        if let Some(match_config) = group[1].match_config.as_mut() {
            match_config
                .body
                .insert("$.query".to_string(), serde_json::json!("invoices"));
        }
        assert!(!PatternDetector::are_duplicates(&group));
    }

    #[test]
    fn method_and_url_order_does_not_decide_duplication() {
        let mut group = vec![
            answered("a", "GET", "/x", 200, r#"{"id":1}"#),
            answered("b", "GET", "/x", 200, r#"{"id":1}"#),
        ];
        if let Some(match_config) = group[0].match_config.as_mut() {
            match_config.methods = vec!["GET".to_string(), "HEAD".to_string()];
        }
        if let Some(match_config) = group[1].match_config.as_mut() {
            match_config.methods = vec!["HEAD".to_string(), "GET".to_string()];
        }
        assert!(PatternDetector::are_duplicates(&group));
    }

    #[test]
    fn test_analyze_query_params_absolute_urls() {
        let mocks = vec![
            create_rest_mock(
                "q1",
                "GET",
                "exact:https://api.example.com/v2/folders/0/items?fields=name&offset=0",
            ),
            create_rest_mock(
                "q2",
                "GET",
                "exact:https://api.example.com/v2/folders/0/items?fields=name&offset=100",
            ),
        ];

        let analysis = PatternDetector::analyze_query_param_variations(&mocks);
        assert!(analysis.has_variations);
        assert!(analysis.has_common_base_path);
    }
}
