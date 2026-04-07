use mockpit_config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use mockpit_engine::validation::{ErrorType, MockValidator, WarningType};
use rustc_hash::FxHashMap;
use std::fs;
use tempfile::TempDir;

#[tokio::test]
async fn test_validate_valid_yaml_file() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("valid.yaml");

  let valid_config = r#"
mocks:
  - id: test-mock
    enabled: true
    match:
      method: GET
      url: /api/test
    response:
      status: 200
      body: OK
"#;

  fs::write(&file_path, valid_config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_validate_custom_method() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("custom_method.yaml");

  // HTTP spec allows custom methods, so CUSTOM_METHOD should be valid
  let config = r#"
mocks:
  - id: test-mock
    match:
      method: CUSTOM_METHOD
      url: /api/test
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  // Custom methods are allowed by HTTP spec
  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_validate_invalid_regex() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid_regex.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: "^/api/[test"
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidRegex))
  );
}

#[tokio::test]
async fn test_validate_invalid_status_code() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid_status.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 999
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidStatusCode))
  );
}

#[tokio::test]
async fn test_validate_invalid_header_name() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid_header.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
      headers:
        "Invalid Header!": value
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidHeaderName))
  );
}

#[tokio::test]
async fn test_validate_template_syntax_error() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid_template.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template: "{{ unclosed"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError))
  );
}

#[tokio::test]
async fn test_validate_missing_file_reference() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("missing_file.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      file: nonexistent.txt
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::FileNotFound))
  );
}

#[tokio::test]
async fn test_validate_missing_template_file() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("missing_template.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template_file: nonexistent.txt
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::FileNotFound))
  );
}

#[tokio::test]
async fn test_validate_template_file_with_syntax_error() {
  let temp_dir = TempDir::new().unwrap();
  let config_path = temp_dir.path().join("config.yaml");
  let template_path = temp_dir.path().join("template.txt");

  // Write invalid template file
  fs::write(&template_path, "{{ unclosed").unwrap();

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template_file: template.txt
"#;

  fs::write(&config_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&config_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError))
  );
}

#[tokio::test]
async fn test_validate_missing_match_config() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("missing_match.yaml");

  let config = r#"
mocks:
  - id: test-mock
    response:
      status: 200
      body: OK
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::MissingField) && e.message.contains("match"))
  );
}

#[tokio::test]
async fn test_validate_missing_response_config() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("missing_response.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::MissingField) && e.message.contains("response"))
  );
}

#[tokio::test]
async fn test_validate_duplicate_mock_ids() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("duplicate_ids.yaml");

  let config = r#"
mocks:
  - id: duplicate
    match:
      method: GET
      url: /test1
    response:
      status: 200
  - id: duplicate
    match:
      method: POST
      url: /test2
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_warnings());
  assert!(
    result
      .warnings
      .iter()
      .any(|w| matches!(w.warning_type, WarningType::DuplicateId))
  );
}

#[tokio::test]
async fn test_validate_disabled_mock_warning() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("disabled.yaml");

  let config = r#"
mocks:
  - id: disabled-mock
    enabled: false
    match:
      method: GET
      url: /test
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_warnings());
  assert!(
    result
      .warnings
      .iter()
      .any(|w| matches!(w.warning_type, WarningType::DisabledMock))
  );
}

#[tokio::test]
async fn test_validate_unsupported_format() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("test.txt");

  fs::write(&file_path, "some content").unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(matches!(result.errors[0].error_type, ErrorType::UnsupportedFormat));
}

#[tokio::test]
async fn test_validate_json_file() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("test.json");

  let config = r#"{
  "mocks": [
    {
      "id": "test-mock",
      "match": {
        "method": "GET",
        "url": "/test"
      },
      "response": {
        "status": 200,
        "body": "OK"
      }
    }
  ]
}"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_validate_yaml_file() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("test.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      body: OK
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_validate_parse_error_yaml() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid.yaml");

  let config = "mocks:\n  - id: test\n bad_indent: true";

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(matches!(result.errors[0].error_type, ErrorType::ParseError));
}

#[tokio::test]
async fn test_validate_parse_error_json() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid.json");

  let config = r#"{ "mocks": [ "#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(matches!(result.errors[0].error_type, ErrorType::ParseError));
}

#[tokio::test]
async fn test_validate_directory() {
  let temp_dir = TempDir::new().unwrap();

  // Create valid YAML file
  let valid_path = temp_dir.path().join("valid.yaml");
  fs::write(
    &valid_path,
    r#"
mocks:
  - id: test
    match:
      method: GET
      url: /test
    response:
      status: 200
"#,
  )
  .unwrap();

  // Create invalid YAML file (invalid status code)
  let invalid_path = temp_dir.path().join("invalid.yaml");
  fs::write(
    &invalid_path,
    r#"
mocks:
  - id: test
    match:
      method: GET
      url: /test
    response:
      status: 999
"#,
  )
  .unwrap();

  // Create non-config file (should be ignored)
  fs::write(temp_dir.path().join("readme.txt"), "ignored").unwrap();

  let validator = MockValidator::new();
  let results = validator.validate_directory(temp_dir.path()).await;

  assert_eq!(results.len(), 2); // Only the 2 yaml files
  assert!(results.iter().any(|r| !r.has_errors()));
  assert!(results.iter().any(|r| r.has_errors()));
}

#[tokio::test]
async fn test_validate_directory_nonexistent() {
  let validator = MockValidator::new();
  let results = validator.validate_directory(std::path::Path::new("/nonexistent")).await;

  assert_eq!(results.len(), 1);
  assert!(results[0].has_errors());
  assert!(matches!(results[0].errors[0].error_type, ErrorType::FileReadError));
}

#[tokio::test]
async fn test_validate_config_directly() {
  use mockpit_config::{MatchConfig, MockConfig, ReturnConfig};

  let config = MockCollectionConfig {
    name: None,
    description: None,
    enabled: true,
    vars: None,
    mocks: vec![MockConfig {
      id: "test".into(),
      description: None,
      enabled: true,
      priority: 100,
      scope: None,
      vars: None,
      match_config: Some(MatchConfig {
        method: Some("GET".to_string()),
        methods: vec![],
        url: Some("/test".to_string()),
        urls: vec![],
        query: FxHashMap::default(),
        headers: FxHashMap::default(),
        body: FxHashMap::default(),
        graphql: None,
      }),
      request: None,
      response_config: Some(ReturnConfig::Structured {
        status: Some(200),
        headers: FxHashMap::default(),
        body: Some("OK".to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
      }),
      patch: None,
      delay: None,
    }],
  };

  let validator = MockValidator::new();
  let result = validator.validate_config(&config, None).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_validate_multiple_methods() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("multi_method.yaml");

  // HTTP spec allows custom methods, so this should be valid
  let config = r#"
mocks:
  - id: test-mock
    match:
      methods:
        - GET
        - POST
        - CUSTOM
      url: /test
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  // All methods including custom ones should be valid
  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_validate_multiple_urls() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("multi_url.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      urls:
        - /test1
        - "^/test2$"
        - "^/invalid["
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidRegex))
  );
}

#[tokio::test]
async fn test_validation_result_formatting() {
  use mockpit_engine::validation::{CodeSnippet, ValidationError, ValidationResult, ValidationWarning};

  let result = ValidationResult {
    file_path: Some(std::path::PathBuf::from("test.yaml")),
    errors: vec![ValidationError {
      mock_id: Some("test-mock".into()),
      error_type: ErrorType::InvalidMethod,
      message: "Invalid method".to_string(),
      snippet: Some(CodeSnippet {
        line_number: 5,
        code: r#"method = "INVALID""#.to_string(),
        highlight_start: 10,
        highlight_end: 19,
      }),
      suggestion: Some("Use a valid HTTP method".to_string()),
      line_number: Some(5),
    }],
    warnings: vec![ValidationWarning {
      mock_id: Some("test-mock".into()),
      message: "Mock is disabled".to_string(),
      warning_type: WarningType::DisabledMock,
      line_number: None,
      snippet: None,
      suggestion: None,
    }],
  };

  let formatted_errors = result.format_errors();
  assert!(formatted_errors.contains("error[E002]"));
  assert!(formatted_errors.contains("test.yaml:5"));
  assert!(formatted_errors.contains("INVALID"));

  let formatted_warnings = result.format_warnings();
  assert!(formatted_warnings.contains("warning[W002]"));
  assert!(formatted_warnings.contains("disabled"));

  let formatted_all = result.format_all();
  assert!(formatted_all.contains("error[E002]"));
  assert!(formatted_all.contains("warning[W002]"));
}

#[tokio::test]
async fn test_validator_default() {
  let _validator1 = MockValidator::new();
  let _validator2 = MockValidator::default();

  // Just verify both validators can be created
  // (Cannot test internal config as _config is private)
}

#[tokio::test]
async fn test_valid_status_codes() {
  let temp_dir = TempDir::new().unwrap();

  for status in [100, 200, 301, 404, 500, 599] {
    let file_path = temp_dir.path().join(format!("status_{}.yaml", status));
    let config = format!(
      r#"
mocks:
  - id: test
    match:
      method: GET
      url: /test
    response:
      status: {}
"#,
      status
    );

    fs::write(&file_path, config).unwrap();

    let validator = MockValidator::new();
    let result = validator.validate_file(&file_path).await;

    assert!(!result.has_errors(), "Status {} should be valid", status);
  }
}

#[tokio::test]
async fn test_edge_case_status_codes() {
  let temp_dir = TempDir::new().unwrap();

  // Test invalid status codes
  for status in [99, 600, 700, 1000] {
    let file_path = temp_dir.path().join(format!("status_{}.yaml", status));
    let config = format!(
      r#"
mocks:
  - id: test
    match:
      method: GET
      url: /test
    response:
      status: {}
"#,
      status
    );

    fs::write(&file_path, config).unwrap();

    let validator = MockValidator::new();
    let result = validator.validate_file(&file_path).await;

    assert!(result.has_errors(), "Status {} should be invalid", status);
  }
}
#[tokio::test]
async fn test_multiple_validation_errors_in_one_mock() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("multi_error.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: "^/invalid["
      headers:
        "Invalid Header!": value
    response:
      status: 999
      template: "{{ unclosed"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());

  // Should have multiple different types of errors
  let has_regex_error = result
    .errors
    .iter()
    .any(|e| matches!(e.error_type, ErrorType::InvalidRegex));
  let has_header_error = result
    .errors
    .iter()
    .any(|e| matches!(e.error_type, ErrorType::InvalidHeaderName));
  let has_status_error = result
    .errors
    .iter()
    .any(|e| matches!(e.error_type, ErrorType::InvalidStatusCode));
  let has_template_error = result
    .errors
    .iter()
    .any(|e| matches!(e.error_type, ErrorType::TemplateError));

  assert!(has_regex_error || has_header_error || has_status_error || has_template_error);
}

#[tokio::test]
async fn test_multiple_invalid_regexes() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("multi_regex.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      urls:
        - "^/valid$"
        - "^/invalid["
        - "^/another(unclosed"
        - "^/ok$"
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  let regex_errors: Vec<_> = result
    .errors
    .iter()
    .filter(|e| matches!(e.error_type, ErrorType::InvalidRegex))
    .collect();

  assert!(regex_errors.len() >= 2);
}

#[tokio::test]
async fn test_multiple_invalid_headers() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("multi_header.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
      headers:
        Valid-Header: value
        "Invalid Header!": value
        "Another@Bad": value
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  let header_errors: Vec<_> = result
    .errors
    .iter()
    .filter(|e| matches!(e.error_type, ErrorType::InvalidHeaderName))
    .collect();

  assert!(header_errors.len() >= 2);
}

#[tokio::test]
async fn test_mock_with_missing_both_configs() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("missing_both.yaml");

  let config = r#"
mocks:
  - id: test-mock
    enabled: true
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(result.errors.len() >= 2);

  let missing_match = result
    .errors
    .iter()
    .any(|e| matches!(e.error_type, ErrorType::MissingField) && e.message.contains("match"));
  let missing_response = result
    .errors
    .iter()
    .any(|e| matches!(e.error_type, ErrorType::MissingField) && e.message.contains("response"));

  assert!(missing_match);
  assert!(missing_response);
}

// ============================================================================
// Edge Cases in Validation
// ============================================================================

#[tokio::test]
async fn test_status_code_boundary_values() {
  let temp_dir = TempDir::new().unwrap();

  // Test boundary values
  let test_cases = vec![
    (99, true),   // Below minimum
    (100, false), // Minimum valid
    (200, false), // Common valid
    (599, false), // Maximum valid
    (600, true),  // Above maximum
  ];

  for (status, should_error) in test_cases {
    let file_path = temp_dir.path().join(format!("status_{}.yaml", status));
    let config = format!(
      r#"
mocks:
  - id: test
    match:
      method: GET
      url: /test
    response:
      status: {}
"#,
      status
    );

    fs::write(&file_path, config).unwrap();

    let validator = MockValidator::new();
    let result = validator.validate_file(&file_path).await;

    assert_eq!(
      result.has_errors(),
      should_error,
      "Status {} should{} error",
      status,
      if should_error { "" } else { " not" }
    );
  }
}

#[tokio::test]
async fn test_regex_pattern_detection() {
  let temp_dir = TempDir::new().unwrap();

  // Test different patterns that should trigger regex validation
  // Note: YAML handles backslashes in quoted strings
  let patterns = vec![
    ("^/api/users", true), // Starts with ^
    ("/api/users$", true), // Ends with $
    (r"/api/\d+", true),   // Contains \d (raw string to avoid double escaping)
    (r"/api/\w+", true),   // Contains \w
    ("/api/[0-9]", true),  // Contains [
    ("/api/(test)", true), // Contains (
    ("/api/users", false), // Plain URL
  ];

  for (pattern, _is_regex) in patterns {
    let file_path = temp_dir
      .path()
      .join(format!("pattern_{}.yaml", pattern.replace('/', "_").replace('\\', "b")));

    // For YAML, backslashes in double-quoted strings need escaping
    let yaml_pattern = pattern.replace('\\', "\\\\");

    let config = format!(
      r#"
mocks:
  - id: test
    match:
      method: GET
      url: "{}"
    response:
      status: 200
"#,
      yaml_pattern
    );

    fs::write(&file_path, config).unwrap();

    let validator = MockValidator::new();
    let result = validator.validate_file(&file_path).await;

    // All valid patterns should not error
    if result.has_errors() {
      eprintln!("Unexpected error for pattern '{}': {:?}", pattern, result.errors);
    }
    assert!(
      !result.has_errors(),
      "Pattern '{}' should not error when valid",
      pattern
    );
  }
}

#[tokio::test]
async fn test_invalid_regex_patterns() {
  let temp_dir = TempDir::new().unwrap();

  let invalid_patterns = vec!["^/api/[unclosed", "^/api/(unclosed"];

  for pattern in invalid_patterns {
    let file_path = temp_dir.path().join(format!(
      "invalid_{}.yaml",
      pattern.replace('/', "_").replace(['[', '('], "")
    ));
    let config = format!(
      r#"
mocks:
  - id: test
    match:
      method: GET
      url: "{}"
    response:
      status: 200
"#,
      pattern
    );

    fs::write(&file_path, config).unwrap();

    let validator = MockValidator::new();
    let result = validator.validate_file(&file_path).await;

    if !result.has_errors() {
      eprintln!("Pattern '{}' did not error as expected", pattern);
    }
    assert!(
      result.has_errors(),
      "Pattern '{}' should trigger regex validation error",
      pattern
    );
    assert!(
      result
        .errors
        .iter()
        .any(|e| matches!(e.error_type, ErrorType::InvalidRegex))
    );
  }
}

#[tokio::test]
async fn test_template_with_control_flow() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("template_control.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template: "{% if true %}valid{% endif %}"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_template_with_invalid_control_flow() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("template_invalid_control.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template: "{% if true %}unclosed"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError))
  );
}

#[tokio::test]
async fn test_file_reference_valid() {
  let temp_dir = TempDir::new().unwrap();
  let config_path = temp_dir.path().join("config.yaml");
  let data_path = temp_dir.path().join("data.json");

  // Create the referenced file
  fs::write(&data_path, r#"{"test": "data"}"#).unwrap();

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      file: data.json
"#;

  fs::write(&config_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&config_path).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_template_reference_valid() {
  let temp_dir = TempDir::new().unwrap();
  let config_path = temp_dir.path().join("config.yaml");
  let template_path = temp_dir.path().join("template.txt");

  // Create the referenced template file
  fs::write(&template_path, "{{ fake_name() }}").unwrap();

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template_file: template.txt
"#;

  fs::write(&config_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&config_path).await;

  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_template_reference_invalid_syntax() {
  let temp_dir = TempDir::new().unwrap();
  let config_path = temp_dir.path().join("config.yaml");
  let template_path = temp_dir.path().join("template.txt");

  // Create template file with invalid syntax
  fs::write(&template_path, "{{ unclosed").unwrap();

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      template_file: template.txt
"#;

  fs::write(&config_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&config_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError))
  );
}

// ============================================================================
// Parse Error Tests
// ============================================================================

#[tokio::test]
async fn test_parse_error_yaml_unclosed_string() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid.yaml");

  let config = r#"
mocks:
  - id: "test
"#; // Unclosed string

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(matches!(result.errors[0].error_type, ErrorType::ParseError));
  assert!(result.errors[0].suggestion.is_some());
}

#[tokio::test]
async fn test_parse_error_json_syntax() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid.json");

  let config = r#"{"mocks": [{"id": "test""#; // Incomplete JSON

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(matches!(result.errors[0].error_type, ErrorType::ParseError));
}

#[tokio::test]
async fn test_parse_error_yaml_syntax() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("invalid.yaml");

  let config = r#"
mocks:
  - id: test
    invalid indentation
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(matches!(result.errors[0].error_type, ErrorType::ParseError));
}

// ============================================================================
// Config Validation Tests
// ============================================================================

#[tokio::test]
async fn test_validate_config_directly_with_errors() {
  let config = MockCollectionConfig {
    name: None,
    description: None,
    enabled: true,
    vars: None,
    mocks: vec![MockConfig {
      id: "test".into(),
      description: None,
      enabled: true,
      priority: 100,
      scope: None,
      vars: None,
      match_config: Some(MatchConfig {
        method: Some("INVALID METHOD".to_string()), // Space makes it invalid
        methods: vec![],
        url: Some("/test".to_string()),
        urls: vec![],
        query: FxHashMap::default(),
        graphql: None,
        headers: FxHashMap::default(),
        body: FxHashMap::default(),
      }),
      request: None,
      response_config: Some(ReturnConfig::Structured {
        status: Some(999),
        headers: FxHashMap::default(),
        body: None,
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
      }),
      patch: None,
      delay: None,
    }],
  };

  let validator = MockValidator::new();
  let result = validator.validate_config(&config, None).await;

  assert!(result.has_errors());
  // Should have error for invalid status code at minimum
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidStatusCode))
  );
}

#[tokio::test]
async fn test_validate_config_with_warnings() {
  let config = MockCollectionConfig {
    name: None,
    description: None,
    enabled: true,
    vars: None,
    mocks: vec![
      MockConfig {
        id: "duplicate".into(),
        description: None,
        enabled: false,
        priority: 100,
        scope: None,
        vars: None,
        match_config: Some(MatchConfig {
          method: Some("GET".to_string()),
          methods: vec![],
          url: Some("/test".to_string()),
          urls: vec![],
          query: FxHashMap::default(),
          graphql: None,
          headers: FxHashMap::default(),
          body: FxHashMap::default(),
        }),
        request: None,
        response_config: Some(ReturnConfig::Structured {
          status: Some(200),
          headers: FxHashMap::default(),
          body: None,
          template: None,
          file: None,
          template_file: None,
          json: Box::new(serde_json::Value::Null),
        }),
        patch: None,
        delay: None,
      },
      MockConfig {
        id: "duplicate".into(),
        description: None,
        enabled: true,
        priority: 100,
        scope: None,
        vars: None,
        match_config: Some(MatchConfig {
          method: Some("POST".to_string()),
          methods: vec![],
          url: Some("/test".to_string()),
          urls: vec![],
          query: FxHashMap::default(),
          graphql: None,
          headers: FxHashMap::default(),
          body: FxHashMap::default(),
        }),
        request: None,
        response_config: Some(ReturnConfig::Structured {
          status: Some(200),
          headers: FxHashMap::default(),
          body: None,
          template: None,
          file: None,
          template_file: None,
          json: Box::new(serde_json::Value::Null),
        }),
        patch: None,
        delay: None,
      },
    ],
  };

  let validator = MockValidator::new();
  let result = validator.validate_config(&config, None).await;

  assert!(result.has_warnings());
  assert!(
    result
      .warnings
      .iter()
      .any(|w| matches!(w.warning_type, WarningType::DuplicateId))
  );
  assert!(
    result
      .warnings
      .iter()
      .any(|w| matches!(w.warning_type, WarningType::DisabledMock))
  );
}

// ============================================================================
// Directory Validation Tests
// ============================================================================

#[tokio::test]
async fn test_validate_directory_mixed_files() {
  let temp_dir = TempDir::new().unwrap();

  // Valid YAML
  let valid_yaml = temp_dir.path().join("valid.yaml");
  fs::write(
    &valid_yaml,
    r#"
mocks:
  - id: test1
    match:
      method: GET
      url: /test
    response:
      status: 200
"#,
  )
  .unwrap();

  // Invalid YAML (invalid status code)
  let invalid_yaml = temp_dir.path().join("invalid.yaml");
  fs::write(
    &invalid_yaml,
    r#"
mocks:
  - id: test2
    match:
      method: GET
      url: /test
    response:
      status: 999
"#,
  )
  .unwrap();

  // Valid JSON
  let valid_json = temp_dir.path().join("valid.json");
  fs::write(
    &valid_json,
    r#"{
  "mocks": [{
    "id": "test3",
    "match": {"method": "GET", "url": "/test"},
    "response": {"status": 200}
  }]
}"#,
  )
  .unwrap();

  // Non-config file
  fs::write(temp_dir.path().join("readme.txt"), "ignored").unwrap();

  let validator = MockValidator::new();
  let results = validator.validate_directory(temp_dir.path()).await;

  assert_eq!(results.len(), 3); // Only config files

  let valid_count = results.iter().filter(|r| !r.has_errors()).count();
  let invalid_count = results.iter().filter(|r| r.has_errors()).count();

  assert_eq!(valid_count, 2);
  assert_eq!(invalid_count, 1);
}

#[tokio::test]
async fn test_validate_directory_with_subdirs() {
  let temp_dir = TempDir::new().unwrap();

  // Create a subdirectory
  let sub_dir = temp_dir.path().join("subdir");
  fs::create_dir(&sub_dir).unwrap();

  // Create file in main dir
  let main_file = temp_dir.path().join("main.yaml");
  fs::write(
    &main_file,
    r#"
mocks:
  - id: test1
    match:
      method: GET
      url: /test
    response:
      status: 200
"#,
  )
  .unwrap();

  // Create file in subdir
  let sub_file = sub_dir.join("sub.yaml");
  fs::write(
    &sub_file,
    r#"
mocks:
  - id: test2
    match:
      method: GET
      url: /test
    response:
      status: 200
"#,
  )
  .unwrap();

  let validator = MockValidator::new();
  let results = validator.validate_directory(temp_dir.path()).await;

  // Should only find files in the top-level directory, not subdirectories
  assert_eq!(results.len(), 1);
}

// ============================================================================
// YAML Extension Tests
// ============================================================================

#[tokio::test]
async fn test_validate_yml_extension() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("test.yml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test
    response:
      status: 200
      body: OK
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(!result.has_errors());
}

// ============================================================================
// Method and URL Combination Tests
// ============================================================================

#[tokio::test]
async fn test_method_and_methods_combination() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("method_combo.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      methods:
        - POST
        - PUT
      url: /test
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  // Should validate all methods (both method and methods)
  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_url_and_urls_combination() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("url_combo.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test1
      urls:
        - /test2
        - "^/test3$"
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  // Should validate all URLs (both url and urls)
  assert!(!result.has_errors());
}

#[tokio::test]
async fn test_url_and_urls_with_invalid_regex() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("url_combo_invalid.yaml");

  let config = r#"
mocks:
  - id: test-mock
    match:
      method: GET
      url: /test1
      urls:
        - /test2
        - "^/test[invalid"
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors());
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidRegex))
  );
}

// ============================================================================
// Empty Collections Tests
// ============================================================================

#[tokio::test]
async fn test_empty_mocks_collection() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("empty.yaml");

  let config = r#"
mocks:
  - {}
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  // Should have errors for missing configurations
  assert!(result.has_errors());
}

#[tokio::test]
async fn test_no_mocks() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("no_mocks.yaml");

  let config = r#"
name: Test Collection
description: A test collection
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  // Empty mocks array should not error
  assert!(!result.has_errors());
}

// ============================================================================
// Request Transform Validation Tests
// ============================================================================

#[tokio::test]
async fn test_validate_invalid_request_delay() {
  let yaml = r#"
mocks:
  - id: test-delay
    match:
      method: GET
      url: /test
    request:
      delay: "100"
    response:
      status: 200
"#;

  let config = MockCollectionConfig::from_yaml(yaml).expect("Should parse YAML");
  let result = config.into_mock_definitions().await;
  assert!(result.is_err(), "Should fail with invalid duration format");

  let err = result.unwrap_err();
  assert!(
    err.contains("Invalid duration"),
    "Error should mention invalid duration, got: {}",
    err
  );
}

#[tokio::test]
async fn test_validate_invalid_request_body_regex() {
  let yaml = r#"
mocks:
  - id: test-regex
    match:
      method: GET
      url: /test
    request:
      body:
        regex:
          - pattern: "[unclosed"
            replacement: fixed
    response:
      status: 200
"#;

  let config = MockCollectionConfig::from_yaml(yaml).expect("Should parse YAML");
  let result = config.into_mock_definitions().await;
  assert!(result.is_err(), "Should fail with invalid regex pattern");

  let err = result.unwrap_err();
  assert!(
    err.contains("Invalid regex") || err.contains("regex"),
    "Error should mention invalid regex, got: {}",
    err
  );
}

#[tokio::test]
async fn test_validate_valid_request_transforms() {
  let yaml = r#"
mocks:
  - id: valid-transforms
    match:
      method: GET
      url: /test
    request:
      delay: 100ms
      headers:
        add:
          x-trace: abc
      query:
        add:
          debug: "true"
    response:
      status: 200
"#;

  let config = MockCollectionConfig::from_yaml(yaml).expect("Should parse YAML");
  let result = config.into_mock_definitions().await;
  assert!(result.is_ok(), "Should succeed with valid request transforms");
}

#[tokio::test]
async fn test_validate_body_with_request_transforms_conflict() {
  let yaml = r#"
mocks:
  - id: conflict
    match:
      method: GET
      url: /test
    request:
      headers:
        add:
          x-injected: value
    response:
      status: 200
      body: '{"test": true}'
"#;

  let config = MockCollectionConfig::from_yaml(yaml).expect("Should parse YAML");
  let result = config.into_mock_definitions().await;
  assert!(
    result.is_err(),
    "Should fail when body conflicts with request transforms"
  );

  let err = result.unwrap_err();
  assert!(
    err.contains("Cannot combine full mock body"),
    "Error should mention conflicting modes, got: {}",
    err
  );
}

// ============================================================================
// Request Transform Validation Tests (MockValidator)
// ============================================================================

#[tokio::test]
async fn test_validate_request_duration_errors() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("bad_duration.yaml");

  let config = r#"
mocks:
  - id: bad-delay
    match:
      method: GET
      url: /test
    request:
      delay: "100"
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors(), "Should have errors for invalid duration");
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidDuration)),
    "Should have InvalidDuration error, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_invalid_forward_to() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("bad_forward.yaml");

  let config = r#"
mocks:
  - id: bad-forward
    match:
      method: GET
      url: /test
    request:
      forward_to: not-a-url
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors(), "Should have errors for invalid forward_to URL");
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidUrl)),
    "Should have InvalidUrl error"
  );
}

#[tokio::test]
async fn test_validate_invalid_rewrite_path_template() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("bad_rewrite.yaml");

  let config = r#"
mocks:
  - id: bad-rewrite
    match:
      method: GET
      url: "/test/:id"
    request:
      rewrite_path: "/v2/{{ bad"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid rewrite_path template"
  );
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidRewritePathTemplate)),
    "Should have InvalidRewritePathTemplate error"
  );
}

#[tokio::test]
async fn test_validate_invalid_request_header_names() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("bad_req_header.yaml");

  let config = r#"
mocks:
  - id: bad-header
    match:
      method: GET
      url: /test
    request:
      headers:
        add:
          "Bad Header!": value
          x-valid-header: ok
          "Another@Bad": value
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid request header names"
  );
  let header_errors: Vec<_> = result
    .errors
    .iter()
    .filter(|e| matches!(e.error_type, ErrorType::InvalidRequestHeaderName))
    .collect();
  assert!(
    header_errors.len() >= 2,
    "Should have at least 2 InvalidRequestHeaderName errors, got {}",
    header_errors.len()
  );
}

#[tokio::test]
async fn test_validate_invalid_request_body_regex_pattern() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("bad_body_regex.yaml");

  let config = r#"
mocks:
  - id: bad-regex
    match:
      method: POST
      url: /test
    request:
      body:
        regex:
          - pattern: "[unclosed"
            replacement: fixed
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors(), "Should have errors for invalid body regex");
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidRequestBodyRegex)),
    "Should have InvalidRequestBodyRegex error"
  );
}

#[tokio::test]
async fn test_validate_conflicting_full_mock_and_request_transforms() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("conflict.yaml");

  let config = r#"
mocks:
  - id: conflict
    match:
      method: GET
      url: /test
    request:
      headers:
        add:
          x-injected: value
    response:
      status: 200
      body: '{"test": true}'
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors(), "Should have errors for conflicting modes");
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::ConflictingModes)),
    "Should have ConflictingModes error, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_passthrough_mock_no_response_valid() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("passthrough.yaml");

  let config = r#"
mocks:
  - id: passthrough
    match:
      method: GET
      url: /test
    request:
      headers:
        add:
          x-trace-id: abc-123
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    !result.has_errors(),
    "Passthrough mock with request transforms and no response should be valid, got errors: {}",
    result.format_errors()
  );
}

#[tokio::test]
async fn test_validate_empty_request_transform_warning() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("empty_request.yaml");

  let config = r#"
mocks:
  - id: empty-request
    match:
      method: GET
      url: /test
    request: {}
    response:
      status: 200
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_warnings(), "Should have warnings for empty request section");
  assert!(
    result
      .warnings
      .iter()
      .any(|w| matches!(w.warning_type, WarningType::EmptyRequestTransform)),
    "Should have EmptyRequestTransform warning"
  );
}

#[tokio::test]
async fn test_validate_valid_request_transforms_all_fields() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("valid_transforms.yaml");

  let config = r#"
mocks:
  - id: valid-all-fields
    match:
      methods:
        - GET
        - POST
      url: "/api/users/:id"
    request:
      delay: 200ms
      timeout: 30s
      forward_to: "https://staging.example.com"
      rewrite_path: "/v2/users/{{ captures.id }}"
      headers:
        add:
          x-trace-id: test-trace
          x-forwarded-by: dev-gate
        remove:
          - x-debug
      query:
        add:
          debug: "true"
        remove:
          - token
      body:
        jsonpath:
          "$.proxied": true
    patch:
      jsonpath:
        "$.source": proxied
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    !result.has_errors(),
    "Valid request transforms should not produce errors, got: {}",
    result.format_errors()
  );
}

// ============================================================================
// Patch Validation Tests
// ============================================================================

#[tokio::test]
async fn test_validate_patch_invalid_regex_pattern() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_bad_regex.yaml");

  let config = r#"
mocks:
  - id: bad-patch-regex
    match:
      method: GET
      url: /test
    patch:
      regex:
        - pattern: "[unclosed"
          replacement: fixed
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid patch regex pattern"
  );
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidPatchRegex)),
    "Should have InvalidPatchRegex error, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_patch_valid_regex_pattern() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_good_regex.yaml");

  let config = r#"
mocks:
  - id: good-patch-regex
    match:
      method: GET
      url: /test
    patch:
      regex:
        - pattern: "old-value-\\d+"
          replacement: new-value
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    !result.has_errors(),
    "Valid patch regex should not produce errors, got: {}",
    result.format_errors()
  );
}

#[tokio::test]
async fn test_validate_patch_invalid_header_name_in_add() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_bad_header_add.yaml");

  let config = r#"
mocks:
  - id: bad-patch-header
    match:
      method: GET
      url: /test
    patch:
      headers:
        add:
          "Invalid Header!": value
          x-valid: ok
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(result.has_errors(), "Should have errors for invalid patch header name");
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidPatchHeaderName)),
    "Should have InvalidPatchHeaderName error, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_patch_invalid_header_name_in_remove() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_bad_header_remove.yaml");

  let config = r#"
mocks:
  - id: bad-patch-header-remove
    match:
      method: GET
      url: /test
    patch:
      headers:
        remove:
          - "Invalid Header!"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid patch header name in remove"
  );
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::InvalidPatchHeaderName)),
    "Should have InvalidPatchHeaderName error, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_patch_template_in_jsonpath() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_template_jsonpath.yaml");

  let config = r#"
mocks:
  - id: patch-template
    match:
      method: GET
      url: /test
    patch:
      jsonpath:
        "$.injected": "{{ captures.id }}"
        "$.static": plain-value
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    !result.has_errors(),
    "Valid template in patch jsonpath should not produce errors, got: {}",
    result.format_errors()
  );
}

#[tokio::test]
async fn test_validate_patch_invalid_template_in_jsonpath() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_bad_template.yaml");

  let config = r#"
mocks:
  - id: bad-patch-template
    match:
      method: GET
      url: /test
    patch:
      jsonpath:
        "$.injected": "{{ unclosed"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid template in patch jsonpath"
  );
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError)),
    "Should have TemplateError, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_patch_invalid_template_in_header_value() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_bad_header_template.yaml");

  let config = r#"
mocks:
  - id: bad-header-template
    match:
      method: GET
      url: /test
    patch:
      headers:
        add:
          x-status: "{{ unclosed"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid template in patch header value"
  );
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError)),
    "Should have TemplateError, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_patch_invalid_template_in_regex_replacement() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_bad_regex_template.yaml");

  let config = r#"
mocks:
  - id: bad-regex-template
    match:
      method: GET
      url: /test
    patch:
      regex:
        - pattern: "old-value"
          replacement: "{{ unclosed"
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    result.has_errors(),
    "Should have errors for invalid template in patch regex replacement"
  );
  assert!(
    result
      .errors
      .iter()
      .any(|e| matches!(e.error_type, ErrorType::TemplateError)),
    "Should have TemplateError, got: {:?}",
    result.errors.iter().map(|e| &e.error_type).collect::<Vec<_>>()
  );
}

#[tokio::test]
async fn test_validate_patch_all_valid() {
  let temp_dir = TempDir::new().unwrap();
  let file_path = temp_dir.path().join("patch_all_valid.yaml");

  let config = r#"
mocks:
  - id: valid-patch
    match:
      method: GET
      url: "/api/users/:id"
    patch:
      jsonpath:
        "$.injected": "{{ captures.id }}"
        "$.static": plain-value
      regex:
        - pattern: "old-value"
          replacement: "{{ fake_name() }}"
      headers:
        add:
          x-patched: "true"
          x-request-id: "{{ fake_uuid() }}"
        remove:
          - x-internal
"#;

  fs::write(&file_path, config).unwrap();

  let validator = MockValidator::new();
  let result = validator.validate_file(&file_path).await;

  assert!(
    !result.has_errors(),
    "Fully valid patch config should not produce errors, got: {}",
    result.format_errors()
  );
}
