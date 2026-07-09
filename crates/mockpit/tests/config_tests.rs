#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Edge case tests for config.rs to improve coverage from 68.71% to 90%+

use mockpit::config::{
    HeaderMatchConfig, MatchConfig, MockCollectionConfig, MockConfig, RequestConfig, ReturnConfig,
};
use rustc_hash::FxHashMap;
use std::time::Duration;
use tempfile::TempDir;

// ============================================================================
// MockCollectionConfig Parsing Tests
// ============================================================================

#[test]
fn test_mock_collection_defaults() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.name, None);
    assert_eq!(config.description, None);
    assert!(config.enabled); // default_enabled
    assert_eq!(config.mocks.len(), 1);
}

#[test]
fn test_mock_collection_disabled() {
    let yaml = r#"
enabled: false
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(!config.enabled);
}

#[test]
fn test_empty_mocks_collection() {
    let yaml = r#"
name: "Empty Collection"
description: "No mocks"
enabled: true
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.mocks.len(), 0);
    assert_eq!(config.name, Some("Empty Collection".to_string()));
}

#[tokio::test]
async fn test_from_file_json() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.json");

    let content = r#"{
  "mocks": [
    {
      "id": "test",
      "match": {"url": "/test"},
      "response": {"status": 200}
    }
  ]
}"#;

    std::fs::write(&file_path, content).unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_ok());
    let config = result.unwrap();
    assert_eq!(config.mocks.len(), 1);
}

#[tokio::test]
async fn test_from_file_yaml() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.yaml");

    let content = r"
mocks:
  - id: test
    match:
      url: /test
    response:
      status: 200
";

    std::fs::write(&file_path, content).unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_ok());
    let config = result.unwrap();
    assert_eq!(config.mocks.len(), 1);
}

#[tokio::test]
async fn test_from_file_yml_extension() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.yml");

    let content = r"
mocks:
  - id: test
    match:
      url: /test
    response:
      status: 200
";

    std::fs::write(&file_path, content).unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_from_file_no_extension() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test");

    std::fs::write(&file_path, "content").unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no extension"));
}

#[tokio::test]
async fn test_from_file_unsupported_extension() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.xml");

    std::fs::write(&file_path, "<root/>").unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Unsupported file format")
    );
}

#[tokio::test]
async fn test_from_file_nonexistent() {
    let result = MockCollectionConfig::from_file("/nonexistent/path/file.toml").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_from_har_string_basic() {
    let har_content = r#"{
  "log": {
    "version": "1.2",
    "creator": {"name": "test", "version": "1.0"},
    "entries": [
      {
        "startedDateTime": "2024-01-01T00:00:00.000Z",
        "time": 100,
        "request": {
          "method": "GET",
          "url": "https://example.com/api/test",
          "httpVersion": "HTTP/1.1",
          "headers": [],
          "queryString": [],
          "cookies": [],
          "headersSize": -1,
          "bodySize": -1
        },
        "response": {
          "status": 200,
          "statusText": "OK",
          "httpVersion": "HTTP/1.1",
          "headers": [],
          "cookies": [],
          "content": {
            "size": 13,
            "mimeType": "text/plain",
            "text": "test response"
          },
          "redirectURL": "",
          "headersSize": -1,
          "bodySize": 13
        },
        "cache": {},
        "timings": {"send": 0, "wait": 100, "receive": 0}
      }
    ]
  }
}"#;

    let result = MockCollectionConfig::from_har(har_content).await;
    assert!(result.is_ok());
    let config = result.unwrap();
    assert!(config.name.is_some());
    assert!(config.name.unwrap().contains("HAR"));
    assert!(config.enabled);
}

#[tokio::test]
async fn test_from_file_har_extension() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.har");

    let content = r#"{
  "log": {
    "version": "1.2",
    "creator": {"name": "test", "version": "1.0"},
    "entries": []
  }
}"#;

    std::fs::write(&file_path, content).unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_from_file_json_har_detection() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.json");

    let content = r#"{"log": {"version": "1.2", "creator": {"name": "test", "version": "1.0"}, "entries": []}}"#;

    std::fs::write(&file_path, content).unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_ok());
    let config = result.unwrap();
    // Should be detected as HAR
    assert!(config.name.is_some());
}

#[tokio::test]
async fn test_from_file_json_with_log_in_content() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.json");

    // JSON that contains "log" but not as top-level key
    let content =
        r#"{"mocks": [{"id": "test", "match": {"url": "/log"}, "return": {"status": 200}}]}"#;

    std::fs::write(&file_path, content).unwrap();

    let result = MockCollectionConfig::from_file(&file_path).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_into_mock_definitions_empty() {
    let config = MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![],
    };

    let result = config.into_mock_definitions().await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 0);
}

#[tokio::test]
async fn test_into_mock_definitions_with_dir() {
    let temp_dir = TempDir::new().unwrap();
    let config_dir = temp_dir.path();

    let config = MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![MockConfig {
            id: "test".into(),
            description: None,
            priority: 100,
            enabled: true,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                method: Some("GET".to_string()),
                methods: vec![],
                url: Some("/test".to_string()),
                urls: vec![],
                headers: FxHashMap::default(),
                query: FxHashMap::default(),
                graphql: None,
                body: FxHashMap::default(),
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: FxHashMap::default(),
                body: Some("test".to_string()),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            patch: None,
            delay: None,
            sse: None,
            ws: None,
        }],
    };

    let result = config
        .into_mock_definitions_with_dir(Some(config_dir), None)
        .await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 1);
}

// ============================================================================
// MockConfig Tests
// ============================================================================

#[tokio::test]
async fn test_mock_config_missing_match() {
    let config = MockConfig {
        id: "test".into(),
        description: None,
        priority: 100,
        enabled: true,
        scope: None,
        vars: None,
        match_config: None,
        request: None,
        response_config: Some(ReturnConfig::Structured {
            status: Some(200),
            headers: FxHashMap::default(),
            body: Some("test".to_string()),
            template: None,
            file: None,
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        }),
        patch: None,
        delay: None,
        sse: None,
        ws: None,
    };

    let result = config.into_mock_definition().await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Missing 'match' configuration")
    );
}

#[tokio::test]
async fn test_mock_config_missing_return() {
    let config = MockConfig {
        id: "test".into(),
        description: None,
        priority: 100,
        enabled: true,
        scope: None,
        vars: None,
        match_config: Some(MatchConfig {
            method: Some("GET".to_string()),
            methods: vec![],
            url: Some("/test".to_string()),
            urls: vec![],
            headers: FxHashMap::default(),
            query: FxHashMap::default(),
            graphql: None,
            body: FxHashMap::default(),
        }),
        request: None,
        response_config: None,
        patch: None,
        delay: None,
        sse: None,
        ws: None,
    };

    // No response_config means it defaults to empty response (no longer an error)
    let result = config.into_mock_definition().await;
    assert!(result.is_ok());
}

#[test]
fn test_mock_config_with_scope() {
    let yaml = r#"
id: "test"
scope: "test-suite"
priority: 200
enabled: false
match:
  url: "/test"
response:
  status: 200
"#;

    let config: MockConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.scope, Some("test-suite".into()));
    assert_eq!(config.priority, 200);
    assert!(!config.enabled);
}

// ============================================================================
// MatchConfig Tests - Ultra-flat syntax
// ============================================================================

#[test]
fn test_match_string_valid() {
    let yaml = r#"
mocks:
  - id: "test"
    match: "POST /api/users"
    response:
      status: 201
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.method, Some("POST".to_string()));
    assert_eq!(match_config.url, Some("/api/users".to_string()));
}

#[test]
fn test_match_string_with_multiple_spaces() {
    let yaml = r#"
mocks:
  - id: "test"
    match: "GET    /path/with/spaces"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.method, Some("GET".to_string()));
    assert_eq!(match_config.url, Some("/path/with/spaces".to_string()));
}

#[test]
fn test_match_string_invalid_no_space() {
    let yaml = r#"
mocks:
  - id: "test"
    match: "GET"
    response:
      status: 200
"#;

    let result = serde_yaml::from_str::<MockCollectionConfig>(yaml);
    assert!(result.is_err());
}

#[test]
fn test_match_string_invalid_method() {
    let yaml = r#"
mocks:
  - id: "test"
    match: "INVALID /api/test"
    response:
      status: 200
"#;

    let result = serde_yaml::from_str::<MockCollectionConfig>(yaml);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid HTTP method")
    );
}

#[test]
fn test_match_method_shortcut_get() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      GET: "/api/health"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert!(match_config.methods.contains(&"GET".to_string()));
    assert!(match_config.urls.contains(&"/api/health".to_string()));
}

#[test]
fn test_match_method_shortcut_multiple() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      GET: "/api/users"
      POST: "/api/users"
      DELETE: "/api/users/:id"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.methods.len(), 3);
    assert!(match_config.methods.contains(&"GET".to_string()));
    assert!(match_config.methods.contains(&"POST".to_string()));
    assert!(match_config.methods.contains(&"DELETE".to_string()));
    assert_eq!(match_config.urls.len(), 3);
}

#[test]
fn test_match_method_shortcut_all_http_methods() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      GET: "/1"
      POST: "/2"
      PUT: "/3"
      DELETE: "/4"
      PATCH: "/5"
      HEAD: "/6"
      OPTIONS: "/7"
      TRACE: "/8"
      CONNECT: "/9"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.methods.len(), 9);
}

#[test]
fn test_match_method_shortcut_case_insensitive() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      get: "/api/test"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert!(match_config.methods.contains(&"get".to_string()));
}

#[test]
fn test_match_singular_and_plural_methods() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      method: "GET"
      methods: ["POST", "PUT"]
      url: "/test"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.method, Some("GET".to_string()));
    assert_eq!(match_config.methods.len(), 2);
}

#[test]
fn test_match_singular_and_plural_urls() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test1"
      urls: ["/test2", "/test3"]
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.url, Some("/test1".to_string()));
    assert_eq!(match_config.urls.len(), 2);
}

#[test]
fn test_match_headers_inline() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      headers:
        authorization: "Bearer token"
        content-type: "application/json"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.headers.len(), 2);
}

#[test]
fn test_match_query_inline() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      query:
        page: "1"
        limit: "10"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.query.len(), 2);
    assert_eq!(match_config.query.get("page"), Some(&"1".to_string()));
}

#[test]
fn test_match_body_inline() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        "$.user.name": "Alice"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let match_config = config.mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.body.len(), 1);
}

#[test]
fn test_match_config_serialization() {
    let match_config = MatchConfig {
        method: Some("GET".to_string()),
        url: Some("/test".to_string()),
        ..Default::default()
    };

    let serialized = serde_yaml::to_string(&match_config).unwrap();
    assert!(serialized.contains("method"));
    assert!(serialized.contains("url"));
}

// ============================================================================
// ReturnConfig Tests - All Variants
// ============================================================================

#[test]
fn test_return_template_variant() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    return: "{{ request.body }}"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Template(s) => assert!(s.contains("request.body")),
        _ => panic!("Expected Template variant"),
    }
}

#[test]
fn test_return_status_shortcuts_single() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      "200": "OK"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::StatusShortcuts(shortcuts) => {
            assert_eq!(shortcuts.len(), 1);
            assert_eq!(shortcuts.get(&200), Some(&"OK".to_string()));
        }
        _ => panic!("Expected StatusShortcuts variant"),
    }
}

#[test]
fn test_return_status_shortcuts_multiple() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      "200": "OK"
      "201": "Created"
      "404": "Not Found"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::StatusShortcuts(shortcuts) => {
            assert_eq!(shortcuts.len(), 3);
            assert!(shortcuts.contains_key(&200));
            assert!(shortcuts.contains_key(&201));
            assert!(shortcuts.contains_key(&404));
        }
        _ => panic!("Expected StatusShortcuts variant"),
    }
}

#[test]
fn test_return_status_shortcuts_boundary_codes() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      "100": "Continue"
      "599": "Network error"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::StatusShortcuts(shortcuts) => {
            assert!(shortcuts.contains_key(&100));
            assert!(shortcuts.contains_key(&599));
        }
        _ => panic!("Expected StatusShortcuts variant"),
    }
}

#[test]
fn test_return_status_shortcuts_mixed_with_structured_error() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      "200": "OK"
      status: 201
"#;

    let result = serde_yaml::from_str::<MockCollectionConfig>(yaml);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Cannot mix status shortcuts")
    );
}

#[test]
fn test_return_structured_basic() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 200
      body: "test"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { status, body, .. } => {
            assert_eq!(*status, Some(200));
            assert_eq!(body, &Some("test".to_string()));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_structured_with_json() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 200
      json:
        name: "test"
        count: 42
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { json, .. } => {
            assert!(json.is_object());
            let obj = json.as_object().unwrap();
            assert_eq!(obj.get("name").unwrap().as_str(), Some("test"));
            assert_eq!(obj.get("count").unwrap().as_i64(), Some(42));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_structured_with_headers() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 200
      headers:
        content-type: "application/json"
        x-custom: "value"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 2);
            assert_eq!(
                headers.get("content-type"),
                Some(&"application/json".to_string())
            );
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_mock_config_with_delay_field() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "100ms"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.mocks[0].delay.as_deref(), Some("100ms"));
}

#[test]
fn test_return_default() {
    let config = ReturnConfig::default();
    match config {
        ReturnConfig::Structured {
            status,
            headers,
            body,
            template,
            file,
            template_file,
            json,
        } => {
            assert_eq!(status, None);
            assert!(headers.is_empty());
            assert_eq!(body, None);
            assert_eq!(template, None);
            assert_eq!(file, None);
            assert_eq!(template_file, None);
            assert!(json.is_null());
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_response_config_body_method() {
    let config1 = ReturnConfig::Template("template body".to_string());
    assert_eq!(config1.body(), Some(&"template body".to_string()));

    let mut shortcuts = FxHashMap::default();
    shortcuts.insert(200, "ok body".to_string());
    let config2 = ReturnConfig::StatusShortcuts(shortcuts);
    assert_eq!(config2.body(), Some(&"ok body".to_string()));

    let config3 = ReturnConfig::Structured {
        status: Some(200),
        headers: FxHashMap::default(),
        body: Some("structured body".to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    };
    assert_eq!(config3.body(), Some(&"structured body".to_string()));
}

#[test]
fn test_response_config_status_method() {
    let config1 = ReturnConfig::Template("template".to_string());
    assert_eq!(config1.status(), None);

    let mut shortcuts = FxHashMap::default();
    shortcuts.insert(201, "created".to_string());
    shortcuts.insert(404, "not found".to_string());
    let config2 = ReturnConfig::StatusShortcuts(shortcuts);
    // Should return first status code
    let status = config2.status();
    assert!(status == Some(201) || status == Some(404));

    let config3 = ReturnConfig::Structured {
        status: Some(204),
        headers: FxHashMap::default(),
        body: None,
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    };
    assert_eq!(config3.status(), Some(204));
}

#[test]
fn test_return_serialization() {
    let config = ReturnConfig::Structured {
        status: Some(200),
        headers: FxHashMap::default(),
        body: Some("test".to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    };

    let serialized = serde_yaml::to_string(&config).unwrap();
    assert!(serialized.contains("status"));
    assert!(serialized.contains("body"));
}

// ============================================================================
// Explicit Body Type Field Tests
// ============================================================================

#[test]
fn test_return_file_field() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      file: "response.json"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { file, .. } => {
            assert_eq!(file, &Some("response.json".to_string()));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_template_file_field() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      template_file: "response.tpl"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { template_file, .. } => {
            assert_eq!(template_file, &Some("response.tpl".to_string()));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_template_field() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      template: "{{ request.body }}"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { template, .. } => {
            assert!(template.as_ref().unwrap().contains("{{"));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_template_tera_syntax() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      template: "{% if user %}hello{% endif %}"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { template, .. } => {
            assert!(template.as_ref().unwrap().contains("{%"));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_json_inline_template() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      json: "{{ dynamic_json }}"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { json, .. } => {
            assert!(json.as_str().unwrap().contains("{{"));
        }
        _ => panic!("Expected Structured variant"),
    }
}

#[test]
fn test_return_json_object() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      json:
        status: "success"
        data:
          count: 42
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let response_config = config.mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { json, .. } => {
            assert!(json.is_object());
        }
        _ => panic!("Expected Structured variant"),
    }
}

// ============================================================================
// BodyMatcherConfig Tests - Via RequestConfig
// ============================================================================

#[tokio::test]
async fn test_body_matcher_legacy_contains_string() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        contains: "text"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = mock_def[0].request.body_matcher.as_ref().unwrap();
    assert!(matcher.matches(b"this text is here", None));
    assert!(!matcher.matches(b"no match", None));
}

#[tokio::test]
async fn test_body_matcher_legacy_contains_array() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        contains: ["text1", "text2"]
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = mock_def[0].request.body_matcher.as_ref().unwrap();
    // Should match first element
    assert!(matcher.matches(b"text1 is here", None));
}

#[tokio::test]
async fn test_body_matcher_legacy_regex() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        regex: "\\d{3}"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = mock_def[0].request.body_matcher.as_ref().unwrap();
    assert!(matcher.matches(b"number 123 here", None));
    assert!(!matcher.matches(b"no numbers", None));
}

#[tokio::test]
async fn test_body_matcher_prefix_regex() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        "~\\d{3}": true
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = mock_def[0].request.body_matcher.as_ref().unwrap();
    assert!(matcher.matches(b"number 456", None));
}

#[tokio::test]
async fn test_body_matcher_prefix_contains() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        "@important": true
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = mock_def[0].request.body_matcher.as_ref().unwrap();
    assert!(matcher.matches(b"this is important", None));
}

#[tokio::test]
async fn test_body_matcher_prefix_json_path() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      body:
        "$.user.id": 42
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = mock_def[0].request.body_matcher.as_ref().unwrap();
    assert!(matcher.matches(br#"{"user": {"id": 42}}"#, None));
}

// ============================================================================
// HeaderMatchConfig Tests - Via RequestConfig
// ============================================================================

#[tokio::test]
async fn test_header_match_exact() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      headers:
        content-type: "application/json"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = &mock_def[0].request.header_matchers[0];

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::HeaderName::from_static("content-type"),
        http::header::HeaderValue::from_static("application/json"),
    );
    assert!(matcher.matches(&headers));
}

#[tokio::test]
async fn test_header_match_regex() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      headers:
        authorization: "~Bearer.*"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = &mock_def[0].request.header_matchers[0];

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::HeaderName::from_static("authorization"),
        http::header::HeaderValue::from_static("Bearer token123"),
    );
    assert!(matcher.matches(&headers));
}

#[tokio::test]
async fn test_header_match_present() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      headers:
        x-custom: "?"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = &mock_def[0].request.header_matchers[0];

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::HeaderName::from_static("x-custom"),
        http::header::HeaderValue::from_static("any-value"),
    );
    assert!(matcher.matches(&headers));
}

#[tokio::test]
async fn test_header_match_absent() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
      headers:
        x-cache: "!"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    let matcher = &mock_def[0].request.header_matchers[0];

    let headers = http::HeaderMap::new();
    assert!(matcher.matches(&headers));
}

// ============================================================================
// URL Pattern Parsing Tests
// ============================================================================

#[test]
fn test_url_pattern_express_optional_param() {
    let yaml = r#"
url_patterns: ["/api/users/:id?"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_express_multiple_params() {
    let yaml = r#"
url_patterns: ["/users/:user_id/posts/:post_id/comments/:comment_id"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_express_with_wildcard() {
    let yaml = r#"
url_patterns: ["/api/*/users/:id"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_glob_double_star() {
    let yaml = r#"
url_patterns: ["/api/**/files"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_glob_single_star() {
    let yaml = r#"
url_patterns: ["/api/*/data"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_digit() {
    let yaml = r#"
url_patterns: ["^/api/v\\d+/users$"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_word() {
    let yaml = r#"
url_patterns: ["/api/\\w+"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_whitespace() {
    let yaml = r#"
url_patterns: ["/api/\\s+"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_bracket() {
    let yaml = r#"
url_patterns: ["/api/[a-z]+"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_paren() {
    let yaml = r#"
url_patterns: ["/api/(users|posts)"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_plus() {
    let yaml = r#"
url_patterns: ["/api/.+"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_regex_star() {
    let yaml = r#"
url_patterns: ["/api/.*"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

#[test]
fn test_url_pattern_exact_simple() {
    let yaml = r#"
url_patterns: ["/api/users"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let matcher = config.into_request_matcher().unwrap();
    assert_eq!(matcher.url_patterns.len(), 1);
}

// ============================================================================
// Duration Parsing Tests
// ============================================================================

#[tokio::test]
async fn test_delay_microseconds() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "500us"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    assert_eq!(
        mock_def[0].response.delay,
        Some(std::time::Duration::from_micros(500))
    );
}

#[tokio::test]
async fn test_delay_milliseconds() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "250ms"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    assert_eq!(
        mock_def[0].response.delay,
        Some(std::time::Duration::from_millis(250))
    );
}

#[tokio::test]
async fn test_delay_seconds() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "2s"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    assert_eq!(
        mock_def[0].response.delay,
        Some(std::time::Duration::from_secs(2))
    );
}

#[tokio::test]
async fn test_delay_zero() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "0ms"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock_def = config.into_mock_definitions().await.unwrap();
    assert_eq!(
        mock_def[0].response.delay,
        Some(std::time::Duration::from_millis(0))
    );
}

#[tokio::test]
async fn test_delay_invalid_format() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "100"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_delay_invalid_unit() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "100m"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_delay_invalid_value() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    delay: "abcms"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(result.is_err());
}

// ============================================================================
// RequestConfig Tests
// ============================================================================

#[test]
fn test_request_config_valid_custom_method() {
    // HTTP spec allows custom methods, so "CUSTOM" should be accepted
    let yaml = r#"
methods: ["CUSTOM"]
url_patterns: ["/test"]
"#;

    let config: RequestConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_request_matcher();
    // Custom methods are allowed, so this should succeed
    assert!(result.is_ok());
    let matcher = result.unwrap();
    assert_eq!(matcher.methods.len(), 1);
}

#[test]
fn test_request_config_invalid_header_name() {
    let mut headers = FxHashMap::default();
    headers.insert(
        "Invalid Header!".to_string(),
        HeaderMatchConfig::Exact("value".to_string()),
    );

    let config = RequestConfig {
        methods: vec![],
        url_patterns: vec![],
        headers,
        query: FxHashMap::default(),
        body_matcher: None,
        graphql_matcher: None,
    };

    let result = config.into_request_matcher();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid header name")
    );
}

#[test]
fn test_request_config_query_matchers() {
    let mut query = FxHashMap::default();
    query.insert("page".to_string(), "1".to_string());
    query.insert("limit".to_string(), "10".to_string());

    let config = RequestConfig {
        methods: vec![],
        url_patterns: vec![],
        headers: FxHashMap::default(),
        query,
        body_matcher: None,
        graphql_matcher: None,
    };

    let result = config.into_request_matcher().unwrap();
    assert_eq!(result.query_matchers.len(), 2);
}

// ============================================================================
// Invalid Status Code Tests
// ============================================================================

#[tokio::test]
async fn test_invalid_status_code_zero() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 0
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid status code")
    );
}

#[tokio::test]
async fn test_invalid_status_code_too_high() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      status: 1000
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid status code")
    );
}

// ============================================================================
// Template Validation Tests
// ============================================================================

#[tokio::test]
async fn test_template_validation_invalid_syntax() {
    // Template validation has been moved out of config parsing
    // Config parsing now succeeds, validation happens separately via MockValidator
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      template: "{{ unclosed"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    // Config parsing succeeds - the template error will be caught by MockValidator
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_template_file_nonexistent() {
    let yaml = r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      template_file: "nonexistent.tpl"
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Failed to read template file")
    );
}

#[tokio::test]
async fn test_template_file_invalid_syntax() {
    // Template validation has been moved out of config parsing
    // Config parsing now succeeds, validation happens separately via MockValidator
    let temp_dir = TempDir::new().unwrap();
    let template_path = temp_dir.path().join("invalid.tpl");
    std::fs::write(&template_path, "{{ unclosed").unwrap();

    let yaml = format!(
        r#"
mocks:
  - id: "test"
    match:
      url: "/test"
    response:
      template_file: "{}"
"#,
        template_path.display().to_string().replace('\\', "/")
    );

    let config: MockCollectionConfig = serde_yaml::from_str(&yaml).unwrap();
    let result = config.into_mock_definitions().await;
    // Config parsing succeeds - the template error will be caught by MockValidator
    assert!(result.is_ok());
}

// ============================================================================
// File Loading Tests
// ============================================================================

#[tokio::test]
async fn test_body_file_with_config_dir() {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("response.txt");
    std::fs::write(&file_path, "test content").unwrap();

    let config = MockConfig {
        id: "test".into(),
        description: None,
        priority: 100,
        enabled: true,
        scope: None,
        vars: None,
        match_config: Some(MatchConfig {
            method: Some("GET".to_string()),
            methods: vec![],
            url: Some("/test".to_string()),
            urls: vec![],
            headers: FxHashMap::default(),
            query: FxHashMap::default(),
            graphql: None,
            body: FxHashMap::default(),
        }),
        request: None,
        response_config: Some(ReturnConfig::Structured {
            status: Some(200),
            headers: FxHashMap::default(),
            body: None,
            template: None,
            file: Some("response.txt".to_string()),
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        }),
        patch: None,
        delay: None,
        sse: None,
        ws: None,
    };

    let result = config
        .into_mock_definition_with_dir(Some(temp_dir.path()))
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_body_file_not_found_fallback() {
    let config = MockConfig {
        id: "test".into(),
        description: None,
        priority: 100,
        enabled: true,
        scope: None,
        vars: None,
        match_config: Some(MatchConfig {
            method: Some("GET".to_string()),
            methods: vec![],
            url: Some("/test".to_string()),
            urls: vec![],
            headers: FxHashMap::default(),
            query: FxHashMap::default(),
            graphql: None,
            body: FxHashMap::default(),
        }),
        request: None,
        response_config: Some(ReturnConfig::Structured {
            status: Some(200),
            headers: FxHashMap::default(),
            body: None,
            template: None,
            file: Some("nonexistent.txt".to_string()),
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        }),
        patch: None,
        delay: None,
        sse: None,
        ws: None,
    };

    // Should succeed but file will be loaded on-demand
    let result = config.into_mock_definition().await;
    assert!(result.is_ok());
}

// ============================================================================
// Request Transform Config Parsing Tests
// ============================================================================

#[test]
fn test_parse_request_transform_full_in_mock() {
    let yaml = r#"
mocks:
  - id: "full-transform"
    match:
      method: "GET"
      url: "/api/users/:id"
    request:
      delay: "500ms"
      timeout: "10s"
      forward_to: "https://staging.example.com"
      rewrite_path: "/v2/users/{{ captures.id }}"
      headers:
        add:
          x-trace-id: "{{ fake_uuid() }}"
          x-forwarded-for: "mock-proxy"
        remove: ["x-real-ip"]
      query:
        add:
          debug: "true"
          source: "mock"
        remove: ["sensitive_key"]
      body:
        jsonpath:
          "$.metadata.proxied": true
          "$.clientId": "test-client"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.mocks.len(), 1);

    let mock = &config.mocks[0];
    let request = mock
        .request
        .as_ref()
        .expect("request transforms should exist");

    assert_eq!(request.delay.as_deref(), Some("500ms"));
    assert_eq!(request.timeout.as_deref(), Some("10s"));
    assert_eq!(
        request.forward_to.as_deref(),
        Some("https://staging.example.com")
    );
    assert_eq!(
        request.rewrite_path.as_deref(),
        Some("/v2/users/{{ captures.id }}")
    );
    assert_eq!(request.headers.add.len(), 2);
    assert_eq!(request.headers.remove.len(), 1);
    assert_eq!(request.query.add.len(), 2);
    assert_eq!(request.query.remove.len(), 1);
    assert_eq!(request.body.jsonpath.len(), 2);
    assert!(!request.is_empty());
}

#[test]
fn test_parse_request_transform_headers_only() {
    let yaml = r#"
mocks:
  - id: "headers-only"
    match:
      method: "GET"
      url: "/api/test"
    request:
      headers:
        add:
          x-injected: "value"
          x-source: "proxy"
        remove: ["x-internal"]
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock = &config.mocks[0];
    let request = mock
        .request
        .as_ref()
        .expect("request transforms should exist");

    assert_eq!(request.headers.add.len(), 2);
    assert_eq!(request.headers.remove.len(), 1);
    assert!(request.delay.is_none());
    assert!(request.timeout.is_none());
    assert!(request.forward_to.is_none());
    assert!(request.rewrite_path.is_none());
    assert!(request.query.add.is_empty());
    assert!(request.query.remove.is_empty());
    assert!(request.body.jsonpath.is_empty());
    assert!(request.body.regex.is_empty());
}

#[test]
fn test_parse_request_transform_body_patches() {
    let yaml = r#"
mocks:
  - id: "body-patches"
    match:
      method: "POST"
      url: "/api/data"
    request:
      body:
        jsonpath:
          "$.metadata.source": "proxy"
          "$.count": 42
        regex:
          - pattern: "old-value"
            replacement: "new-value"
          - pattern: "staging"
            replacement: "production"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock = &config.mocks[0];
    let request = mock
        .request
        .as_ref()
        .expect("request transforms should exist");

    assert_eq!(request.body.jsonpath.len(), 2);
    assert_eq!(request.body.regex.len(), 2);
}

#[test]
fn test_parse_request_transform_upstream_options() {
    let yaml = r#"
mocks:
  - id: "upstream-opts"
    match:
      method: "GET"
      url: "/api/proxy"
    request:
      forward_to: "https://internal.example.com"
      timeout: "30s"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock = &config.mocks[0];
    let request = mock
        .request
        .as_ref()
        .expect("request transforms should exist");

    assert_eq!(
        request.forward_to.as_deref(),
        Some("https://internal.example.com")
    );
    assert_eq!(request.timeout.as_deref(), Some("30s"));
}

#[test]
fn test_parse_request_transform_rewrite_path() {
    let yaml = r#"
mocks:
  - id: "rewrite"
    match:
      method: "GET"
      url: "/api/v1/users/:id"
    request:
      rewrite_path: "/v2/users/{{ captures.id }}"
    response:
      status: 200
"#;

    let config: MockCollectionConfig = serde_yaml::from_str(yaml).unwrap();
    let mock = &config.mocks[0];
    let request = mock
        .request
        .as_ref()
        .expect("request transforms should exist");

    assert_eq!(
        request.rewrite_path.as_deref(),
        Some("/v2/users/{{ captures.id }}")
    );
}

#[tokio::test]
async fn test_heuristic_patch_upstream_when_request_present() {
    let yaml = r#"
mocks:
  - id: "passthrough"
    match:
      method: "GET"
      url: "/api/test"
    request:
      headers:
        add:
          x-injected: "value"
    response:
      status: 200
"#;

    let config = MockCollectionConfig::from_yaml(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(
        result.is_ok(),
        "Should succeed for passthrough mock with request transforms"
    );

    let defs = result.unwrap();
    assert_eq!(defs.len(), 1);
    assert!(
        defs[0].request_transforms.is_some(),
        "request_transforms should be Some for mock with request section"
    );
}

#[tokio::test]
async fn test_error_body_with_request_transforms() {
    let yaml = r#"
mocks:
  - id: "conflict"
    match:
      method: "GET"
      url: "/api/test"
    request:
      headers:
        add:
          x-injected: "value"
    response:
      status: 200
      body: '{"test": true}'
"#;

    let config = MockCollectionConfig::from_yaml(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(
        result.is_err(),
        "Should fail when combining body with request transforms"
    );

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Cannot combine full mock body"),
        "Error should mention conflicting modes, got: {err}"
    );
}

#[tokio::test]
async fn test_response_patches_without_body() {
    let yaml = r#"
mocks:
  - id: "patches-only"
    match:
      method: "GET"
      url: "/api/test"
    patch:
      jsonpath:
        "$.count": 42
"#;

    let config = MockCollectionConfig::from_yaml(yaml).unwrap();
    let result = config.into_mock_definitions().await;
    assert!(
        result.is_ok(),
        "Should succeed for top-level patch without body"
    );
}

#[tokio::test]
async fn test_build_request_transforms_all_types() {
    let yaml = r#"
mocks:
  - id: "all-transforms"
    match:
      method: "GET"
      url: "/api/users/:id"
    request:
      delay: "200ms"
      timeout: "10s"
      forward_to: "https://staging.example.com"
      rewrite_path: "/v2/users"
      headers:
        add:
          x-trace: "abc"
        remove: ["x-old"]
      query:
        add:
          debug: "true"
        remove: ["token"]
      body:
        jsonpath:
          "$.injected": true
    response:
      status: 200
"#;

    let config = MockCollectionConfig::from_yaml(yaml).unwrap();
    let defs = config.into_mock_definitions().await.unwrap();
    assert_eq!(defs.len(), 1);

    let rt = defs[0]
        .request_transforms
        .as_ref()
        .expect("request_transforms should be Some");

    // Header add (1) + header remove (1) + query add (1) + query remove (1) + body jsonpath (1) = 5
    assert_eq!(rt.patches.len(), 5);

    assert_eq!(rt.pre_delay, Some(Duration::from_millis(200)));

    assert_eq!(rt.upstream_options.timeout, Some(Duration::from_secs(10)));
    assert_eq!(
        rt.upstream_options.forward_to.as_deref(),
        Some("https://staging.example.com")
    );

    assert_eq!(rt.rewrite_path.as_deref(), Some("/v2/users"));
}

#[test]
fn test_is_full_mock_method() {
    // Template variant: is_full_mock = true
    let template = ReturnConfig::Template("hello".to_string());
    assert!(template.is_full_mock());

    // StatusShortcuts variant: is_full_mock = true
    let mut shortcuts = FxHashMap::default();
    shortcuts.insert(200u16, "ok".to_string());
    let status_shortcuts = ReturnConfig::StatusShortcuts(shortcuts);
    assert!(status_shortcuts.is_full_mock());

    // Structured with body Some: is_full_mock = true
    let with_body = ReturnConfig::Structured {
        status: Some(200),
        headers: FxHashMap::default(),
        body: Some("test".to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    };
    assert!(with_body.is_full_mock());

    // Structured with json object (non-null): is_full_mock = true
    let with_json = ReturnConfig::Structured {
        status: Some(200),
        headers: FxHashMap::default(),
        body: None,
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::json!({"key": "value"})),
    };
    assert!(with_json.is_full_mock());

    // Structured with neither body nor json: is_full_mock = false
    let empty_structured = ReturnConfig::Structured {
        status: Some(200),
        headers: FxHashMap::default(),
        body: None,
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    };
    assert!(!empty_structured.is_full_mock());
}
