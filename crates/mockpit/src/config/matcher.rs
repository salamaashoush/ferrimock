//! Request matching configuration

use super::patterns::{is_valid_http_method, parse_url_pattern};
use crate::types::{
    BodyMatcher, HeaderMatcher, QueryMatcher, RequestMatcher, SmallVec, UrlPattern,
};
use http::Method;
use http::header::HeaderName;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Match configuration (flat syntax)
/// Supports both singular and plural forms for methods/urls
/// Also supports ultra-flat syntax:
/// - `match = "GET /url"` - parses method and URL from string
/// - `match.GET = "/url"` - HTTP method as key
#[derive(Debug, Clone, Default)]
pub struct MatchConfig {
    /// Single HTTP method (e.g., "GET")
    pub method: Option<String>,

    /// Multiple HTTP methods
    pub methods: Vec<String>,

    /// Single URL pattern
    pub url: Option<String>,

    /// Multiple URL patterns
    pub urls: Vec<String>,

    /// Header matching conditions
    pub headers: FxHashMap<String, HeaderMatchConfig>,

    /// Query parameter matching (inline syntax)
    pub query: FxHashMap<String, String>,

    /// Body matcher configuration (inline syntax with auto-detection)
    pub body: FxHashMap<String, serde_json::Value>,

    /// GraphQL matcher configuration
    pub graphql: Option<GraphQLMatchConfig>,
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for MatchConfig {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "MatchConfig".into()
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let value = serde_json::json!({
          "description": "Request matching configuration. Supports string shorthand ('GET /url') or structured object.",
          "oneOf": [
            {
              "type": "string",
              "description": "Ultra-flat syntax: 'METHOD /url' (e.g., 'GET /api/users')"
            },
            {
              "type": "object",
              "description": "Structured match configuration with method/url/headers/query/body/graphql fields, or HTTP method as key (e.g., GET: '/url')",
              "properties": {
                "method": { "type": "string", "description": "Single HTTP method" },
                "methods": { "type": "array", "items": { "type": "string" }, "description": "Multiple HTTP methods" },
                "url": { "type": "string", "description": "Single URL pattern" },
                "urls": { "type": "array", "items": { "type": "string" }, "description": "Multiple URL patterns" },
                "headers": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Header matching conditions" },
                "query": { "type": "object", "additionalProperties": { "type": "string" }, "description": "Query parameter matching" },
                "body": { "type": "object", "description": "Body matcher (key prefix: $ for JSONPath, ~ for regex, @ for contains)" },
                "graphql": { "description": "GraphQL operation matcher" },
                "GET": { "type": "string", "description": "Method-as-key shortcut" },
                "POST": { "type": "string", "description": "Method-as-key shortcut" },
                "PUT": { "type": "string", "description": "Method-as-key shortcut" },
                "DELETE": { "type": "string", "description": "Method-as-key shortcut" },
                "PATCH": { "type": "string", "description": "Method-as-key shortcut" }
              }
            }
          ]
        });
        if let serde_json::Value::Object(map) = value {
            map.into()
        } else {
            serde_json::Map::new().into()
        }
    }
}

// Custom serialization for MatchConfig
impl serde::Serialize for MatchConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;

        if let Some(m) = &self.method {
            map.serialize_entry("method", m)?;
        }
        if !self.methods.is_empty() {
            map.serialize_entry("methods", &self.methods)?;
        }
        if let Some(u) = &self.url {
            map.serialize_entry("url", u)?;
        }
        if !self.urls.is_empty() {
            map.serialize_entry("urls", &self.urls)?;
        }
        if !self.headers.is_empty() {
            map.serialize_entry("headers", &self.headers)?;
        }
        if !self.query.is_empty() {
            map.serialize_entry("query", &self.query)?;
        }
        if !self.body.is_empty() {
            map.serialize_entry("body", &self.body)?;
        }
        if let Some(g) = &self.graphql {
            map.serialize_entry("graphql", g)?;
        }

        map.end()
    }
}

// Custom deserialization for MatchConfig
impl<'de> serde::Deserialize<'de> for MatchConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        #[serde(untagged)]
        #[allow(clippy::large_enum_variant)]
        enum Helper {
            /// Ultra-flat string syntax: "GET /url"
            String(String),

            /// Structured form with potential method shortcuts
            Structured {
                #[serde(default)]
                method: Option<String>,
                #[serde(default)]
                methods: Vec<String>,
                #[serde(default)]
                url: Option<String>,
                #[serde(default)]
                urls: Vec<String>,
                #[serde(default)]
                headers: FxHashMap<String, HeaderMatchConfig>,
                #[serde(default)]
                query: FxHashMap<String, String>,
                #[serde(default)]
                body: FxHashMap<String, serde_json::Value>,
                #[serde(default)]
                graphql: Option<GraphQLMatchConfig>,

                /// Capture any unknown fields for HTTP method shortcuts
                #[serde(flatten)]
                extra: FxHashMap<String, serde_json::Value>,
            },
        }

        let helper = Helper::deserialize(deserializer)?;

        match helper {
            // Parse "GET /url" or "POST /api/users"
            Helper::String(s) => {
                let parts: Vec<&str> = s.splitn(2, ' ').collect();
                if parts.len() != 2 {
                    return Err(D::Error::custom(format!(
                        "Invalid match string '{s}'. Expected format: 'METHOD /url'"
                    )));
                }

                let method = parts
                    .first()
                    .ok_or_else(|| D::Error::custom("missing method"))?
                    .trim()
                    .to_string();
                let url = parts
                    .get(1)
                    .ok_or_else(|| D::Error::custom("missing url"))?
                    .trim()
                    .to_string();

                // Validate HTTP method
                if !is_valid_http_method(&method) {
                    return Err(D::Error::custom(format!("Invalid HTTP method: {method}")));
                }

                Ok(MatchConfig {
                    method: Some(method),
                    url: Some(url),
                    ..Default::default()
                })
            }

            // Structured form with potential method shortcuts
            Helper::Structured {
                method,
                mut methods,
                url,
                mut urls,
                headers,
                query,
                body,
                graphql,
                extra,
            } => {
                // Check for HTTP method shortcuts (e.g., match.GET = "/url")
                for (key, value) in &extra {
                    if is_valid_http_method(key) {
                        // This is a method shortcut: match.GET = "/url"
                        methods.push(key.clone());

                        // Value should be a URL pattern (string)
                        if let Some(url_str) = value.as_str() {
                            urls.push(url_str.to_string());
                        } else {
                            return Err(D::Error::custom(format!(
                                "Method shortcut value must be a string URL pattern, got: {value:?}"
                            )));
                        }
                    }
                }

                Ok(MatchConfig {
                    method,
                    methods,
                    url,
                    urls,
                    headers,
                    query,
                    body,
                    graphql,
                })
            }
        }
    }
}

impl MatchConfig {
    /// Convert to RequestConfig for backward compatibility
    pub fn into_request_config(self) -> RequestConfig {
        let mut methods = self.methods;
        if let Some(method) = self.method {
            methods.push(method);
        }

        let mut url_patterns = self.urls;
        if let Some(url) = self.url {
            url_patterns.push(url);
        }

        // Convert body map to BodyMatcherConfig if present
        let body_matcher = if self.body.is_empty() {
            None
        } else {
            Some(BodyMatcherConfig::Inline(self.body))
        };

        RequestConfig {
            methods,
            url_patterns,
            headers: self.headers,
            query: self.query,
            body_matcher,
            graphql_matcher: self.graphql,
        }
    }
}

/// Request matching configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestConfig {
    /// HTTP methods to match (empty = match all)
    #[serde(default)]
    pub methods: Vec<String>,

    /// URL patterns to match
    #[serde(default)]
    pub url_patterns: Vec<String>,

    /// Header matching conditions
    #[serde(default)]
    pub headers: FxHashMap<String, HeaderMatchConfig>,

    /// Query parameter matching (inline syntax)
    #[serde(default)]
    pub query: FxHashMap<String, String>,

    /// Body matcher configuration
    #[serde(default)]
    pub body_matcher: Option<BodyMatcherConfig>,

    /// GraphQL matcher configuration
    #[serde(default)]
    pub graphql_matcher: Option<GraphQLMatchConfig>,
}

impl RequestConfig {
    /// Convert to a RequestMatcher
    pub fn into_request_matcher(self) -> crate::Result<RequestMatcher> {
        // Parse methods
        let methods: Result<SmallVec<[Method; 2]>, _> = self
            .methods
            .iter()
            .map(|m| Method::from_str(m).map_err(|e| crate::mp_err!("Invalid method '{m}': {e}")))
            .collect();
        let methods = methods?;

        // Parse URL patterns
        let url_patterns: Result<SmallVec<[UrlPattern; 1]>, _> = self
            .url_patterns
            .iter()
            .map(|p| parse_url_pattern(p))
            .collect();
        let url_patterns = url_patterns?;

        // Parse header matchers
        let header_matchers: Result<SmallVec<[HeaderMatcher; 2]>, _> = self
            .headers
            .into_iter()
            .map(|(name, config)| {
                let header_name = HeaderName::from_str(&name)
                    .map_err(|e| crate::mp_err!("Invalid header name '{name}': {e}"))?;
                config.into_header_matcher(header_name)
            })
            .collect();
        let header_matchers = header_matchers?;

        // Parse query parameter matchers from the query map with inline syntax support
        let query_matchers: crate::Result<SmallVec<[QueryMatcher; 2]>> = self
            .query
            .into_iter()
            .map(|(name, value)| {
                if let Some(regex_pattern) = value.strip_prefix('~') {
                    QueryMatcher::regex(name, regex_pattern)
                        .map_err(|e| crate::mp_err!("Invalid query regex: {e}"))
                } else if value == "?" {
                    Ok(QueryMatcher::present(name))
                } else if value == "!" {
                    Ok(QueryMatcher::absent(name))
                } else {
                    Ok(QueryMatcher::exact(name, value))
                }
            })
            .collect();
        let query_matchers = query_matchers?;

        // Parse body matcher
        let body_matcher = self
            .body_matcher
            .map(BodyMatcherConfig::into_body_matcher)
            .transpose()?;

        // Parse GraphQL matcher
        let graphql_matcher = self
            .graphql_matcher
            .map(GraphQLMatchConfig::into_graphql_matcher)
            .transpose()?;

        Ok(RequestMatcher {
            methods,
            url_patterns,
            header_matchers,
            query_matchers,
            body_matcher,
            graphql_matcher,
        })
    }
}

/// Header matching configuration - inline syntax only
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum HeaderMatchConfig {
    /// String value with inline syntax support:
    /// - Plain string: exact match
    /// - ~pattern: regex match
    /// - ?: header must be present
    /// - !: header must be absent
    Exact(String),
}

impl HeaderMatchConfig {
    pub fn into_header_matcher(self, name: HeaderName) -> crate::Result<HeaderMatcher> {
        match self {
            HeaderMatchConfig::Exact(value) => {
                // Check for inline matcher syntax prefixes
                if let Some(regex_pattern) = value.strip_prefix('~') {
                    // ~pattern = regex match
                    HeaderMatcher::regex(name, regex_pattern)
                        .map_err(|e| crate::mp_err!("Invalid header regex: {e}"))
                } else if value == "?" {
                    // ? = header must be present
                    Ok(HeaderMatcher::present(name))
                } else if value == "!" {
                    // ! = header must be absent
                    Ok(HeaderMatcher::absent(name))
                } else {
                    // Plain string = exact match
                    Ok(HeaderMatcher::exact(name, value))
                }
            }
        }
    }
}

/// Body matcher configuration - inline syntax only with auto-detection
///
/// Auto-detection based on key prefix:
/// - `"$.path" = value` - JSONPath ($ prefix)
/// - `"~pattern" = true` - Regex (~ prefix)
/// - `"@text" = true` - Contains (@ prefix)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum BodyMatcherConfig {
    /// Inline syntax - HashMap with auto-detection based on key prefixes
    Inline(
        #[allow(clippy::disallowed_types)]
        #[cfg_attr(
            feature = "schema",
            schemars(with = "std::collections::HashMap<String, serde_json::Value>")
        )]
        FxHashMap<String, serde_json::Value>,
    ),
}

impl BodyMatcherConfig {
    pub fn into_body_matcher(self) -> crate::Result<BodyMatcher> {
        match self {
            BodyMatcherConfig::Inline(map) => {
                // Parse inline syntax from the map
                // We expect exactly one entry for inline syntax
                if map.len() != 1 {
                    return Err(crate::mp_err!("Body matcher requires exactly one key-value pair"));
                }

                let Some((key, value)) = map.iter().next() else {
                    return Err(crate::mp_err!("Body matcher map is empty"));
                };

                // Check for legacy syntax (backward compatibility)
                if key == "contains" {
                    // Legacy syntax: contains = ["text1", "text2"] or contains = "text"
                    if let Some(arr) = value.as_array() {
                        // Array of strings - create an AND matcher for all of them
                        let text_list: Result<Vec<String>, _> = arr
                            .iter()
                            .map(|v| {
                                v.as_str()
                                    .ok_or_else(|| {
                                        "contains array must contain strings".to_string()
                                    })
                                    .map(std::string::ToString::to_string)
                            })
                            .collect();
                        let text_list = text_list?;

                        if text_list.is_empty() {
                            return Err(crate::mp_err!("contains array cannot be empty"));
                        }

                        // For multiple contains, we need to check all of them
                        // Create a combined matcher using the first one and verify others in matching
                        // For now, just use the first one as a simple contains check
                        // (Full AND logic would require changes to BodyMatcher type)
                        return Ok(BodyMatcher::contains(
                            text_list
                                .first()
                                .ok_or_else(|| crate::mp_err!("contains array cannot be empty"))?,
                        ));
                    } else if let Some(text) = value.as_str() {
                        // Single string
                        return Ok(BodyMatcher::contains(text));
                    }
                    return Err(crate::mp_err!("contains value must be a string or array of strings"));
                } else if key == "regex" {
                    // Legacy syntax: regex = "pattern"
                    if let Some(pattern) = value.as_str() {
                        return BodyMatcher::regex(pattern)
                            .map_err(|e| crate::mp_err!("Invalid regex pattern: {e}"));
                    }
                    return Err(crate::mp_err!("regex value must be a string"));
                } else if key == "json_path" {
                    // Legacy syntax: json_path = { "$.path" = "value" }
                    if let Some(obj) = value.as_object() {
                        // Should have exactly one entry
                        if obj.len() != 1 {
                            return Err(
                                "json_path object must have exactly one key-value pair".to_string().into()
                            );
                        }
                        let Some((path, expected_value)) = obj.iter().next() else {
                            return Err(crate::mp_err!("json_path object is empty"));
                        };
                        return Ok(BodyMatcher::json_path(path.clone(), expected_value.clone()));
                    }
                    return Err(crate::mp_err!("json_path value must be an object"));
                }

                // Auto-detect based on key prefix
                if let Some(regex_pattern) = key.strip_prefix('~') {
                    // ~pattern = regex match
                    BodyMatcher::regex(regex_pattern)
                        .map_err(|e| crate::mp_err!("Invalid regex pattern: {e}"))
                } else if let Some(contains_text) = key.strip_prefix('@') {
                    // @text = contains match
                    Ok(BodyMatcher::contains(contains_text))
                } else if key.starts_with('$') {
                    // $ prefix = JSONPath match
                    Ok(BodyMatcher::json_path(key.clone(), value.clone()))
                } else {
                    // No prefix: error - must use explicit prefix
                    Err(crate::mp_err!(
                        "Body matcher key '{key}' must start with $, ~, or @ for JSONPath, regex, or contains matching"
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod tests {
    use super::*;

    #[test]
    fn test_header_match_simple() {
        let yaml = r#"
url_patterns:
  - "/test"
headers:
  content-type: "application/json"
"#;

        let config: RequestConfig =
            serde_yaml::from_str(yaml).expect("Failed to parse YAML config");
        let matcher = config
            .into_request_matcher()
            .expect("Failed to convert to request matcher");

        assert_eq!(matcher.header_matchers.len(), 1);
    }

    #[test]
    fn test_body_matcher_contains() {
        let mut body_map = FxHashMap::default();
        body_map.insert("@test".to_string(), serde_json::Value::Bool(true));

        let config = RequestConfig {
            methods: vec![],
            url_patterns: vec![],
            headers: FxHashMap::default(),
            body_matcher: Some(BodyMatcherConfig::Inline(body_map)),
            query: FxHashMap::default(),
            graphql_matcher: None,
        };

        let matcher = config
            .into_request_matcher()
            .expect("Failed to convert to request matcher");
        assert!(matcher.body_matcher.is_some());
        assert!(
            matcher
                .body_matcher
                .expect("body_matcher should exist")
                .matches(b"this is a test", None)
        );
    }

    #[test]
    fn test_body_matcher_regex() {
        let mut body_map = FxHashMap::default();
        body_map.insert(
            r"~\d{3}-\d{3}-\d{4}".to_string(),
            serde_json::Value::Bool(true),
        );

        let config = RequestConfig {
            methods: vec![],
            url_patterns: vec![],
            headers: FxHashMap::default(),
            body_matcher: Some(BodyMatcherConfig::Inline(body_map)),
            query: FxHashMap::default(),
            graphql_matcher: None,
        };

        let matcher = config
            .into_request_matcher()
            .expect("Failed to convert to request matcher");
        assert!(matcher.body_matcher.is_some());
        assert!(
            matcher
                .body_matcher
                .expect("body_matcher should exist")
                .matches(b"Phone: 123-456-7890", None)
        );
    }

    #[test]
    fn test_body_matcher_json_path() {
        let mut body_map = FxHashMap::default();
        body_map.insert(
            "$.user.name".to_string(),
            serde_json::Value::String("John".to_string()),
        );

        let config = RequestConfig {
            methods: vec![],
            url_patterns: vec![],
            headers: FxHashMap::default(),
            body_matcher: Some(BodyMatcherConfig::Inline(body_map)),
            query: FxHashMap::default(),
            graphql_matcher: None,
        };

        let matcher = config
            .into_request_matcher()
            .expect("Failed to convert to request matcher");
        assert!(matcher.body_matcher.is_some());
        assert!(
            matcher
                .body_matcher
                .expect("body_matcher should exist")
                .matches(br#"{"user":{"name":"John"}}"#, None)
        );
    }
}

/// GraphQL matcher configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum GraphQLMatchConfig {
    /// Simple string: operation name or type shorthand
    /// - "GetUser" → matches operation name
    /// - "query" → matches any query
    /// - "mutation" → matches any mutation
    /// - "*" → matches any GraphQL operation
    Simple(String),

    /// Boolean value for introspection matching
    /// - true → matches any introspection query
    Boolean(bool),

    /// Structured object with operation details
    Structured {
        /// Operation name
        #[serde(skip_serializing_if = "Option::is_none")]
        operation: Option<String>,

        /// Specific query operation name
        #[serde(skip_serializing_if = "Option::is_none")]
        query: Option<String>,

        /// Specific mutation operation name
        #[serde(skip_serializing_if = "Option::is_none")]
        mutation: Option<String>,

        /// Specific subscription operation name
        #[serde(skip_serializing_if = "Option::is_none")]
        subscription: Option<String>,

        /// Introspection matcher
        /// - true or "true" → match any introspection
        /// - "schema" → match __schema queries
        /// - "type" → match __type queries
        /// - "typename" → match __typename queries
        /// - "*" → match any introspection (same as true)
        #[serde(skip_serializing_if = "Option::is_none")]
        introspection: Option<IntrospectionMatchConfig>,

        /// Variable matchers (flat map for nested paths)
        /// Example: { "id": "123", "input.role": "admin" }
        #[serde(default, skip_serializing_if = "FxHashMap::is_empty")]
        #[allow(clippy::disallowed_types)]
        #[cfg_attr(
            feature = "schema",
            schemars(with = "std::collections::HashMap<String, serde_json::Value>")
        )]
        variables: FxHashMap<String, serde_json::Value>,
    },
}

/// Introspection match configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum IntrospectionMatchConfig {
    /// Boolean: true = match any introspection
    Bool(bool),
    /// String: "schema", "type", "typename", "*"
    String(String),
}

impl IntrospectionMatchConfig {
    pub fn into_introspection_matcher(self) -> Option<crate::types::IntrospectionMatcher> {
        match self {
            IntrospectionMatchConfig::Bool(true) => Some(crate::types::IntrospectionMatcher::Any),
            IntrospectionMatchConfig::Bool(false) => None,
            IntrospectionMatchConfig::String(s) => match s.as_str() {
                "true" | "*" => Some(crate::types::IntrospectionMatcher::Any),
                "schema" => Some(crate::types::IntrospectionMatcher::Schema),
                "type" => Some(crate::types::IntrospectionMatcher::Type),
                "typename" => Some(crate::types::IntrospectionMatcher::TypeName),
                _ => None,
            },
        }
    }
}

impl GraphQLMatchConfig {
    pub fn into_graphql_matcher(self) -> crate::Result<crate::types::GraphQLMatcher> {
        use crate::types::{GraphQLMatcher, GraphQLOperationType};

        match self {
            // Boolean syntax (for introspection)
            GraphQLMatchConfig::Boolean(true) => {
                // match.graphql = true → match any introspection
                Ok(GraphQLMatcher {
                    operation_name: None,
                    operation_type: None,
                    match_any: false,
                    variable_matchers: FxHashMap::default(),
                    introspection_matcher: Some(crate::types::IntrospectionMatcher::Any),
                })
            }
            GraphQLMatchConfig::Boolean(false) => {
                Err(crate::mp_err!("match.graphql = false is invalid"))
            }

            // Simple string syntax
            GraphQLMatchConfig::Simple(s) => match s.as_str() {
                "*" => {
                    // Match any GraphQL operation
                    Ok(GraphQLMatcher {
                        operation_name: None,
                        operation_type: None,
                        match_any: true,
                        variable_matchers: FxHashMap::default(),
                        introspection_matcher: None,
                    })
                }
                "query" => {
                    // Match any query
                    Ok(GraphQLMatcher {
                        operation_name: None,
                        operation_type: Some(GraphQLOperationType::Query),
                        match_any: false,
                        variable_matchers: FxHashMap::default(),
                        introspection_matcher: None,
                    })
                }
                "mutation" => {
                    // Match any mutation
                    Ok(GraphQLMatcher {
                        operation_name: None,
                        operation_type: Some(GraphQLOperationType::Mutation),
                        match_any: false,
                        variable_matchers: FxHashMap::default(),
                        introspection_matcher: None,
                    })
                }
                "subscription" => {
                    // Match any subscription
                    Ok(GraphQLMatcher {
                        operation_name: None,
                        operation_type: Some(GraphQLOperationType::Subscription),
                        match_any: false,
                        variable_matchers: FxHashMap::default(),
                        introspection_matcher: None,
                    })
                }
                operation_name => {
                    // Match specific operation name (any type)
                    Ok(GraphQLMatcher {
                        operation_name: Some(operation_name.to_string()),
                        operation_type: None,
                        match_any: false,
                        variable_matchers: FxHashMap::default(),
                        introspection_matcher: None,
                    })
                }
            },

            // Structured syntax
            GraphQLMatchConfig::Structured {
                operation,
                query,
                mutation,
                subscription,
                introspection,
                variables,
            } => {
                // Parse introspection matcher if present
                let introspection_matcher =
                    introspection.and_then(IntrospectionMatchConfig::into_introspection_matcher);

                // Priority: specific type fields > operation field
                // Handle wildcard "*" specially - it means match any operation of that type
                let (operation_name, operation_type) = if let Some(query_name) = query {
                    if query_name == "*" {
                        (None, Some(GraphQLOperationType::Query))
                    } else {
                        (Some(query_name), Some(GraphQLOperationType::Query))
                    }
                } else if let Some(mutation_name) = mutation {
                    if mutation_name == "*" {
                        (None, Some(GraphQLOperationType::Mutation))
                    } else {
                        (Some(mutation_name), Some(GraphQLOperationType::Mutation))
                    }
                } else if let Some(subscription_name) = subscription {
                    if subscription_name == "*" {
                        (None, Some(GraphQLOperationType::Subscription))
                    } else {
                        (
                            Some(subscription_name),
                            Some(GraphQLOperationType::Subscription),
                        )
                    }
                } else if let Some(operation_name) = operation {
                    (Some(operation_name), None)
                } else {
                    (None, None)
                };

                // Require at least operation name/type, variables, or introspection
                if operation_name.is_none()
                    && operation_type.is_none()
                    && variables.is_empty()
                    && introspection_matcher.is_none()
                {
                    return Err(
            "GraphQL matcher requires at least operation name, type, variables, or introspection".to_string().into(),
          );
                }

                Ok(GraphQLMatcher {
                    operation_name,
                    operation_type,
                    match_any: false,
                    variable_matchers: variables,
                    introspection_matcher,
                })
            }
        }
    }
}
