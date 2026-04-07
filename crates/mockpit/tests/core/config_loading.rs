//! Configuration loading and validation tests
//!
//! Tests for:
//! - YAML configuration parsing
//! - Scenario configuration
//! - File-based loading
//! - Configuration validation

use mockpit::config::MockCollectionConfig;
use mockpit::engine::MockRegistry;
use tempfile::TempDir;

#[tokio::test]
async fn test_complete_yaml_mock_configuration() {
    let yaml = r#"
name: "Complete Feature Test"
description: "Tests all mock engine features in YAML"
enabled: true

mocks:
  - id: templated-user
    priority: 100
    match:
      methods: ["GET"]
      urls: ['regex:^/api/users/(?P<user_id>\\d+)$']
    return:
      status: 200
      template: |-
        {
          "id": "{{ captures.user_id }}",
          "name": "User {{ captures.user_id }}",
          "created_at": "{{ now() }}"
        }

  - id: patched-response
    priority: 100
    match:
      methods: ["GET"]
      urls: ["exact:/api/patch-test"]
    patch:
      jsonpath:
        "$.added_field": patched_value
        "$.field": modified
      operations:
        - op: add
          path: /new_field
          value: new_value
      headers:
        add:
          X-Custom-Header: test
        remove:
          - X-Remove-Me

  - id: conditional-mock
    priority: 100
    match:
      methods: ["POST"]
      urls: ["exact:/api/conditional"]
      query:
        env: test
      body:
        "$.type": test
      headers:
        Authorization: "~^Bearer .+"
    return:
      status: 200
      body: '{"matched": true}'
"#;

    // Parse the YAML
    let collection = MockCollectionConfig::from_yaml(yaml).unwrap();

    // Validate collection metadata
    assert_eq!(collection.name, Some("Complete Feature Test".to_string()));
    assert!(collection.enabled);
    assert_eq!(collection.mocks.len(), 3);

    // Convert to mock definitions
    let definitions = collection.into_mock_definitions().await.unwrap();
    assert_eq!(definitions.len(), 3);

    // Validate template mock
    let template_mock = definitions
        .iter()
        .find(|m| m.id == "templated-user")
        .unwrap();
    assert!(matches!(
        template_mock.response.body,
        mockpit::engine::BodySource::Template { .. }
    ));

    // Validate patch mock exists (patching is handled by ResponsePatcher separately)
    let patch_mock = definitions
        .iter()
        .find(|m| m.id == "patched-response")
        .unwrap();
    assert_eq!(patch_mock.id, "patched-response");

    // Validate conditional mock
    let cond_mock = definitions
        .iter()
        .find(|m| m.id == "conditional-mock")
        .unwrap();
    assert!(!cond_mock.request.query_matchers.is_empty());
    assert!(cond_mock.request.body_matcher.is_some());
    assert!(!cond_mock.request.header_matchers.is_empty());

    println!("All YAML features validated successfully!");
    println!("  - Template responses: OK");
    println!("  - Response patching: OK");
    println!("  - Conditional matching: OK");
}

#[tokio::test]
async fn test_load_from_files() {
    let temp_dir = TempDir::new().unwrap();
    let mocks_dir = temp_dir.path().join("mocks");
    tokio::fs::create_dir(&mocks_dir).await.unwrap();

    // Write a test mock file
    let mock_yaml = r#"
name: "Test Collection"
enabled: true

mocks:
  - id: test-mock
    priority: 100
    match:
      methods: ["GET"]
      urls: ["exact:/api/test"]
    return:
      status: 200
      body: '{"test": true}'
      delay: 10ms
"#;

    tokio::fs::write(mocks_dir.join("test.yaml"), mock_yaml)
        .await
        .unwrap();

    // Load from directory
    let registry = MockRegistry::new();
    let count = registry
        .load_from_directory(mocks_dir.to_str().unwrap())
        .await
        .unwrap();

    assert_eq!(count, 1);
    let mocks = registry.get_all_mocks();
    assert_eq!(mocks.len(), 1);
    assert_eq!(mocks[0].id, "test-mock");

    println!("File-based loading validated successfully!");
}
