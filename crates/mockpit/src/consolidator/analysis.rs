//! Response analysis and pagination detection for mock consolidation

use crate::Result;
use crate::config::MockConfig;
use crate::consolidator::pattern::QueryParamAnalysis;
use crate::type_detector::{FieldType, TypeDetector};
use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value as JsonValue;
use std::sync::LazyLock;

#[allow(clippy::expect_used)] // Static regex literal -- panic on invalid pattern is correct
static PATH_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(\d+)(?:/|\?|$)").expect("Failed to compile path ID regex"));

/// Pagination pattern detected in responses
#[derive(Debug, Clone)]
pub struct PaginationPattern {
    /// Total count field (e.g., "total_count", "total", "count")
    pub total_field: Option<String>,
    /// Offset field (e.g., "offset", "skip")
    pub offset_field: Option<String>,
    /// Limit field (e.g., "limit", "per_page", "page_size")
    pub limit_field: Option<String>,
    /// Next marker/cursor field (e.g., "next_marker", "next_cursor", "next_url")
    pub next_field: Option<String>,
    /// Previous marker/cursor field (e.g., "prev_marker", "prev_cursor", "prev_url")
    pub prev_field: Option<String>,
    /// Has more field (e.g., "has_more", "has_next")
    pub has_more_field: Option<String>,
    /// Detected total value from samples (for store_get_or_set default)
    pub sample_total: Option<i64>,
    /// Pagination type: offset-based, cursor-based, or page-based
    pub pagination_type: PaginationType,
    /// Static query parameters (non-pagination params that should be preserved in URLs)
    pub static_query_params: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationType {
    Offset,
    Cursor,
    Page,
}

/// Analysis of response patterns in a group
#[derive(Debug)]
pub struct ResponseAnalysis {
    /// Fields that vary across responses
    pub varying_fields: Vec<(String, FieldType)>,
    /// Fields that are constant across all responses
    pub constant_fields: Vec<(String, JsonValue)>,
    /// Whether responses have matching IDs with path IDs
    pub has_matching_path_ids: bool,
    /// Whether responses are JSON
    pub is_json: bool,
    /// Top-level type (object, array, etc.)
    pub top_level_type: String,
    /// Detected pagination pattern (if any)
    pub pagination_pattern: Option<PaginationPattern>,
}

/// Analysis of GraphQL variables across a group of mocks
#[derive(Debug, Clone)]
pub struct GraphQLVariableAnalysis {
    /// Variables that vary across mocks (e.g., `["id", "input.role"]`)
    pub varying_variables: Vec<String>,
    /// Variables that are constant across all mocks with their values
    pub constant_variables: Vec<(String, JsonValue)>,
    /// Whether any of the mocks have variables
    pub has_variables: bool,
    /// Whether there are variables that could be used for template generation
    pub has_varying_variables: bool,
}

impl GraphQLVariableAnalysis {
    /// Create an empty analysis (for non-GraphQL or no variables)
    pub fn empty() -> Self {
        Self {
            varying_variables: vec![],
            constant_variables: vec![],
            has_variables: false,
            has_varying_variables: false,
        }
    }
}

// Type conversions to bdg-mock-codegen types

impl From<&PaginationPattern> for crate::codegen::PaginationInfo {
    fn from(pattern: &PaginationPattern) -> Self {
        crate::codegen::PaginationInfo {
            total_field: pattern.total_field.clone(),
            offset_field: pattern.offset_field.clone(),
            limit_field: pattern.limit_field.clone(),
            next_field: pattern.next_field.clone(),
            prev_field: pattern.prev_field.clone(),
            has_more_field: pattern.has_more_field.clone(),
            sample_total: pattern.sample_total,
            pagination_type: match pattern.pagination_type {
                PaginationType::Offset => crate::codegen::PaginationType::Offset,
                PaginationType::Cursor => crate::codegen::PaginationType::Cursor,
                PaginationType::Page => crate::codegen::PaginationType::Page,
            },
            static_query_params: pattern.static_query_params.clone(),
        }
    }
}

impl From<&ResponseAnalysis> for crate::codegen::ResponseStructure {
    fn from(analysis: &ResponseAnalysis) -> Self {
        crate::codegen::ResponseStructure {
            varying_fields: analysis.varying_fields.clone(),
            constant_fields: analysis.constant_fields.clone(),
            has_matching_path_ids: analysis.has_matching_path_ids,
            is_json: analysis.is_json,
            top_level_type: analysis.top_level_type.clone(),
            pagination: analysis
                .pagination_pattern
                .as_ref()
                .map(std::convert::Into::into),
        }
    }
}

impl From<&GraphQLVariableAnalysis> for crate::codegen::GraphQLVariableInfo {
    fn from(analysis: &GraphQLVariableAnalysis) -> Self {
        crate::codegen::GraphQLVariableInfo {
            varying_variables: analysis.varying_variables.clone(),
            constant_variables: analysis.constant_variables.clone(),
            has_variables: analysis.has_variables,
            has_varying_variables: analysis.has_varying_variables,
        }
    }
}

/// Response analyzer for detecting patterns in mock responses
pub struct ResponseAnalyzer {
    type_detector: TypeDetector,
    enable_stateful_pagination: bool,
}

impl ResponseAnalyzer {
    pub fn new(enable_stateful_pagination: bool) -> Self {
        Self {
            type_detector: TypeDetector::new(),
            enable_stateful_pagination,
        }
    }

    /// Analyze response patterns across a group to detect field types
    #[allow(clippy::indexing_slicing)] // Indices are bounds-checked: `responses.is_empty()` guard before `responses[0]`
    pub fn analyze_response_patterns(&self, group: &[MockConfig]) -> Result<ResponseAnalysis> {
        let responses: Vec<JsonValue> = group
            .iter()
            .filter_map(|mock| {
                mock.response_config
                    .as_ref()
                    .and_then(|rc| rc.body())
                    .and_then(|body| serde_json::from_str(body).ok())
            })
            .collect();

        if responses.is_empty() {
            return Ok(ResponseAnalysis {
                varying_fields: vec![],
                constant_fields: vec![],
                has_matching_path_ids: false,
                is_json: false,
                top_level_type: "text".to_string(),
                pagination_pattern: None,
            });
        }

        let top_level_type = match &responses[0] {
            JsonValue::Object(_) => "object",
            JsonValue::Array(_) => "array",
            _ => "primitive",
        }
        .to_string();

        if matches!(responses[0], JsonValue::Array(_)) {
            return Ok(self.analyze_top_level_array_responses(&responses));
        }

        if !matches!(responses[0], JsonValue::Object(_)) {
            return Ok(ResponseAnalysis {
                varying_fields: vec![],
                constant_fields: vec![],
                has_matching_path_ids: false,
                is_json: true,
                top_level_type,
                pagination_pattern: None,
            });
        }

        let response_refs: Vec<&JsonValue> = responses.iter().collect();
        let (varying_fields, constant_fields) = self.analyze_object_fields(&response_refs);

        let has_matching_path_ids = Self::check_matching_path_ids(group, &responses);
        let pagination_pattern = self.detect_pagination_pattern(&responses, group);

        Ok(ResponseAnalysis {
            varying_fields,
            constant_fields,
            has_matching_path_ids,
            is_json: true,
            top_level_type,
            pagination_pattern,
        })
    }

    /// Analyze top-level array responses (e.g., GET /users -> [{...}, {...}])
    fn analyze_top_level_array_responses(&self, responses: &[JsonValue]) -> ResponseAnalysis {
        let all_objects: Vec<&JsonValue> = responses
            .iter()
            .filter_map(|r| r.as_array())
            .flatten()
            .filter(|v| v.is_object())
            .collect();

        if all_objects.is_empty() {
            return ResponseAnalysis {
                varying_fields: vec![],
                constant_fields: vec![],
                has_matching_path_ids: false,
                is_json: true,
                top_level_type: "array".to_string(),
                pagination_pattern: None,
            };
        }

        let (varying_fields, constant_fields) = self.analyze_object_fields(&all_objects);

        ResponseAnalysis {
            varying_fields,
            constant_fields,
            has_matching_path_ids: false,
            is_json: true,
            top_level_type: "array".to_string(),
            pagination_pattern: None,
        }
    }

    /// Extract common field analysis logic for objects
    #[allow(clippy::type_complexity, clippy::indexing_slicing)] // `.windows(2)` guarantees 2-element slices; values[0] guarded by non-empty check
    pub fn analyze_object_fields(
        &self,
        objects: &[&JsonValue],
    ) -> (Vec<(String, FieldType)>, Vec<(String, JsonValue)>) {
        let mut all_fields = FxHashSet::default();
        for obj in objects {
            if let JsonValue::Object(map) = obj {
                all_fields.extend(map.keys().cloned());
            }
        }

        let mut varying_fields = Vec::new();
        let mut constant_fields = Vec::new();

        for field in all_fields {
            let values: Vec<&JsonValue> = objects
                .iter()
                .filter_map(|obj| obj.as_object().and_then(|map| map.get(&field)))
                .collect();

            if values.is_empty() {
                continue;
            }

            let all_same = values.windows(2).all(|w| w[0] == w[1]);

            if all_same {
                constant_fields.push((field.clone(), values[0].clone()));
            } else {
                let (field_type, _confidence) = self.type_detector.detect_type(&field, &values);
                varying_fields.push((field, field_type));
            }
        }

        (varying_fields, constant_fields)
    }

    /// Check if response IDs match path IDs
    fn check_matching_path_ids(group: &[MockConfig], responses: &[JsonValue]) -> bool {
        if group.len() != responses.len() {
            return false;
        }

        let path_id_regex = &*PATH_ID_REGEX;

        for (mock, response) in group.iter().zip(responses.iter()) {
            let url_pattern = mock
                .match_config
                .as_ref()
                .and_then(|mc| mc.urls.first().or(mc.url.as_ref()));

            if let Some(url_pattern) = url_pattern {
                let url = url_pattern.strip_prefix("exact:").unwrap_or(url_pattern);

                if let Some(caps) = path_id_regex.captures(url)
                    && let Some(path_id) = caps
                        .get(1)
                        .and_then(|m: regex::Match<'_>| m.as_str().parse::<i64>().ok())
                    && let Some(response_id) =
                        response.get("id").and_then(serde_json::Value::as_i64)
                    && path_id == response_id
                {
                    return true;
                }
            }
        }

        false
    }

    /// Detect pagination patterns in responses with fuzzy field matching
    #[allow(clippy::indexing_slicing)] // `objects[0]` guarded by `objects.is_empty()` check
    pub fn detect_pagination_pattern(
        &self,
        responses: &[JsonValue],
        group: &[MockConfig],
    ) -> Option<PaginationPattern> {
        if responses.is_empty() || !self.enable_stateful_pagination {
            return None;
        }

        let objects: Vec<&serde_json::Map<String, JsonValue>> =
            responses.iter().filter_map(|r| r.as_object()).collect();

        if objects.is_empty() {
            return None;
        }

        // Exact match field names (most common)
        let total_fields = ["total_count", "total", "count", "total_items", "totalCount"];
        let offset_fields = ["offset", "skip", "start"];
        let limit_fields = ["limit", "per_page", "page_size", "perPage", "pageSize"];
        let next_fields = [
            "next_marker",
            "next_cursor",
            "next_url",
            "next",
            "nextMarker",
            "nextCursor",
        ];
        let prev_fields = [
            "prev_marker",
            "prev_cursor",
            "prev_url",
            "prev",
            "previous",
            "prevMarker",
            "prevCursor",
        ];
        let has_more_fields = ["has_more", "has_next", "hasMore", "hasNext"];

        // Try exact matching first
        let total_field =
            Self::find_field_fuzzy(objects[0], &total_fields, &["total", "count", "num"]);
        let offset_field =
            Self::find_field_fuzzy(objects[0], &offset_fields, &["offset", "skip", "start"]);
        let limit_field =
            Self::find_field_fuzzy(objects[0], &limit_fields, &["limit", "page", "size"]);
        let next_field = Self::find_field_fuzzy(objects[0], &next_fields, &["next"]);
        let prev_field = Self::find_field_fuzzy(objects[0], &prev_fields, &["prev", "previous"]);
        // Don't use "next" as fallback for has_more - it conflicts with next_field
        let has_more_field = Self::find_field_fuzzy(objects[0], &has_more_fields, &["more"]);

        if total_field.is_none() && next_field.is_none() && prev_field.is_none() {
            return None;
        }

        // Determine pagination type based on fields and their values
        let pagination_type = if next_field.is_some() || prev_field.is_some() {
            // Check if next/prev contain URLs (page-based) vs cursors/tokens
            if let Some(ref next_f) = next_field {
                if let Some(val) = objects[0].get(next_f) {
                    if let Some(next_str) = val.as_str() {
                        // If next field contains a URL, it's page-based pagination
                        if next_str.starts_with("http://")
                            || next_str.starts_with("https://")
                            || next_str.contains("?page=")
                        {
                            PaginationType::Page
                        } else {
                            // Otherwise it's cursor-based (tokens, markers, etc.)
                            PaginationType::Cursor
                        }
                    } else if val.is_number() {
                        PaginationType::Page
                    } else {
                        PaginationType::Cursor
                    }
                } else {
                    PaginationType::Cursor
                }
            } else {
                PaginationType::Offset
            }
        } else if offset_field.is_some() {
            PaginationType::Offset
        } else {
            // Has limit but no offset/next/prev - assume offset-based
            PaginationType::Offset
        };

        // Calculate the maximum total seen across all responses (not just first)
        let sample_total = total_field.as_ref().and_then(|field| {
            objects
                .iter()
                .filter_map(|obj| obj.get(field).and_then(serde_json::Value::as_i64))
                .max()
        });

        let query_analysis =
            crate::consolidator::pattern::PatternDetector::analyze_query_param_variations(group);
        let has_pagination_query_params = Self::has_pagination_params(&query_analysis);

        if has_pagination_query_params
            || (total_field.is_some() && (offset_field.is_some() || next_field.is_some()))
        {
            // Extract static query params from next/previous URLs
            let static_query_params = Self::extract_static_query_params(
                &objects,
                next_field.as_ref(),
                prev_field.as_ref(),
                &query_analysis,
            );

            Some(PaginationPattern {
                total_field,
                offset_field,
                limit_field,
                next_field,
                prev_field,
                has_more_field,
                sample_total,
                pagination_type,
                static_query_params,
            })
        } else {
            None
        }
    }

    fn has_pagination_params(query_analysis: &QueryParamAnalysis) -> bool {
        query_analysis.varying_params.iter().any(|p| {
            p == "offset"
                || p == "limit"
                || p == "page"
                || p == "per_page"
                || p == "marker"
                || p == "cursor"
        })
    }

    /// Extract static (non-pagination) query parameters from next/previous URLs
    fn extract_static_query_params(
        objects: &[&serde_json::Map<String, JsonValue>],
        next_field: Option<&String>,
        prev_field: Option<&String>,
        _query_analysis: &crate::consolidator::pattern::QueryParamAnalysis,
    ) -> String {
        // Try to get a sample URL from next or previous field
        let sample_url = next_field
            .and_then(|field| {
                objects
                    .iter()
                    .find_map(|obj| obj.get(field).and_then(|v| v.as_str()))
            })
            .or_else(|| {
                prev_field.and_then(|field| {
                    objects
                        .iter()
                        .find_map(|obj| obj.get(field).and_then(|v| v.as_str()))
                })
            });

        if let Some(url) = sample_url {
            // Parse URL and extract query params
            if let Some(query_start) = url.find('?') {
                let query_string = url.get(query_start + 1..).unwrap_or("");

                // Parse query params and filter out pagination-related ones
                let pagination_params = [
                    "page",
                    "limit",
                    "offset",
                    "per_page",
                    "cursor",
                    "marker",
                    "skip",
                    "page_size",
                ];
                let static_params: Vec<String> = query_string
                    .split('&')
                    .filter(|param| {
                        if let Some(key) = param.split('=').next() {
                            !pagination_params.contains(&key)
                        } else {
                            false
                        }
                    })
                    .map(std::string::ToString::to_string)
                    .collect();

                return static_params.join("&");
            }
        }

        // No static params found
        String::new()
    }

    /// Find field with fuzzy matching - tries exact match first, then substring match
    fn find_field_fuzzy(
        obj: &serde_json::Map<String, JsonValue>,
        exact_matches: &[&str],
        fuzzy_patterns: &[&str],
    ) -> Option<String> {
        // Try exact matches first
        for &field in exact_matches {
            if obj.contains_key(field) {
                return Some(field.to_string());
            }
        }

        // Try fuzzy/substring matching
        for key in obj.keys() {
            let key_lower = key.to_lowercase();
            for &pattern in fuzzy_patterns {
                // Match if field contains the pattern
                // e.g., "totalRecords" matches "total", "record_count" matches "count"
                if key_lower.contains(pattern) {
                    return Some(key.clone());
                }
            }
        }

        None
    }

    /// Analyze GraphQL variables across a group of mocks to detect varying vs constant variables
    ///
    /// This is used to determine which variables should be extracted for template generation.
    /// For example, if all mocks have the same GraphQL operation but different variable values,
    /// we can create a template that extracts those variables from the request.
    #[allow(clippy::indexing_slicing)] // `.windows(2)` guarantees 2-element slices; values[0] guarded by non-empty check
    pub fn analyze_graphql_variables(group: &[MockConfig]) -> GraphQLVariableAnalysis {
        use rustc_hash::FxHashMap;

        // Collect all variables from all mocks
        let mut all_variables: FxHashMap<String, Vec<JsonValue>> = FxHashMap::default();

        for mock in group {
            if let Some(ref match_config) = mock.match_config
                && let Some(ref graphql) = match_config.graphql
            {
                // Extract variables from the GraphQL config
                let variables = Self::extract_graphql_variables(graphql);

                for (key, value) in variables {
                    all_variables.entry(key).or_default().push(value);
                }
            }
        }

        if all_variables.is_empty() {
            return GraphQLVariableAnalysis::empty();
        }

        // Analyze which variables vary vs are constant
        let mut varying_variables = Vec::new();
        let mut constant_variables = Vec::new();

        for (key, values) in &all_variables {
            // Check if all values are the same
            if values.len() == 1 || values.windows(2).all(|w| w[0] == w[1]) {
                // Constant variable - all values are the same
                constant_variables.push((key.clone(), values[0].clone()));
            } else {
                // Varying variable - values differ
                varying_variables.push(key.clone());
            }
        }

        GraphQLVariableAnalysis {
            varying_variables: varying_variables.clone(),
            constant_variables,
            has_variables: true,
            has_varying_variables: !varying_variables.is_empty(),
        }
    }

    /// Extract variables from a GraphQL config
    fn extract_graphql_variables(
        graphql: &crate::config::matcher::GraphQLMatchConfig,
    ) -> FxHashMap<String, JsonValue> {
        use crate::config::matcher::GraphQLMatchConfig;

        match graphql {
            GraphQLMatchConfig::Structured { variables, .. } => variables.clone(),
            _ => FxHashMap::default(),
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
    use crate::config::{MatchConfig, ReturnConfig};
    use rustc_hash::FxHashMap;

    #[test]
    fn test_pagination_pattern_detection_offset_based() {
        let analyzer = ResponseAnalyzer::new(true);

        let mocks = vec![
            MockConfig {
                id: "test-1".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["GET".to_string()],
                    urls: vec!["exact:/api/items?offset=0&limit=10".to_string()],
                    ..Default::default()
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(200),
                    headers: FxHashMap::default(),
                    body: Some(
                        r#"{"total_count": 100, "offset": 0, "limit": 10, "items": []}"#
                            .to_string(),
                    ),
                    template: None,
                    file: None,
                    template_file: None,
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: None,
            },
            MockConfig {
                id: "test-2".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["GET".to_string()],
                    urls: vec!["exact:/api/items?offset=10&limit=10".to_string()],
                    ..Default::default()
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(200),
                    headers: FxHashMap::default(),
                    body: Some(
                        r#"{"total_count": 100, "offset": 10, "limit": 10, "items": []}"#
                            .to_string(),
                    ),
                    template: None,
                    file: None,
                    template_file: None,
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: None,
            },
        ];

        let responses: Vec<JsonValue> = mocks
            .iter()
            .filter_map(|m| {
                m.response_config
                    .as_ref()
                    .and_then(|rc| rc.body())
                    .and_then(|b| serde_json::from_str(b).ok())
            })
            .collect();

        let pattern = analyzer.detect_pagination_pattern(&responses, &mocks);
        assert!(pattern.is_some(), "Should detect pagination pattern");

        let pattern = pattern.unwrap();
        assert_eq!(pattern.pagination_type, PaginationType::Offset);
        assert!(pattern.total_field.is_some());
        assert!(pattern.offset_field.is_some());
        assert!(pattern.limit_field.is_some());
        assert_eq!(pattern.sample_total, Some(100));
    }

    #[test]
    fn test_extract_static_query_params_from_next_url() {
        let json_val = serde_json::json!({
          "count": 80,
          "next": "https://api.example.com/docs?status=active&sort=desc&page=2&limit=10",
          "previous": null,
          "results": []
        });
        let objects = vec![json_val.as_object().unwrap()];

        let next_field = Some("next".to_string());
        let prev_field = Some("previous".to_string());
        let query_analysis = crate::consolidator::pattern::QueryParamAnalysis {
            has_variations: true,
            has_common_base_path: true,
            varying_params: vec![],
            constant_params: vec![],
            variation_count: 1,
        };

        let static_params = ResponseAnalyzer::extract_static_query_params(
            &objects,
            next_field.as_ref(),
            prev_field.as_ref(),
            &query_analysis,
        );

        assert_eq!(static_params, "status=active&sort=desc");
    }

    #[test]
    fn test_extract_static_query_params_filters_pagination_params() {
        let json_val = serde_json::json!({
          "total": 100,
          "next": "https://api.example.com/items?filter=enabled&offset=20&limit=10&include=meta",
          "results": []
        });
        let objects = vec![json_val.as_object().unwrap()];

        let next_field = Some("next".to_string());
        let prev_field = None;
        let query_analysis = crate::consolidator::pattern::QueryParamAnalysis {
            has_variations: false,
            has_common_base_path: true,
            varying_params: vec![],
            constant_params: vec![],
            variation_count: 1,
        };

        let static_params = ResponseAnalyzer::extract_static_query_params(
            &objects,
            next_field.as_ref(),
            prev_field.as_ref(),
            &query_analysis,
        );

        // Should include filter and include, but NOT offset or limit
        assert_eq!(static_params, "filter=enabled&include=meta");
    }

    #[test]
    fn test_extract_static_query_params_from_previous_url() {
        let json_val = serde_json::json!({
          "count": 50,
          "next": null,
          "previous": "https://api.example.com/data?q=test&category=docs&page=1&limit=20",
          "results": []
        });
        let objects = vec![json_val.as_object().unwrap()];

        let next_field = Some("next".to_string());
        let prev_field = Some("previous".to_string());
        let query_analysis = crate::consolidator::pattern::QueryParamAnalysis {
            has_variations: false,
            has_common_base_path: true,
            varying_params: vec![],
            constant_params: vec![],
            variation_count: 1,
        };

        let static_params = ResponseAnalyzer::extract_static_query_params(
            &objects,
            next_field.as_ref(),
            prev_field.as_ref(),
            &query_analysis,
        );

        // Should extract from previous URL when next is null
        assert_eq!(static_params, "q=test&category=docs");
    }

    #[test]
    fn test_extract_static_query_params_no_urls() {
        let json_val = serde_json::json!({
          "total": 10,
          "results": []
        });
        let objects = vec![json_val.as_object().unwrap()];

        let next_field = None;
        let prev_field = None;
        let query_analysis = crate::consolidator::pattern::QueryParamAnalysis {
            has_variations: false,
            has_common_base_path: true,
            varying_params: vec![],
            constant_params: vec![],
            variation_count: 1,
        };

        let static_params = ResponseAnalyzer::extract_static_query_params(
            &objects,
            next_field.as_ref(),
            prev_field.as_ref(),
            &query_analysis,
        );

        // Should return empty string when no URLs available
        assert_eq!(static_params, "");
    }

    #[test]
    fn test_pagination_pattern_has_static_query_params() {
        let analyzer = ResponseAnalyzer::new(true);

        let mocks = vec![
            MockConfig {
                id: "test-1".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["GET".to_string()],
                    urls: vec![
                        "exact:/api/search?q=test&status=active&page=1&limit=10".to_string(),
                    ],
                    ..Default::default()
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(200),
                    headers: FxHashMap::default(),
                    body: Some(
                        r#"{
              "count": 100,
              "next": "https://api.example.com/search?q=test&status=active&page=2&limit=10",
              "previous": null,
              "results": [{"id": 1}]
            }"#
                        .to_string(),
                    ),
                    template: None,
                    file: None,
                    template_file: None,
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: None,
            },
            MockConfig {
                id: "test-2".into(),
                description: None,
                priority: 99,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["GET".to_string()],
                    urls: vec![
                        "exact:/api/search?q=test&status=active&page=2&limit=10".to_string(),
                    ],
                    ..Default::default()
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(200),
                    headers: FxHashMap::default(),
                    body: Some(
                        r#"{
              "count": 100,
              "next": "https://api.example.com/search?q=test&status=active&page=3&limit=10",
              "previous": "https://api.example.com/search?q=test&status=active&page=1&limit=10",
              "results": [{"id": 2}]
            }"#
                        .to_string(),
                    ),
                    template: None,
                    file: None,
                    template_file: None,
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: None,
            },
        ];

        let responses: Vec<serde_json::Value> = mocks
            .iter()
            .filter_map(|m| {
                m.response_config
                    .as_ref()
                    .and_then(|rc| rc.body())
                    .and_then(|b| serde_json::from_str(b).ok())
            })
            .collect();

        let pattern = analyzer.detect_pagination_pattern(&responses, &mocks);
        assert!(pattern.is_some(), "Should detect pagination pattern");

        let pattern = pattern.unwrap();
        assert_eq!(pattern.pagination_type, PaginationType::Page);
        assert_eq!(pattern.static_query_params, "q=test&status=active");
    }

    #[test]
    fn test_no_duplicate_next_field_fuzzy_matching() {
        // This test verifies that "next" is NOT matched as has_more_field
        let analyzer = ResponseAnalyzer::new(true);

        let mocks = vec![MockConfig {
            id: "test-1".into(),
            description: None,
            priority: 100,
            enabled: true,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec!["GET".to_string()],
                urls: vec!["exact:/api/docs?page=1&limit=10".to_string()],
                ..Default::default()
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: FxHashMap::default(),
                body: Some(
                    r#"{
            "count": 80,
            "next": "https://api.example.com/docs?page=2&limit=10",
            "previous": null,
            "results": []
          }"#
                    .to_string(),
                ),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            patch: None,
            delay: None,
        }];

        let responses: Vec<serde_json::Value> = mocks
            .iter()
            .filter_map(|m| {
                m.response_config
                    .as_ref()
                    .and_then(|rc| rc.body())
                    .and_then(|b| serde_json::from_str(b).ok())
            })
            .collect();

        let pattern = analyzer.detect_pagination_pattern(&responses, &mocks);
        assert!(pattern.is_some());

        let pattern = pattern.unwrap();
        // "next" should be detected as next_field, NOT as has_more_field
        assert_eq!(pattern.next_field, Some("next".to_string()));
        assert_eq!(pattern.has_more_field, None);
    }

    // GraphQL Variable Analysis Tests

    #[test]
    fn test_graphql_variable_analysis_varying_variables() {
        use crate::config::matcher::{GraphQLMatchConfig, MatchConfig};

        let mocks = vec![
            MockConfig {
                id: "get-user-1".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec!["/graphql".to_string()],
                    graphql: Some(GraphQLMatchConfig::Structured {
                        query: Some("GetUser".to_string()),
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        operation: None,
                        variables: {
                            let mut vars = FxHashMap::default();
                            vars.insert("id".to_string(), serde_json::json!("123"));
                            vars
                        },
                    }),
                    ..Default::default()
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
            },
            MockConfig {
                id: "get-user-2".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec!["/graphql".to_string()],
                    graphql: Some(GraphQLMatchConfig::Structured {
                        query: Some("GetUser".to_string()),
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        operation: None,
                        variables: {
                            let mut vars = FxHashMap::default();
                            vars.insert("id".to_string(), serde_json::json!("456"));
                            vars
                        },
                    }),
                    ..Default::default()
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
            },
        ];

        let analysis = ResponseAnalyzer::analyze_graphql_variables(&mocks);

        assert!(analysis.has_variables);
        assert!(analysis.has_varying_variables);
        assert_eq!(analysis.varying_variables.len(), 1);
        assert!(analysis.varying_variables.contains(&"id".to_string()));
        assert_eq!(analysis.constant_variables.len(), 0);
    }

    #[test]
    fn test_graphql_variable_analysis_constant_variables() {
        use crate::config::matcher::{GraphQLMatchConfig, MatchConfig};

        let mocks = vec![
            MockConfig {
                id: "get-user-1".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec!["/graphql".to_string()],
                    graphql: Some(GraphQLMatchConfig::Structured {
                        query: Some("GetUser".to_string()),
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        operation: None,
                        variables: {
                            let mut vars = FxHashMap::default();
                            vars.insert("status".to_string(), serde_json::json!("active"));
                            vars
                        },
                    }),
                    ..Default::default()
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
            },
            MockConfig {
                id: "get-user-2".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec!["/graphql".to_string()],
                    graphql: Some(GraphQLMatchConfig::Structured {
                        query: Some("GetUser".to_string()),
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        operation: None,
                        variables: {
                            let mut vars = FxHashMap::default();
                            vars.insert("status".to_string(), serde_json::json!("active"));
                            vars
                        },
                    }),
                    ..Default::default()
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
            },
        ];

        let analysis = ResponseAnalyzer::analyze_graphql_variables(&mocks);

        assert!(analysis.has_variables);
        assert!(!analysis.has_varying_variables);
        assert_eq!(analysis.varying_variables.len(), 0);
        assert_eq!(analysis.constant_variables.len(), 1);
        assert_eq!(
            analysis.constant_variables[0],
            ("status".to_string(), serde_json::json!("active"))
        );
    }

    #[test]
    fn test_graphql_variable_analysis_mixed_variables() {
        use crate::config::matcher::{GraphQLMatchConfig, MatchConfig};

        let mocks = vec![
            MockConfig {
                id: "get-user-1".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec!["/graphql".to_string()],
                    graphql: Some(GraphQLMatchConfig::Structured {
                        query: Some("GetUser".to_string()),
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        operation: None,
                        variables: {
                            let mut vars = FxHashMap::default();
                            vars.insert("id".to_string(), serde_json::json!("123"));
                            vars.insert("includeDetails".to_string(), serde_json::json!(true));
                            vars
                        },
                    }),
                    ..Default::default()
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
            },
            MockConfig {
                id: "get-user-2".into(),
                description: None,
                priority: 100,
                enabled: true,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec!["POST".to_string()],
                    urls: vec!["/graphql".to_string()],
                    graphql: Some(GraphQLMatchConfig::Structured {
                        query: Some("GetUser".to_string()),
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        operation: None,
                        variables: {
                            let mut vars = FxHashMap::default();
                            vars.insert("id".to_string(), serde_json::json!("456"));
                            vars.insert("includeDetails".to_string(), serde_json::json!(true));
                            vars
                        },
                    }),
                    ..Default::default()
                }),
                request: None,
                response_config: None,
                patch: None,
                delay: None,
            },
        ];

        let analysis = ResponseAnalyzer::analyze_graphql_variables(&mocks);

        assert!(analysis.has_variables);
        assert!(analysis.has_varying_variables);
        assert_eq!(analysis.varying_variables.len(), 1);
        assert!(analysis.varying_variables.contains(&"id".to_string()));
        assert_eq!(analysis.constant_variables.len(), 1);
        assert_eq!(
            analysis.constant_variables[0],
            ("includeDetails".to_string(), serde_json::json!(true))
        );
    }

    #[test]
    fn test_graphql_variable_analysis_no_variables() {
        use crate::config::matcher::{GraphQLMatchConfig, MatchConfig};

        let mocks = vec![MockConfig {
            id: "get-schema".into(),
            description: None,
            priority: 100,
            enabled: true,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec!["POST".to_string()],
                urls: vec!["/graphql".to_string()],
                graphql: Some(GraphQLMatchConfig::Simple("query".to_string())),
                ..Default::default()
            }),
            request: None,
            response_config: None,
            patch: None,
            delay: None,
        }];

        let analysis = ResponseAnalyzer::analyze_graphql_variables(&mocks);

        assert!(!analysis.has_variables);
        assert!(!analysis.has_varying_variables);
        assert_eq!(analysis.varying_variables.len(), 0);
        assert_eq!(analysis.constant_variables.len(), 0);
    }

    #[test]
    fn test_graphql_variable_analysis_non_graphql_mocks() {
        use crate::config::MatchConfig;

        let mocks = vec![MockConfig {
            id: "rest-endpoint".into(),
            description: None,
            priority: 100,
            enabled: true,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec!["GET".to_string()],
                urls: vec!["/api/users".to_string()],
                graphql: None,
                ..Default::default()
            }),
            request: None,
            response_config: None,
            patch: None,
            delay: None,
        }];

        let analysis = ResponseAnalyzer::analyze_graphql_variables(&mocks);

        assert!(!analysis.has_variables);
        assert!(!analysis.has_varying_variables);
        assert_eq!(analysis.varying_variables.len(), 0);
        assert_eq!(analysis.constant_variables.len(), 0);
    }
}
