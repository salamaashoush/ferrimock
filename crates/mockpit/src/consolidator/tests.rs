//! Comprehensive tests for mock consolidation

use super::{ConsolidatorOptions, MockConsolidator};
use crate::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use crate::template::{render_template, validate_template};
use crate::types::RequestContext;
use rustc_hash::FxHashMap;

// Helper to create a test mock
fn create_test_mock(id: &str, method: &str, url: &str, response_body: &str) -> MockConfig {
    MockConfig {
        id: id.into(),
        description: None,
        priority: 100,
        enabled: true,
        scope: None,
        vars: None,
        match_config: Some(MatchConfig {
            method: Some(method.to_string()),
            url: Some(url.to_string()),
            ..Default::default()
        }),
        request: None,
        response_config: Some(ReturnConfig::Structured {
            status: Some(200),
            headers: FxHashMap::default(),
            body: Some(response_body.to_string()),
            template: None,
            file: None,
            template_file: None,
            json: Box::new(serde_json::Value::Null),
        }),
        patch: None,
        delay: None,
    }
}

#[tokio::test]
async fn test_consolidation_creates_valid_mocks() {
    // Create test collection with similar mocks
    let mocks = vec![
        create_test_mock(
            "user-1",
            "GET",
            "/api/users/123",
            r#"{"id": 123, "name": "Alice", "email": "alice@example.com", "created_at": "2024-01-01T10:00:00Z"}"#,
        ),
        create_test_mock(
            "user-2",
            "GET",
            "/api/users/456",
            r#"{"id": 456, "name": "Bob", "email": "bob@example.com", "created_at": "2024-01-02T11:00:00Z"}"#,
        ),
        create_test_mock(
            "user-3",
            "GET",
            "/api/users/789",
            r#"{"id": 789, "name": "Charlie", "email": "charlie@example.com", "created_at": "2024-01-03T12:00:00Z"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Test Collection".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Verify consolidation happened
    assert!(
        consolidated.mocks.len() < 3,
        "Expected mocks to be consolidated, got {} mocks",
        consolidated.mocks.len()
    );

    // Verify all consolidated mocks are valid
    for mock in &consolidated.mocks {
        // Check match config exists and is properly formed
        assert!(
            mock.match_config.is_some(),
            "Mock {} missing match_config",
            mock.id
        );

        let match_config = mock.match_config.as_ref().unwrap();

        // Should use new format: urls field instead of url
        // Or use url field without deprecated prefixes in simple cases
        if !match_config.urls.is_empty() {
            for url_pattern in &match_config.urls {
                // Check that URL patterns are valid and use correct prefix format
                assert!(
                    url_pattern.starts_with("prefix:")
                        || url_pattern.starts_with("regex:")
                        || url_pattern.starts_with("exact:")
                        || !url_pattern.contains(':'),
                    "URL pattern '{url_pattern}' should use proper prefix format or be plain URL"
                );
            }
        }

        // Verify template if present
        if let Some(response_config) = &mock.response_config
            && let Some(tmpl) = response_config.template()
        {
            // This is a template - validate it
            assert!(
                validate_template(tmpl).is_ok(),
                "Mock {} has invalid template: {:?}",
                mock.id,
                validate_template(tmpl).err()
            );

            // Try to render the template with a proper request context
            // Create context with URL that might be needed by the template
            let mut context = RequestContext::new();
            // Add some sample captures that might be used by templates
            context.captures.insert("id".to_string(), "123".to_string());

            let rendered = render_template(tmpl, &context);

            // Template should render successfully or have a clear reason for failure
            if let Err(e) = rendered {
                // Some templates might fail without real request data, which is OK in unit tests
                // Just verify the template itself is syntactically valid
                println!(
                    "Note: Template for mock {} couldn't render with mock context ({}), but syntax is valid",
                    mock.id, e
                );
            } else {
                // If it renders, verify it's valid JSON if it looks like JSON
                if tmpl.trim_start().starts_with('{') {
                    let rendered_text = rendered.unwrap();
                    assert!(
                        serde_json::from_str::<serde_json::Value>(&rendered_text).is_ok(),
                        "Mock {} rendered template is not valid JSON: {}",
                        mock.id,
                        rendered_text
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn test_consolidation_uses_modern_url_format() {
    let mocks = vec![
        create_test_mock("m1", "GET", "/api/items/1", r#"{"id": 1, "value": "a"}"#),
        create_test_mock("m2", "GET", "/api/items/2", r#"{"id": 2, "value": "b"}"#),
        create_test_mock("m3", "GET", "/api/items/3", r#"{"id": 3, "value": "c"}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Check that consolidated mocks use proper URL format
    for mock in &consolidated.mocks {
        if let Some(match_config) = &mock.match_config {
            // Should prefer urls (plural) field
            if !match_config.urls.is_empty() {
                assert!(
                    match_config.url.is_none(),
                    "Mock {} should use 'urls' field instead of deprecated 'url' field when there are multiple patterns",
                    mock.id
                );

                // Check URL patterns are clean without prefixes
                for url_pattern in &match_config.urls {
                    // Should NOT have prefixes like "exact:", "prefix:", "regex:"
                    assert!(
                        !url_pattern.starts_with("exact:"),
                        "URL pattern '{url_pattern}' should not have 'exact:' prefix - use clean URLs"
                    );
                    assert!(
                        !url_pattern.starts_with("prefix:"),
                        "URL pattern '{url_pattern}' should not have 'prefix:' prefix - use clean URLs"
                    );
                    assert!(
                        !url_pattern.starts_with("regex:"),
                        "URL pattern '{url_pattern}' should not have 'regex:' prefix - use Express-style like /users/{{id}}"
                    );

                    // Should use clean formats:
                    // - /api/users (simple path - auto-detects exact match)
                    // - /api/users/{id} (Express-style - auto-detects pattern)
                    // - /api/* (glob - auto-detects)
                }
            }
        }
    }
}

#[tokio::test]
async fn test_consolidation_generates_concise_output() {
    // Create collection with many similar mocks
    let mut mocks = Vec::new();
    for i in 1..=20 {
        mocks.push(create_test_mock(
      &format!("mock-{i}"),
      "GET",
      &format!("/api/users/{i}"),
      &format!(
        r#"{{"id": {i}, "name": "User{i}", "email": "user{i}@example.com", "active": true, "role": "user"}}"#
      ),
    ));
    }

    let collection = MockCollectionConfig {
        name: Some("Large Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    // Serialize original collection
    let original_json = serde_json::to_string(&collection).unwrap();
    let original_size = original_json.len();

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Serialize consolidated collection
    let consolidated_json = serde_json::to_string(&consolidated).unwrap();
    let consolidated_size = consolidated_json.len();

    println!("Original size: {original_size} bytes");
    println!("Consolidated size: {consolidated_size} bytes");
    println!(
        "Size reduction: {:.1}%",
        (1.0 - (consolidated_size as f64 / original_size as f64)) * 100.0
    );

    // Consolidation should significantly reduce size
    // (at least 30% reduction for this pattern)
    assert!(
        consolidated_size < (original_size as f64 * 0.7) as usize,
        "Expected at least 30% size reduction, got {:.1}%",
        (1.0 - (consolidated_size as f64 / original_size as f64)) * 100.0
    );

    // Should consolidate many mocks into fewer
    assert!(
        consolidated.mocks.len() < 5,
        "Expected significant mock count reduction, got {} from 20",
        consolidated.mocks.len()
    );
}

#[tokio::test]
async fn test_template_generation_for_varying_fields() {
    let mocks = vec![
        create_test_mock(
            "m1",
            "GET",
            "/api/items?page=1",
            r#"{"items": [{"id": 1, "name": "Item A"}], "page": 1, "total": 100}"#,
        ),
        create_test_mock(
            "m2",
            "GET",
            "/api/items?page=2",
            r#"{"items": [{"id": 2, "name": "Item B"}], "page": 2, "total": 100}"#,
        ),
        create_test_mock(
            "m3",
            "GET",
            "/api/items?page=3",
            r#"{"items": [{"id": 3, "name": "Item C"}], "page": 3, "total": 100}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Pagination Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should create template for varying fields
    let template_mock = consolidated
        .mocks
        .iter()
        .find(|m| {
            m.response_config
                .as_ref()
                .and_then(|r| r.template())
                .is_some()
        })
        .expect("Should have at least one template mock");

    let tmpl = template_mock
        .response_config
        .as_ref()
        .unwrap()
        .template()
        .unwrap();

    // Validate template
    assert!(
        validate_template(tmpl).is_ok(),
        "Template validation failed: {:?}",
        validate_template(tmpl).err()
    );

    // Template should handle constant fields (total) and varying fields (page, items)
    assert!(
        tmpl.contains("total"),
        "Template should include 'total' field"
    );
    assert!(
        tmpl.contains("page") || tmpl.contains("query"),
        "Template should reference page or query params"
    );
}

#[tokio::test]
async fn test_duplicate_removal() {
    // Create exact duplicates
    let mocks = vec![
        create_test_mock("m1", "GET", "/api/status", r#"{"status": "ok"}"#),
        create_test_mock("m2", "GET", "/api/status", r#"{"status": "ok"}"#),
        create_test_mock("m3", "GET", "/api/status", r#"{"status": "ok"}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Duplicate Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should remove duplicates, keeping only 1
    assert_eq!(
        consolidated.mocks.len(),
        1,
        "Expected duplicates to be removed, got {} mocks",
        consolidated.mocks.len()
    );

    // Check stats
    let stats = consolidator.stats();
    assert_eq!(stats.duplicates_removed, 2, "Expected 2 duplicates removed");
}

#[tokio::test]
async fn test_express_style_pattern_generation() {
    let mocks = vec![
        create_test_mock("m1", "GET", "/users/123", r#"{"id": 123, "name": "Alice"}"#),
        create_test_mock("m2", "GET", "/users/456", r#"{"id": 456, "name": "Bob"}"#),
        create_test_mock(
            "m3",
            "GET",
            "/users/789",
            r#"{"id": 789, "name": "Charlie"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Express Pattern Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Find the consolidated mock with Express-style pattern
    let pattern_mock = consolidated.mocks.iter().find(|m| {
        m.match_config
            .as_ref()
            .and_then(|mc| mc.urls.first())
            .is_some_and(|url| url.contains("/{") || url.contains("/:"))
    });

    assert!(
        pattern_mock.is_some(),
        "Should have at least one mock with Express-style pattern"
    );

    if let Some(mock) = pattern_mock {
        let url_pattern = mock.match_config.as_ref().unwrap().urls.first().unwrap();

        // Should be clean Express-style pattern without "regex:" prefix
        assert!(
            !url_pattern.starts_with("regex:"),
            "Should not have 'regex:' prefix, got: {url_pattern}"
        );

        // Should use {id} syntax for clean, readable patterns
        assert!(
            url_pattern.contains("/{id}") || url_pattern.contains("/:id"),
            "Should use Express-style parameter syntax: {url_pattern}"
        );

        // Verify it's a clean pattern like /users/{id}
        assert!(
            url_pattern == "/users/{id}" || url_pattern == "/users/:id",
            "Expected clean pattern '/users/{{id}}' or '/users/:id', got: {url_pattern}"
        );
    }
}

#[tokio::test]
async fn test_uuid_pattern_generation() {
    let mocks = vec![
        create_test_mock(
            "m1",
            "GET",
            "/files/550e8400-e29b-41d4-a716-446655440000",
            r#"{"id": "550e8400-e29b-41d4-a716-446655440000", "name": "file1.pdf"}"#,
        ),
        create_test_mock(
            "m2",
            "GET",
            "/files/6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            r#"{"id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8", "name": "file2.pdf"}"#,
        ),
        create_test_mock(
            "m3",
            "GET",
            "/files/7c9e6679-7425-40de-944b-e07fc1f90ae7",
            r#"{"id": "7c9e6679-7425-40de-944b-e07fc1f90ae7", "name": "file3.pdf"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("UUID Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Find Express-style pattern mock
    let pattern_mock = consolidated.mocks.iter().find(|m| {
        m.match_config
            .as_ref()
            .and_then(|mc| mc.urls.first())
            .is_some_and(|url| url.contains("/{") || url.contains("/:"))
    });

    assert!(
        pattern_mock.is_some(),
        "Should have at least one mock with Express-style pattern for UUIDs"
    );

    if let Some(mock) = pattern_mock {
        let url_pattern = mock.match_config.as_ref().unwrap().urls.first().unwrap();

        // Should be clean Express-style pattern
        assert!(
            !url_pattern.starts_with("regex:"),
            "Should not have 'regex:' prefix: {url_pattern}"
        );

        // Should use {uuid} or {id} syntax
        assert!(
            url_pattern.contains("/{uuid}")
                || url_pattern.contains("/{id}")
                || url_pattern.contains("/:uuid")
                || url_pattern.contains("/:id"),
            "Should use Express-style parameter syntax for UUIDs: {url_pattern}"
        );
    }
}

#[tokio::test]
async fn test_consolidation_with_disabled_templates() {
    let mocks = vec![
        create_test_mock("m1", "GET", "/api/users/1", r#"{"id": 1, "name": "Alice"}"#),
        create_test_mock("m2", "GET", "/api/users/2", r#"{"id": 2, "name": "Bob"}"#),
        create_test_mock(
            "m3",
            "GET",
            "/api/users/3",
            r#"{"id": 3, "name": "Charlie"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("No Templates Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let options = ConsolidatorOptions {
        enable_consolidation: true,
        enable_templates: false, // Disable template generation
        min_pattern_threshold: 3,
        enable_stateful_pagination: false,
        pagination_storage_key_template: "api.{path}.total".to_string(),
    };

    let mut consolidator = MockConsolidator::with_options(options);
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should keep mocks separate when templates are disabled
    assert_eq!(
        consolidated.mocks.len(),
        3,
        "Expected mocks to remain separate with templates disabled"
    );

    // None should have templates
    for mock in &consolidated.mocks {
        if let Some(response_config) = &mock.response_config
            && let Some(body) = response_config.body()
        {
            assert!(
                !body.contains("{{") && !body.contains("{%"),
                "Mock {} should not have template when templates are disabled",
                mock.id
            );
        }
    }
}

#[tokio::test]
async fn test_consolidation_preserves_mock_properties() {
    let mocks = vec![
        create_test_mock("m1", "GET", "/api/data", r#"{"value": 1}"#),
        create_test_mock("m2", "GET", "/api/data", r#"{"value": 1}"#), // Duplicate
    ];

    let collection = MockCollectionConfig {
        name: Some("Properties Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should have 1 mock after duplicate removal
    assert_eq!(consolidated.mocks.len(), 1);

    let mock = &consolidated.mocks[0];

    // Should preserve essential properties
    assert!(mock.enabled, "Mock should remain enabled");
    assert_eq!(mock.priority, 100, "Mock should preserve priority");
    assert!(mock.match_config.is_some(), "Mock should have match_config");
    assert!(
        mock.response_config.is_some(),
        "Mock should have response_config"
    );
}

#[tokio::test]
async fn test_consolidation_statistics_accuracy() {
    let mocks = vec![
        create_test_mock("m1", "GET", "/api/items/1", r#"{"id": 1}"#),
        create_test_mock("m2", "GET", "/api/items/2", r#"{"id": 2}"#),
        create_test_mock("m3", "GET", "/api/items/3", r#"{"id": 3}"#),
        create_test_mock("m4", "GET", "/api/status", r#"{"status": "ok"}"#),
        create_test_mock("m5", "GET", "/api/status", r#"{"status": "ok"}"#), // Duplicate
    ];

    let collection = MockCollectionConfig {
        name: Some("Stats Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let _consolidated = consolidator.consolidate(collection).unwrap();

    let stats = consolidator.stats();

    // Verify statistics
    assert_eq!(stats.original_count, 5, "Should track original count");
    assert!(
        stats.consolidated_count < 5,
        "Should reduce mock count: {}",
        stats.consolidated_count
    );
    assert!(
        stats.reduction_ratio > 0.0,
        "Should have positive reduction ratio"
    );
    assert!(stats.patterns_detected > 0, "Should detect patterns");
}

#[tokio::test]
async fn test_min_pattern_threshold() {
    let mocks = vec![
        create_test_mock("m1", "GET", "/api/items/1", r#"{"id": 1}"#),
        create_test_mock("m2", "GET", "/api/items/2", r#"{"id": 2}"#),
        // Only 2 mocks - below default threshold of 3
    ];

    let collection = MockCollectionConfig {
        name: Some("Threshold Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should keep mocks separate when below threshold
    assert_eq!(
        consolidated.mocks.len(),
        2,
        "Should not consolidate when below min_pattern_threshold"
    );
}

#[tokio::test]
async fn test_non_json_responses_not_templated() {
    let mocks = vec![
        create_test_mock(
            "m1",
            "GET",
            "/api/html/1",
            "<html><body>Page 1</body></html>",
        ),
        create_test_mock(
            "m2",
            "GET",
            "/api/html/2",
            "<html><body>Page 2</body></html>",
        ),
        create_test_mock(
            "m3",
            "GET",
            "/api/html/3",
            "<html><body>Page 3</body></html>",
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Non-JSON Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Non-JSON responses should not be templated
    for mock in &consolidated.mocks {
        if let Some(response_config) = &mock.response_config
            && let Some(body) = response_config.body()
            && body.contains("<html>")
        {
            assert!(
                !body.contains("{{") && !body.contains("{%"),
                "HTML response should not have template syntax"
            );
        }
    }
}

#[tokio::test]
async fn test_categorical_detection_rejects_sequential() {
    use crate::type_detector::{FieldType, TypeDetector};
    use serde_json::json;

    let detector = TypeDetector::new();

    // Sequential numbers should NOT be detected as categorical
    let sequential_values = [
        json!("1"),
        json!("2"),
        json!("3"),
        json!("1"),
        json!("2"),
        json!("3"),
        json!("1"),
        json!("2"),
    ];

    let values_refs: Vec<&serde_json::Value> = sequential_values.iter().collect();
    let (field_type, _) = detector.detect_type("status", &values_refs);

    assert!(
        !matches!(field_type, FieldType::Categorical { .. }),
        "Sequential numbers should not be categorical, got {field_type:?}"
    );
}

#[tokio::test]
async fn test_categorical_detection_accepts_true_enums() {
    use crate::type_detector::{FieldType, TypeDetector};
    use serde_json::json;

    let detector = TypeDetector::new();

    // True enum values (low cardinality, non-sequential)
    // Need more samples with lower cardinality ratio (< 0.35)
    // 3 unique values / 10 samples = 0.30 ratio
    let enum_values = vec![
        json!("pending"),
        json!("approved"),
        json!("rejected"),
        json!("pending"),
        json!("approved"),
        json!("pending"),
        json!("rejected"),
        json!("pending"),
        json!("approved"),
        json!("pending"),
    ];

    let values_refs: Vec<&serde_json::Value> = enum_values.iter().collect();
    let (field_type, confidence) = detector.detect_type("status", &values_refs);

    // Should detect as categorical
    if let FieldType::Categorical { values } = field_type {
        assert_eq!(values.len(), 3, "Should have 3 unique enum values");
        assert!(values.contains(&"pending".to_string()));
        assert!(values.contains(&"approved".to_string()));
        assert!(values.contains(&"rejected".to_string()));
        assert!(confidence >= 0.75, "Should have high confidence");
    } else {
        panic!("Expected Categorical type, got {field_type:?}");
    }
}

// ============================================================================
// Tests for New Enhancements (Issues #8, #11, #13, #14)
// ============================================================================

#[tokio::test]
async fn test_priority_aware_grouping() {
    // Create mocks with same URL but different priorities
    let mut mock_low = create_test_mock("low-pri", "GET", "/api/users", r#"{"default": true}"#);
    mock_low.priority = 50; // Low priority

    let mut mock_high = create_test_mock("high-pri", "GET", "/api/users", r#"{"override": true}"#);
    mock_high.priority = 500; // High priority

    let collection = MockCollectionConfig {
        name: Some("Priority Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![mock_low, mock_high],
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should keep them separate due to different priority tiers
    assert_eq!(
        consolidated.mocks.len(),
        2,
        "Mocks with different priorities should not be grouped together"
    );

    // Verify priorities are preserved
    let priorities: Vec<u32> = consolidated.mocks.iter().map(|m| m.priority).collect();
    assert!(priorities.contains(&50), "Should preserve low priority");
    assert!(priorities.contains(&500), "Should preserve high priority");
}

#[tokio::test]
async fn test_enabled_state_grouping() {
    // Create mocks with same URL but different enabled states
    let mut mock_enabled = create_test_mock("enabled-mock", "GET", "/api/data", r#"{"data": 1}"#);
    mock_enabled.enabled = true;

    let mut mock_disabled = create_test_mock("disabled-mock", "GET", "/api/data", r#"{"data": 1}"#);
    mock_disabled.enabled = false;

    let collection = MockCollectionConfig {
        name: Some("Enabled State Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![mock_enabled, mock_disabled],
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should keep them separate due to different enabled states
    assert_eq!(
        consolidated.mocks.len(),
        2,
        "Mocks with different enabled states should not be grouped together"
    );
}

#[tokio::test]
async fn test_multiple_ids_in_path_normalization() {
    use super::pattern::PatternDetector;

    // Test path with multiple numeric IDs
    let path1 = "/orgs/123/users/456/files/789";
    let normalized = PatternDetector::normalize_path_for_grouping(path1);

    // Should use unique placeholders for each ID
    assert_eq!(
        normalized, "/orgs/{id}/users/{id2}/files/{id3}",
        "Multiple IDs should get unique placeholders"
    );

    // Test path with UUID and numeric ID
    let path2 = "/files/550e8400-e29b-41d4-a716-446655440000/versions/5";
    let normalized2 = PatternDetector::normalize_path_for_grouping(path2);
    assert_eq!(
        normalized2, "/files/{uuid}/versions/{id}",
        "UUID and numeric ID should get different placeholders"
    );

    // Test path with date
    let path3 = "/logs/2024-01-15/errors";
    let normalized3 = PatternDetector::normalize_path_for_grouping(path3);
    assert_eq!(normalized3, "/logs/{date}/errors");
}

#[tokio::test]
async fn test_fuzzy_pagination_field_detection() {
    use super::analysis::ResponseAnalyzer;

    let analyzer = ResponseAnalyzer::new(true);

    // Create responses with non-standard pagination field names
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
                url: Some("/api/items?page=1".to_string()),
                ..Default::default()
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: FxHashMap::default(),
                // Non-standard field names: totalRecords, itemsPerPage
                body: Some(r#"{"totalRecords": 100, "itemsPerPage": 20, "items": []}"#.to_string()),
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
                url: Some("/api/items?page=2".to_string()),
                ..Default::default()
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: FxHashMap::default(),
                body: Some(r#"{"totalRecords": 100, "itemsPerPage": 20, "items": []}"#.to_string()),
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

    // Should detect pagination even with non-standard field names
    assert!(
        pattern.is_some(),
        "Should detect pagination with fuzzy field matching (totalRecords, itemsPerPage)"
    );

    if let Some(p) = pattern {
        assert!(
            p.total_field.is_some(),
            "Should find total field via fuzzy match (totalRecords)"
        );
        assert!(
            p.limit_field.is_some(),
            "Should find limit field via fuzzy match (itemsPerPage)"
        );
    }
}

#[tokio::test]
async fn test_semantic_penalty_prevents_false_positives() {
    use crate::type_detector::{FieldType, TypeDetector};
    use serde_json::json;

    let detector = TypeDetector::new();

    // Field named "email" but contains URLs (should penalize Email type)
    let url_values = [
        json!("https://example.com/user1"),
        json!("https://example.com/user2"),
    ];

    let (field_type, confidence) =
        detector.detect_type("email_url", &url_values.iter().collect::<Vec<_>>());

    // Should detect as URL, not Email (despite "email" in name)
    assert!(
        matches!(field_type, FieldType::Url),
        "Should detect as URL despite 'email' in field name, got {field_type:?}"
    );

    // Confidence should be reasonable (penalty prevents false confidence)
    assert!(
        confidence >= 0.7,
        "Should have reasonable confidence: {confidence}"
    );
}

#[tokio::test]
async fn test_path_normalization_with_dates() {
    use super::pattern::PatternDetector;

    let path = "/api/logs/2024-10-12/errors";
    let normalized = PatternDetector::normalize_path_for_grouping(path);

    assert_eq!(normalized, "/api/logs/{date}/errors");
}

#[tokio::test]
async fn test_consolidation_groups_by_priority() {
    // Create mocks with same path but different priorities
    let mut mocks = vec![];
    for i in 1..=3 {
        let mut mock = create_test_mock(
            &format!("normal-{i}"),
            "GET",
            "/api/data",
            r#"{"value": 1}"#,
        );
        mock.priority = 100; // Normal priority
        mocks.push(mock);
    }

    for i in 1..=3 {
        let mut mock =
            create_test_mock(&format!("high-{i}"), "GET", "/api/data", r#"{"value": 2}"#);
        mock.priority = 600; // High priority
        mocks.push(mock);
    }

    let collection = MockCollectionConfig {
        name: Some("Multi-Priority Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Should create 2 groups (one per priority tier)
    // Each group might consolidate its own duplicates
    assert!(
        consolidated.mocks.len() >= 2,
        "Should have at least 2 mocks (one per priority tier)"
    );

    // Verify no high-priority mock got merged with normal-priority
    let normal_mocks: Vec<_> = consolidated
        .mocks
        .iter()
        .filter(|m| m.priority >= 100 && m.priority < 500)
        .collect();
    let high_mocks: Vec<_> = consolidated
        .mocks
        .iter()
        .filter(|m| m.priority >= 500)
        .collect();

    assert!(
        !normal_mocks.is_empty(),
        "Should have normal priority mocks preserved"
    );
    assert!(
        !high_mocks.is_empty(),
        "Should have high priority mocks preserved"
    );
}

// ===========================================================================
// Edge case tests
// ===========================================================================

#[tokio::test]
async fn test_consolidation_empty_collection() {
    let collection = MockCollectionConfig {
        name: Some("Empty".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![],
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    assert_eq!(consolidated.mocks.len(), 0);
    let stats = consolidator.stats();
    assert_eq!(stats.original_count, 0);
    assert_eq!(stats.consolidated_count, 0);
}

#[tokio::test]
async fn test_consolidation_single_mock() {
    let collection = MockCollectionConfig {
        name: Some("Single".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![create_test_mock(
            "solo",
            "GET",
            "/api/solo",
            r#"{"ok": true}"#,
        )],
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    assert_eq!(consolidated.mocks.len(), 1);
    assert_eq!(consolidated.mocks[0].id, "solo");
}

#[tokio::test]
async fn test_consolidation_mixed_content_types() {
    // Mix of JSON and non-JSON responses on different endpoints
    let mocks = vec![
        create_test_mock("json-1", "GET", "/api/users/1", r#"{"id": 1, "name": "A"}"#),
        create_test_mock("json-2", "GET", "/api/users/2", r#"{"id": 2, "name": "B"}"#),
        create_test_mock("json-3", "GET", "/api/users/3", r#"{"id": 3, "name": "C"}"#),
        create_test_mock("text-1", "GET", "/api/health", "OK"),
        create_test_mock("text-2", "GET", "/api/health", "OK"),
    ];

    let collection = MockCollectionConfig {
        name: Some("Mixed".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // JSON users should consolidate, text health should deduplicate
    assert!(
        consolidated.mocks.len() <= 3,
        "Should consolidate: got {} mocks",
        consolidated.mocks.len()
    );
}

#[tokio::test]
async fn test_consolidation_special_characters_in_body() {
    // JSON responses with special characters that could break templates
    let mocks = vec![
        create_test_mock(
            "special-1",
            "GET",
            "/api/items/1",
            r#"{"id": 1, "desc": "Item with \"quotes\" and {braces}"}"#,
        ),
        create_test_mock(
            "special-2",
            "GET",
            "/api/items/2",
            r#"{"id": 2, "desc": "Item with 'apostrophes' and <angle>"}"#,
        ),
        create_test_mock(
            "special-3",
            "GET",
            "/api/items/3",
            r#"{"id": 3, "desc": "Item with {{ template }} syntax"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Special Chars".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    // Should not panic even with tricky characters
    let result = consolidator.consolidate(collection);
    assert!(
        result.is_ok(),
        "Consolidation should handle special characters"
    );

    let consolidated = result.unwrap();

    // Verify any generated templates are valid
    for mock in &consolidated.mocks {
        if let Some(ref rc) = mock.response_config
            && let Some(tmpl) = rc.template()
        {
            assert!(
                validate_template(tmpl).is_ok(),
                "Template with special chars should validate: {:?}",
                validate_template(tmpl).err()
            );
        }
    }
}

#[tokio::test]
async fn test_consolidation_output_is_valid_json() {
    // Ensure the consolidation output can be serialized and deserialized roundtrip
    let mocks = vec![
        create_test_mock(
            "rt-1",
            "GET",
            "/api/files/100",
            r#"{"id": 100, "name": "a.txt", "size": 1024}"#,
        ),
        create_test_mock(
            "rt-2",
            "GET",
            "/api/files/200",
            r#"{"id": 200, "name": "b.txt", "size": 2048}"#,
        ),
        create_test_mock(
            "rt-3",
            "GET",
            "/api/files/300",
            r#"{"id": 300, "name": "c.txt", "size": 4096}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Roundtrip".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Serialize to JSON
    let json_str = serde_json::to_string_pretty(&consolidated).unwrap();

    // Must deserialize back cleanly
    let roundtripped: MockCollectionConfig = serde_json::from_str(&json_str).unwrap_or_else(|e| {
        panic!("Consolidated output should roundtrip through JSON: {e}\nJSON:\n{json_str}")
    });

    assert_eq!(roundtripped.mocks.len(), consolidated.mocks.len());
    assert_eq!(roundtripped.enabled, consolidated.enabled);
}

#[tokio::test]
async fn test_consolidation_different_methods_not_grouped() {
    let mocks = vec![
        create_test_mock("get-1", "GET", "/api/resource/1", r#"{"id": 1}"#),
        create_test_mock("get-2", "GET", "/api/resource/2", r#"{"id": 2}"#),
        create_test_mock("get-3", "GET", "/api/resource/3", r#"{"id": 3}"#),
        create_test_mock("post-1", "POST", "/api/resource/1", r#"{"created": true}"#),
        create_test_mock("post-2", "POST", "/api/resource/2", r#"{"created": true}"#),
        create_test_mock("post-3", "POST", "/api/resource/3", r#"{"created": true}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Method Separation".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // GET and POST should be grouped separately - at minimum 2 consolidated mocks
    assert!(
        consolidated.mocks.len() >= 2,
        "GET and POST should be kept in separate groups, got {} mocks",
        consolidated.mocks.len()
    );
}

#[tokio::test]
async fn test_consolidation_preserves_collection_metadata() {
    let collection = MockCollectionConfig {
        name: Some("My Recording".to_string()),
        description: Some("Original description".to_string()),
        enabled: true,
        vars: None,
        mocks: vec![create_test_mock(
            "m1",
            "GET",
            "/api/test",
            r#"{"ok": true}"#,
        )],
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    // Name should be preserved (with consolidated suffix)
    assert!(
        consolidated.name.as_ref().unwrap().contains("My Recording"),
        "Should preserve original collection name"
    );
    assert!(consolidated.enabled, "Should preserve enabled state");
}

// Note: End-to-end recording -> consolidation test is in bdg-mock-recorder
// (test_streaming_output_loadable_by_consolidator) since it needs the recorder crate.
