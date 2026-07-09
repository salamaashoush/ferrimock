//! End-to-end GraphQL integration tests
//!
//! Tests realistic GraphQL request/response scenarios similar to Apollo Client

use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use mockpit::config::GraphQLMatchConfig;
use mockpit::engine::{
    BodySource, MockDefinition, MockMatcher, MockRegistry, RequestMatcher, ResponseGenerator,
    UrlPattern,
};
use rustc_hash::FxHashMap;
use serde_json::json;
use smallvec::smallvec;

/// Helper to create a GraphQL request body
fn graphql_request(query: &str, variables: Option<serde_json::Value>) -> Vec<u8> {
    graphql_request_with_operation_name(query, variables, None)
}

/// Helper to create a GraphQL request body with explicit operation name
fn graphql_request_with_operation_name(
    query: &str,
    variables: Option<serde_json::Value>,
    operation_name: Option<&str>,
) -> Vec<u8> {
    let mut body = json!({
      "query": query
    });

    if let Some(vars) = variables {
        body["variables"] = vars;
    }

    if let Some(op_name) = operation_name {
        body["operationName"] = json!(op_name);
    }

    serde_json::to_vec(&body).unwrap()
}

#[test]
fn test_e2e_basic_query_matching() {
    let registry = MockRegistry::new();

    // Mock for GetUser query
    let mock = MockDefinition {
        id: "get-user-query".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Simple("GetUser".to_string())
                    .into_graphql_matcher()
                    .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "user": {
              "id": "12345",
              "name": "John Doe",
              "email": "john@example.com"
            }
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Simulate GraphQL request
    let query = r#"
    query GetUser {
      user(id: "12345") {
        id
        name
        email
      }
    }
  "#;

    let body = graphql_request_with_operation_name(query, None, Some("GetUser"));
    let headers = HeaderMap::new();

    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));

    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "get-user-query");

    // Verify response contains expected data
    if let BodySource::Inline(body_bytes) = &matched.mock.response.body {
        let body_str = std::str::from_utf8(body_bytes).unwrap();
        assert!(body_str.contains("John Doe"));
    } else {
        panic!("Expected inline body source");
    }
}

#[test]
fn test_e2e_mutation_with_variables() {
    let registry = MockRegistry::new();

    // Mock for CreateUser mutation with role variable matching
    let mock = MockDefinition {
        id: "create-admin-user".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Structured {
                    operation: Some("CreateUser".to_string()),
                    query: None,
                    mutation: Some("CreateUser".to_string()),
                    subscription: None,
                    introspection: None,
                    variables: vec![("input.role".to_string(), json!("admin"))]
                        .into_iter()
                        .collect(),
                }
                .into_graphql_matcher()
                .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "createUser": {
              "id": "new-user-123",
              "name": "Admin User",
              "role": "admin"
            }
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Simulate mutation with admin role
    let query = r"
    mutation CreateUser($input: CreateUserInput!) {
      createUser(input: $input) {
        id
        name
        role
      }
    }
  ";

    let variables = json!({
      "input": {
        "name": "Admin User",
        "email": "admin@example.com",
        "role": "admin"
      }
    });

    let body = graphql_request_with_operation_name(query, Some(variables), Some("CreateUser"));
    let headers = HeaderMap::new();

    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));

    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "create-admin-user");
}

#[test]
fn test_e2e_introspection_query() {
    let registry = MockRegistry::new();

    // Mock for introspection queries
    let mock = MockDefinition {
        id: "introspection-handler".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Boolean(true)
                    .into_graphql_matcher()
                    .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "__schema": {
              "queryType": { "name": "Query" },
              "mutationType": { "name": "Mutation" },
              "subscriptionType": null,
              "types": []
            }
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Simulate introspection query
    let query = r"
    query IntrospectionQuery {
      __schema {
        queryType { name }
        mutationType { name }
        subscriptionType { name }
      }
    }
  ";

    let body = graphql_request(query, None);
    let headers = HeaderMap::new();

    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));

    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "introspection-handler");
}

#[test]
fn test_e2e_priority_based_error_handling() {
    let registry = MockRegistry::new();

    // High priority: Specific error mock for invalid input
    let error_mock = MockDefinition {
        id: "create-user-validation-error".into(),
        priority: 200, // Higher priority
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Structured {
                    operation: Some("CreateUser".to_string()),
                    query: None,
                    mutation: Some("CreateUser".to_string()),
                    subscription: None,
                    introspection: None,
                    variables: vec![("input.email".to_string(), json!("invalid-email"))]
                        .into_iter()
                        .collect(),
                }
                .into_graphql_matcher()
                .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "errors": [{
            "message": "Invalid email format",
            "extensions": {
              "code": "VALIDATION_ERROR"
            }
          }]
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    // Low priority: Success mock for valid input
    let success_mock = MockDefinition {
        id: "create-user-success".into(),
        priority: 100, // Lower priority
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Simple("CreateUser".to_string())
                    .into_graphql_matcher()
                    .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "createUser": {
              "id": "new-user-456",
              "email": "valid@example.com"
            }
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(error_mock);
    registry.add_mock(success_mock);
    let matcher = MockMatcher::new(registry);

    // Test 1: Invalid email should match error mock
    let query = r"
    mutation CreateUser($input: CreateUserInput!) {
      createUser(input: $input) {
        id
        email
      }
    }
  ";

    let invalid_variables = json!({
      "input": {
        "email": "invalid-email"
      }
    });

    let body =
        graphql_request_with_operation_name(query, Some(invalid_variables), Some("CreateUser"));
    let headers = HeaderMap::new();

    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));
    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "create-user-validation-error");

    // Verify error response
    if let BodySource::Inline(body_bytes) = &matched.mock.response.body {
        let body_str = std::str::from_utf8(body_bytes).unwrap();
        assert!(body_str.contains("VALIDATION_ERROR"));
    } else {
        panic!("Expected inline body source");
    }

    // Test 2: Valid email should match success mock
    let valid_variables = json!({
      "input": {
        "email": "valid@example.com"
      }
    });

    let body =
        graphql_request_with_operation_name(query, Some(valid_variables), Some("CreateUser"));
    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));
    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "create-user-success");
}

#[test]
fn test_e2e_subscription_matching() {
    let registry = MockRegistry::new();

    // Mock for GraphQL subscription
    let mock = MockDefinition {
        id: "message-subscription".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Structured {
                    operation: Some("OnMessageReceived".to_string()),
                    query: None,
                    mutation: None,
                    subscription: Some("OnMessageReceived".to_string()),
                    introspection: None,
                    variables: FxHashMap::default(),
                }
                .into_graphql_matcher()
                .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "messageReceived": {
              "id": "msg-789",
              "text": "Hello from subscription!",
              "timestamp": "2025-01-15T10:30:00Z"
            }
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Simulate subscription query
    let query = r"
    subscription OnMessageReceived {
      messageReceived {
        id
        text
        timestamp
      }
    }
  ";

    let body = graphql_request_with_operation_name(query, None, Some("OnMessageReceived"));
    let headers = HeaderMap::new();

    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));

    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "message-subscription");
}

#[test]
fn test_e2e_apollo_client_headers() {
    let registry = MockRegistry::new();

    // Mock that checks for Apollo Client headers
    let mock = MockDefinition {
        id: "apollo-query".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![mockpit::engine::HeaderMatcher::present(
                HeaderName::from_static("content-type",)
            )],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Simple("GetData".to_string())
                    .into_graphql_matcher()
                    .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"data": {"items": []}}"#),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(mock);
    let matcher = MockMatcher::new(registry);

    // Simulate Apollo Client request with typical headers
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );

    let query = r"
    query GetData {
      items {
        id
        name
      }
    }
  ";

    let body = graphql_request_with_operation_name(query, None, Some("GetData"));
    let result = matcher.find_match(&Method::POST, "/graphql", None, &headers, Some(&body));

    assert!(result.is_some());
    let matched = result.unwrap();
    assert_eq!(matched.mock.id, "apollo-query");
}

#[test]
fn test_e2e_variable_based_routing() {
    let registry = MockRegistry::new();

    // Mock 1: Route to fast cache for status=active
    let cache_mock = MockDefinition {
        id: "cached-users".into(),
        priority: 200,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Structured {
                    operation: Some("ListUsers".to_string()),
                    query: Some("ListUsers".to_string()),
                    mutation: None,
                    subscription: None,
                    introspection: None,
                    variables: vec![("filter.status".to_string(), json!("active"))]
                        .into_iter()
                        .collect(),
                }
                .into_graphql_matcher()
                .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "users": [
              {"id": "1", "name": "Alice", "status": "active"},
              {"id": "2", "name": "Bob", "status": "active"}
            ]
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    // Mock 2: Route to slow query for status=archived
    let slow_mock = MockDefinition {
        id: "archived-users".into(),
        priority: 200,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Structured {
                    operation: Some("ListUsers".to_string()),
                    query: Some("ListUsers".to_string()),
                    mutation: None,
                    subscription: None,
                    introspection: None,
                    variables: vec![("filter.status".to_string(), json!("archived"))]
                        .into_iter()
                        .collect(),
                }
                .into_graphql_matcher()
                .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(
                r#"{
          "data": {
            "users": [
              {"id": "999", "name": "Old User", "status": "archived"}
            ]
          }
        }"#,
            ),
        ),
        vars: None,
        streaming: None,
    };

    // Mock 3: Default fallback
    let default_mock = MockDefinition {
        id: "default-users".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/graphql")],
            header_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: Some(
                GraphQLMatchConfig::Simple("ListUsers".to_string())
                    .into_graphql_matcher()
                    .unwrap(),
            ),
            query_matchers: smallvec![],
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"data": {"users": []}}"#),
        ),
        vars: None,
        streaming: None,
    };

    registry.add_mock(cache_mock);
    registry.add_mock(slow_mock);
    registry.add_mock(default_mock);
    let matcher = MockMatcher::new(registry);

    let query = r"
    query ListUsers($filter: UserFilter) {
      users(filter: $filter) {
        id
        name
        status
      }
    }
  ";

    // Test 1: Active users should hit cache
    let active_vars = json!({
      "filter": {
        "status": "active"
      }
    });
    let body = graphql_request_with_operation_name(query, Some(active_vars), Some("ListUsers"));
    let result = matcher.find_match(
        &Method::POST,
        "/graphql",
        None,
        &HeaderMap::new(),
        Some(&body),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().mock.id, "cached-users");

    // Test 2: Archived users should hit slow query
    let archived_vars = json!({
      "filter": {
        "status": "archived"
      }
    });
    let body = graphql_request_with_operation_name(query, Some(archived_vars), Some("ListUsers"));
    let result = matcher.find_match(
        &Method::POST,
        "/graphql",
        None,
        &HeaderMap::new(),
        Some(&body),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().mock.id, "archived-users");

    // Test 3: Other status should hit default
    let other_vars = json!({
      "filter": {
        "status": "pending"
      }
    });
    let body = graphql_request_with_operation_name(query, Some(other_vars), Some("ListUsers"));
    let result = matcher.find_match(
        &Method::POST,
        "/graphql",
        None,
        &HeaderMap::new(),
        Some(&body),
    );
    assert!(result.is_some());
    assert_eq!(result.unwrap().mock.id, "default-users");
}
