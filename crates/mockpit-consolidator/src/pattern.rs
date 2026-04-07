//! URL pattern detection and analysis for mock consolidation

use mockpit_config::MockConfig;
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::LazyLock;
use url::Url;

#[allow(clippy::expect_used)] // Static regex literals -- panic on invalid pattern is correct
static UUID_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}(/|$)")
        .expect("Failed to compile UUID pattern")
});
#[allow(clippy::expect_used)]
static ISO_DATE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/(\d{4}-\d{2}-\d{2})(/|$)").expect("Failed to compile ISO date pattern")
});
#[allow(clippy::expect_used)]
static NUMERIC_ID_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(\d+)(/|$)").expect("Failed to compile numeric ID pattern"));

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
pub struct PatternDetector;

impl PatternDetector {
    /// Group mocks by similar URL patterns
    pub fn group_similar_mocks(mocks: &[MockConfig]) -> Vec<Vec<MockConfig>> {
        let mut groups: FxHashMap<String, Vec<MockConfig>> = FxHashMap::default();

        for mock in mocks {
            let key = Self::extract_pattern_key(mock);
            groups.entry(key).or_default().push(mock.clone());
        }

        groups.into_values().collect()
    }

    /// Extract a pattern key for grouping similar requests
    /// Groups by: method + normalized_path + priority_tier + enabled_state
    fn extract_pattern_key(mock: &MockConfig) -> String {
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

            format!(
                "{}:{}:{}:{}:{}",
                method,
                Self::normalize_path_for_grouping(&path),
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
    fn is_graphql_request(match_config: &mockpit_config::MatchConfig) -> bool {
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
        graphql_config: Option<&mockpit_config::GraphQLMatchConfig>,
    ) -> String {
        use mockpit_config::matcher::{GraphQLMatchConfig, IntrospectionMatchConfig};

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

    /// Normalize a path for grouping (replace numeric IDs, UUIDs, dates)
    /// Uses unique placeholders for multiple IDs in same path
    pub fn normalize_path_for_grouping(path: &str) -> String {
        let mut normalized = path.to_string();
        let mut id_counter = 0;

        // Replace UUIDs first (more specific): /files/550e...000 -> /files/{uuid}
        // Use counter for multiple UUIDs: /orgs/{uuid1}/files/{uuid2}
        let mut uuid_counter = 0;
        normalized = UUID_PATTERN
            .replace_all(&normalized, |caps: &regex::Captures<'_>| {
                uuid_counter += 1;
                let suffix = caps.get(1).map_or("", |m| m.as_str());
                if uuid_counter == 1 {
                    format!("/{{uuid}}{suffix}")
                } else {
                    format!("/{{uuid{uuid_counter}}}{suffix}")
                }
            })
            .to_string();

        // Replace ISO dates: /logs/2024-01-15 -> /logs/{date}
        normalized = ISO_DATE_PATTERN
            .replace_all(&normalized, "/{date}$2")
            .to_string();

        // Replace numeric IDs: /users/123 -> /users/{id}
        // Use counter for multiple IDs: /orgs/{id1}/users/{id2}
        normalized = NUMERIC_ID_PATTERN
            .replace_all(&normalized, |caps: &regex::Captures<'_>| {
                id_counter += 1;
                let suffix = caps.get(2).map_or("", |m| m.as_str());
                if id_counter == 1 {
                    format!("/{{id}}{suffix}")
                } else {
                    format!("/{{id{id_counter}}}{suffix}")
                }
            })
            .to_string();

        normalized
    }

    /// Generate a smart URL pattern based on the URLs in the group
    /// Returns clean URLs without prefixes - system will auto-detect matching strategy
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: callers ensure non-empty group; `.windows(2)` guarantees 2-element slices
    pub fn generate_smart_url_pattern(group: &[MockConfig]) -> String {
        let base_path = Self::extract_base_path(&group[0]);

        let query_analysis = Self::analyze_query_param_variations(group);
        if query_analysis.has_variations && query_analysis.has_common_base_path {
            // Query param variations - just use base path (will be prefix match)
            return base_path;
        }

        let has_varying_path_ids = Self::has_varying_path_segments(group);
        if has_varying_path_ids {
            let normalized = Self::normalize_path_for_grouping(&base_path);
            // Generate Express-style pattern like /users/{id}
            return Self::generate_express_style_pattern(&normalized, group);
        }

        if Self::all_urls_identical(group) {
            let first_url = group[0]
                .match_config
                .as_ref()
                .and_then(|mc| mc.urls.first().or(mc.url.as_ref()))
                .map_or("", std::string::String::as_str);
            // Return clean URL without any prefix - will be exact match
            first_url
                .strip_prefix("exact:")
                .unwrap_or(first_url)
                .to_string()
        } else {
            // Different URLs in group - use base path (will be prefix match)
            base_path
        }
    }

    /// Check if there are varying segments in the path (not query params)
    #[allow(clippy::indexing_slicing)] // `.windows(2)` guarantees 2-element slices
    fn has_varying_path_segments(group: &[MockConfig]) -> bool {
        if group.len() < 2 {
            return false;
        }

        let base_paths: Vec<String> = group.iter().map(Self::extract_base_path).collect();

        let normalized: Vec<String> = base_paths
            .iter()
            .map(|path| Self::normalize_path_for_grouping(path))
            .collect();

        let all_same_normalized = normalized.windows(2).all(|w| w[0] == w[1]);
        let all_same_original = base_paths.windows(2).all(|w| w[0] == w[1]);

        all_same_normalized && !all_same_original
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
    /// Handles both absolute URLs (`https://api.box.com/2.0/users/me`)
    /// and relative paths (`/2.0/users/me`).
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

    /// Generate Express-style pattern for ID-based paths
    /// Returns clean patterns like /users/{id} that the system will auto-convert to regex
    fn generate_express_style_pattern(base_path: &str, _group: &[MockConfig]) -> String {
        // Simply return the base path which already has {id} or {uuid} placeholders
        // The system will auto-convert patterns like /users/{id} to proper regex
        base_path.to_string()
    }

    /// Check if all mocks in group are duplicates
    #[allow(clippy::indexing_slicing)] // `group[0]` safe: `group.len() < 2` returns early
    pub fn are_duplicates(group: &[MockConfig]) -> bool {
        if group.len() < 2 {
            return false;
        }

        let first_urls = group[0].match_config.as_ref().map(|mc| {
            if mc.urls.is_empty() {
                mc.url.as_ref().map(|u| vec![u.clone()]).unwrap_or_default()
            } else {
                mc.urls.clone()
            }
        });
        let first_body = group[0]
            .response_config
            .as_ref()
            .and_then(|rc| rc.body().cloned());
        let first_status = group[0]
            .response_config
            .as_ref()
            .and_then(mockpit_config::ResponseConfig::status);

        group.iter().skip(1).all(|mock| {
            let mock_urls = mock.match_config.as_ref().map(|mc| {
                if mc.urls.is_empty() {
                    mc.url.as_ref().map(|u| vec![u.clone()]).unwrap_or_default()
                } else {
                    mc.urls.clone()
                }
            });
            let mock_body = mock
                .response_config
                .as_ref()
                .and_then(|rc| rc.body().cloned());
            let mock_status = mock
                .response_config
                .as_ref()
                .and_then(mockpit_config::ResponseConfig::status);

            mock_urls == first_urls && mock_body == first_body && mock_status == first_status
        })
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
    use mockpit_config::matcher::{GraphQLMatchConfig, MatchConfig};

    // Helper function to create a test MockConfig with GraphQL config
    fn create_graphql_mock(id: &str, graphql: GraphQLMatchConfig) -> MockConfig {
        MockConfig {
            id: id.into(),
            description: None,
            priority: 100,
            enabled: true,
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
        }
    }

    // Helper function to create a test MockConfig for REST endpoints
    fn create_rest_mock(id: &str, method: &str, url: &str) -> MockConfig {
        MockConfig {
            id: id.into(),
            description: None,
            priority: 100,
            enabled: true,
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
        }
    }

    #[test]
    fn test_normalize_path_numeric_id() {
        assert_eq!(
            PatternDetector::normalize_path_for_grouping("/users/123"),
            "/users/{id}"
        );
        assert_eq!(
            PatternDetector::normalize_path_for_grouping("/api/files/456/download"),
            "/api/files/{id}/download"
        );
    }

    #[test]
    fn test_normalize_path_uuid() {
        let path = "/files/550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            PatternDetector::normalize_path_for_grouping(path),
            "/files/{uuid}"
        );
    }

    #[test]
    fn test_normalize_path_before_regex_generation() {
        let path_with_id = "/app-api/sign-web/file-info/23930793379/";
        let normalized = PatternDetector::normalize_path_for_grouping(path_with_id);
        assert_eq!(normalized, "/app-api/sign-web/file-info/{id}/");

        let path_with_query = "/app-api/sign-web/file-info/23522876283";
        let normalized2 = PatternDetector::normalize_path_for_grouping(path_with_query);
        assert_eq!(normalized2, "/app-api/sign-web/file-info/{id}");
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

        let key1 = PatternDetector::extract_pattern_key(&mock1);
        let key2 = PatternDetector::extract_pattern_key(&mock2);

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

        let query_key = PatternDetector::extract_pattern_key(&query_mock);
        let mutation_key = PatternDetector::extract_pattern_key(&mutation_mock);

        // Query and mutation should have different keys
        assert_ne!(query_key, mutation_key);
        assert!(query_key.contains("gql:query:GetUser"));
        assert!(mutation_key.contains("gql:mutation:CreateUser"));
    }

    #[test]
    fn test_graphql_grouping_introspection() {
        use mockpit_config::matcher::IntrospectionMatchConfig;

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

        let key = PatternDetector::extract_pattern_key(&introspection_mock);
        assert!(key.contains("gql:introspection:schema"));
    }

    #[test]
    fn test_graphql_grouping_simple_syntax() {
        let mock = create_graphql_mock(
            "get-user-simple",
            GraphQLMatchConfig::Simple("GetUser".to_string()),
        );

        let key = PatternDetector::extract_pattern_key(&mock);
        assert!(key.contains("gql:op:GetUser"));
    }

    #[test]
    fn test_graphql_vs_rest_grouping() {
        let graphql_mock = create_graphql_mock(
            "graphql-user",
            GraphQLMatchConfig::Simple("GetUser".to_string()),
        );

        let rest_mock = create_rest_mock("rest-user", "GET", "/api/users");

        let graphql_key = PatternDetector::extract_pattern_key(&graphql_mock);
        let rest_key = PatternDetector::extract_pattern_key(&rest_mock);

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

        let key1 = PatternDetector::extract_pattern_key(&mock1);
        let key2 = PatternDetector::extract_pattern_key(&mock2);

        // Same operation with different variables should have SAME grouping key
        // (variables are not part of the grouping key - they'll be analyzed separately)
        assert_eq!(key1, key2);
        assert!(key1.contains("gql:query:GetUser"));
    }

    // -- Absolute URL handling --

    #[test]
    fn test_extract_base_path_absolute_url() {
        let mock = create_rest_mock("abs-1", "GET", "exact:https://api.box.com/2.0/users/me");
        assert_eq!(PatternDetector::extract_base_path(&mock), "/2.0/users/me");
    }

    #[test]
    fn test_extract_base_path_absolute_url_with_query() {
        let mock = create_rest_mock(
            "abs-2",
            "GET",
            "exact:https://api.box.com/2.0/folders/0/items?fields=name&limit=100",
        );
        assert_eq!(
            PatternDetector::extract_base_path(&mock),
            "/2.0/folders/0/items"
        );
    }

    #[test]
    fn test_extract_base_path_relative_url() {
        let mock = create_rest_mock("rel-1", "GET", "exact:/2.0/users/me");
        assert_eq!(PatternDetector::extract_base_path(&mock), "/2.0/users/me");
    }

    #[test]
    fn test_grouping_mixed_absolute_and_relative() {
        // An absolute URL and relative URL for the same path should group together
        let mock_abs = create_rest_mock("abs", "GET", "exact:https://api.box.com/2.0/users/123");
        let mock_rel = create_rest_mock("rel", "GET", "exact:/2.0/users/456");

        let key_abs = PatternDetector::extract_pattern_key(&mock_abs);
        let key_rel = PatternDetector::extract_pattern_key(&mock_rel);

        // Both should normalize to the same grouping key (path with {id})
        assert_eq!(key_abs, key_rel);
    }

    #[test]
    fn test_analyze_query_params_absolute_urls() {
        let mocks = vec![
            create_rest_mock(
                "q1",
                "GET",
                "exact:https://api.box.com/2.0/folders/0/items?fields=name&offset=0",
            ),
            create_rest_mock(
                "q2",
                "GET",
                "exact:https://api.box.com/2.0/folders/0/items?fields=name&offset=100",
            ),
        ];

        let analysis = PatternDetector::analyze_query_param_variations(&mocks);
        assert!(analysis.has_variations);
        assert!(analysis.has_common_base_path);
    }
}
