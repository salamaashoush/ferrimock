//! Comprehensive tests for mock consolidation

// Test scorers answer `name()` with a literal, which reads as needlessly
// bound against the `&str` the trait must return for scorers that name
// themselves after the artifact they were loaded from.
#![allow(clippy::unnecessary_literal_bound)]

use super::{ConsolidatorOptions, MockConsolidator};
use crate::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use crate::template::{render_template, validate_template};
use crate::types::RequestContext;
use rustc_hash::FxHashMap;
use std::sync::Arc;

// Helper to create a test mock
fn create_test_mock(id: &str, method: &str, url: &str, response_body: &str) -> MockConfig {
    MockConfig {
        id: id.into(),
        description: None,
        priority: 100,
        enabled: true,
        once: false,
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
        network_error: None,
        sse: None,
        ws: None,
        serve: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
    };

    let options = ConsolidatorOptions {
        enable_consolidation: true,
        enable_templates: false, // Disable template generation
        min_pattern_threshold: 3,
        enable_stateful_pagination: false,
        pagination_storage_key_template: "api.{path}.total".to_string(),
        ..ConsolidatorOptions::default()
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
async fn a_merged_template_is_not_outranked_by_a_recording_it_was_built_from() {
    // One endpoint answered two ways over the session. The rarer answer is
    // split into its own partition, and raising it above the template does not
    // make it more specific -- both match on exactly the same thing, so the
    // template becomes unreachable and answers nothing.
    let collection = MockCollectionConfig {
        name: Some("Same matcher, two answers".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![
            create_test_mock(
                "m1",
                "GET",
                "/v2/status",
                r#"{"state": "ready", "count": 1}"#,
            ),
            create_test_mock(
                "m2",
                "GET",
                "/v2/status",
                r#"{"state": "ready", "count": 2}"#,
            ),
            create_test_mock(
                "m3",
                "GET",
                "/v2/status",
                r#"{"state": "ready", "count": 3}"#,
            ),
            create_test_mock("m4", "GET", "/v2/status", r#"{"error": "unavailable"}"#),
        ],
        world: None,
        machines: None,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    let matcher_of = |mock: &MockConfig| {
        mock.match_config
            .as_ref()
            .map(|m| format!("{:?}{:?}", m.methods, m.urls))
            .unwrap_or_default()
    };

    let mut by_matcher: FxHashMap<String, Vec<u32>> = FxHashMap::default();
    for mock in &consolidated.mocks {
        by_matcher
            .entry(matcher_of(mock))
            .or_default()
            .push(mock.priority);
    }

    for (matcher, priorities) in by_matcher {
        if priorities.len() > 1 {
            let first = priorities.first().copied().unwrap_or_default();
            assert!(
                priorities.iter().all(|priority| *priority == first),
                "mocks matching the same thing must not outrank each other, or all but the \
                 top one are unreachable: {matcher} got {priorities:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_merged_mock_answers_with_the_id_it_matched_on() {
    // A response that wraps its resource is the common shape, and the id one
    // level down is still the id the URL asked for. Answering with a random
    // number instead makes the mock contradict the request it was given.
    let collection = MockCollectionConfig {
        name: Some("Wrapped resource".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![
            create_test_mock(
                "m1",
                "GET",
                "/v2/folder/9848115997/extras",
                r#"{"theme": {"id": 1}, "folder": {"id": 9848115997, "name": "One"}}"#,
            ),
            create_test_mock(
                "m2",
                "GET",
                "/v2/folder/9850347912/extras",
                r#"{"theme": {"id": 1}, "folder": {"id": 9850347912, "name": "Two"}}"#,
            ),
            create_test_mock(
                "m3",
                "GET",
                "/v2/folder/9850348888/extras",
                r#"{"theme": {"id": 1}, "folder": {"id": 9850348888, "name": "Three"}}"#,
            ),
        ],
        world: None,
        machines: None,
    };

    let mut consolidator = MockConsolidator::new();
    let consolidated = consolidator.consolidate(collection).unwrap();

    let template = consolidated
        .mocks
        .iter()
        .find_map(|mock| mock.response_config.as_ref().and_then(|rc| rc.template()))
        .expect("the group should have produced a template");

    assert!(
        template.contains("{{ captures.id }}"),
        "the nested id must echo the capture, got: {template}"
    );
    assert!(
        !template.contains("\"id\": {{ get_random"),
        "no id may be invented when the URL already carries it, got: {template}"
    );
}

#[tokio::test]
async fn a_scorer_decides_ahead_of_the_size_threshold() {
    use crate::consolidator::merge::{MergeCandidate, MergeScorer};

    struct AlwaysMerge;
    impl MergeScorer for AlwaysMerge {
        fn name(&self) -> &str {
            "always"
        }
        fn safe_to_merge(&self, _: &MergeCandidate<'_>) -> Option<f64> {
            Some(1.0)
        }
    }

    struct NeverMerge;
    impl MergeScorer for NeverMerge {
        fn name(&self) -> &str {
            "never"
        }
        fn safe_to_merge(&self, _: &MergeCandidate<'_>) -> Option<f64> {
            Some(0.0)
        }
    }

    // Two mocks: below the size threshold, so the built-in rule keeps them apart.
    let below_threshold = || {
        vec![
            create_test_mock("m1", "GET", "/api/items/1", r#"{"id": 1}"#),
            create_test_mock("m2", "GET", "/api/items/2", r#"{"id": 2}"#),
        ]
    };
    // Four mocks: above it, so the built-in rule merges them.
    let above_threshold = || {
        vec![
            create_test_mock("m1", "GET", "/api/items/1", r#"{"id": 1}"#),
            create_test_mock("m2", "GET", "/api/items/2", r#"{"id": 2}"#),
            create_test_mock("m3", "GET", "/api/items/3", r#"{"id": 3}"#),
            create_test_mock("m4", "GET", "/api/items/4", r#"{"id": 4}"#),
        ]
    };

    let collection = |mocks| MockCollectionConfig {
        name: Some("Scorer Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
        world: None,
        machines: None,
    };

    let consolidate = |mocks, scorer: Option<Arc<dyn MergeScorer>>| {
        let mut consolidator = MockConsolidator::with_options(ConsolidatorOptions {
            merge_scorer: scorer,
            ..ConsolidatorOptions::default()
        });
        consolidator
            .consolidate(collection(mocks))
            .unwrap()
            .mocks
            .len()
    };

    assert_eq!(
        consolidate(below_threshold(), None),
        2,
        "with no scorer the size threshold still governs"
    );
    assert_eq!(
        consolidate(below_threshold(), Some(Arc::new(AlwaysMerge))),
        1,
        "a scorer that is sure merges a group the size rule would have kept apart"
    );
    assert_eq!(
        consolidate(above_threshold(), Some(Arc::new(NeverMerge))),
        4,
        "a scorer that refuses keeps a group the size rule would have merged"
    );
}

#[tokio::test]
async fn a_scorer_that_declines_leaves_the_size_threshold_in_charge() {
    use crate::consolidator::merge::MergeScorer;

    struct Quiet;
    impl MergeScorer for Quiet {
        fn name(&self) -> &str {
            "quiet"
        }
    }

    let collection = MockCollectionConfig {
        name: Some("Declining Scorer".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![
            create_test_mock("m1", "GET", "/api/items/1", r#"{"id": 1}"#),
            create_test_mock("m2", "GET", "/api/items/2", r#"{"id": 2}"#),
        ],
        world: None,
        machines: None,
    };

    let mut consolidator = MockConsolidator::with_options(ConsolidatorOptions {
        merge_scorer: Some(Arc::new(Quiet)),
        ..ConsolidatorOptions::default()
    });

    assert_eq!(
        consolidator.consolidate(collection).unwrap().mocks.len(),
        2,
        "declining is not refusing: the group falls back to the size rule"
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
    let normalized = PatternDetector::new().normalize_path_for_grouping(path1);

    // Should use unique placeholders for each ID
    assert_eq!(
        normalized, "/orgs/{id}/users/{id2}/files/{id3}",
        "Multiple IDs should get unique placeholders"
    );

    // Test path with UUID and numeric ID
    let path2 = "/files/550e8400-e29b-41d4-a716-446655440000/versions/5";
    let normalized2 = PatternDetector::new().normalize_path_for_grouping(path2);
    assert_eq!(
        normalized2, "/files/{uuid}/versions/{id}",
        "UUID and numeric ID should get different placeholders"
    );

    // Test path with date
    let path3 = "/logs/2024-01-15/errors";
    let normalized3 = PatternDetector::new().normalize_path_for_grouping(path3);
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
            once: false,
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
            network_error: None,
            sse: None,
            ws: None,
            serve: None,
        },
        MockConfig {
            id: "test-2".into(),
            description: None,
            priority: 100,
            enabled: true,
            once: false,
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
            network_error: None,
            sse: None,
            ws: None,
            serve: None,
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
    let normalized = PatternDetector::new().normalize_path_for_grouping(path);

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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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
        world: None,
        machines: None,
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

// Note: End-to-end recording -> consolidation test lives with the recorder
// (test_streaming_output_loadable_by_consolidator) since it needs the recorder crate.

/// A GraphQL request is its operation *and* its variables.
///
/// Found by consolidating a real recording: sixteen `GetFolderMinimal` calls,
/// each for a different folder, all matched on the operation name alone. That
/// left several mocks nothing could tell apart, so one answered every folder
/// request and the rest were dead.
mod graphql_identity {
    use super::*;
    use crate::config::GraphQLMatchConfig;

    fn gql_mock(id: &str, folder: &str, body: &str) -> MockConfig {
        let mut variables = rustc_hash::FxHashMap::default();
        variables.insert("folderID".to_string(), serde_json::json!(folder));
        variables.insert("first".to_string(), serde_json::json!(20));

        MockConfig {
            id: id.into(),
            match_config: Some(MatchConfig {
                method: Some("POST".to_string()),
                url: Some("/app-api/graphql".to_string()),
                graphql: Some(GraphQLMatchConfig::Structured {
                    operation: Some("GetFolderMinimal".to_string()),
                    query: None,
                    mutation: None,
                    subscription: None,
                    introspection: None,
                    variables,
                }),
                ..MatchConfig::default()
            }),
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: rustc_hash::FxHashMap::default(),
                body: Some(body.to_string()),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            ..MockConfig::default()
        }
    }

    #[test]
    fn a_variable_that_varies_across_the_group_stops_being_pinned() {
        let group = [
            gql_mock("g1", "111", r#"{"id":"111","name":"a"}"#),
            gql_mock("g2", "222", r#"{"id":"222","name":"b"}"#),
            gql_mock("g3", "333", r#"{"id":"333","name":"c"}"#),
        ];

        let mut merged = group[0].clone();
        MockConsolidator::relax_match_to_group(&mut merged, &group);

        let matcher = merged
            .match_config
            .as_ref()
            .and_then(|m| m.graphql.as_ref())
            .expect("still a graphql matcher");

        match matcher {
            GraphQLMatchConfig::Structured {
                operation,
                variables,
                ..
            } => {
                assert_eq!(operation.as_deref(), Some("GetFolderMinimal"));
                assert!(
                    !variables.contains_key("folderID"),
                    "the folder is what varies, so it is the group's placeholder"
                );
                assert!(
                    variables.contains_key("first"),
                    "a variable every member shares still identifies the request"
                );
            }
            other => panic!("expected a structured matcher, got {other:?}"),
        }
    }

    #[test]
    fn nothing_left_to_pin_becomes_the_operation_name_alone() {
        let mut first = gql_mock("g1", "111", "{}");
        let mut second = gql_mock("g2", "222", "{}");
        for mock in [&mut first, &mut second] {
            if let Some(GraphQLMatchConfig::Structured { variables, .. }) =
                mock.match_config.as_mut().and_then(|m| m.graphql.as_mut())
            {
                variables.remove("first");
            }
        }

        let group = [first.clone(), second];
        let mut merged = first;
        MockConsolidator::relax_match_to_group(&mut merged, &group);

        assert!(
            matches!(
                merged.match_config.and_then(|m| m.graphql),
                Some(GraphQLMatchConfig::Simple(operation)) if operation == "GetFolderMinimal"
            ),
            "an empty structured matcher says the same thing less plainly"
        );
    }
}

/// A merged mock has to answer about the thing that was asked for.
///
/// Found by asking what a template does with a request parameter: a group of
/// `/v2/files/{id}` recordings became one mock that answered every request with
/// the same invented id, so a client reading the id back found it did not match
/// what it asked for. The echo existed but only fired for a field literally
/// named `id` whose value was a JSON *number* -- and most APIs
/// return ids as strings.
mod request_echo {
    use super::*;
    use crate::codegen::EchoSource;
    use crate::config::matcher::{GraphQLMatchConfig, HeaderMatchConfig};
    use crate::consolidator::analysis::ResponseAnalyzer;

    fn recorded(url: &str, body: &str) -> MockConfig {
        MockConfig {
            id: url.into(),
            match_config: Some(MatchConfig {
                method: Some("GET".to_string()),
                url: Some(format!("exact:{url}")),
                ..MatchConfig::default()
            }),
            response_config: Some(ReturnConfig::Structured {
                status: Some(200),
                headers: rustc_hash::FxHashMap::default(),
                body: Some(body.to_string()),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            ..MockConfig::default()
        }
    }

    fn echoes(group: &[MockConfig], pattern: &str) -> Vec<(String, EchoSource, String, bool)> {
        let analysis = ResponseAnalyzer::new(false)
            .analyze_response_patterns(group, pattern)
            .expect("analyses");
        let mut found: Vec<(String, EchoSource, String, bool)> = analysis
            .echoed_fields
            .into_iter()
            .map(|(field, echo)| (field, echo.source, echo.name, echo.quoted))
            .collect();
        found.sort();
        found
    }

    fn capture(path: &str, name: &str, quoted: bool) -> (String, EchoSource, String, bool) {
        (
            path.to_string(),
            EchoSource::Capture,
            name.to_string(),
            quoted,
        )
    }

    #[test]
    fn a_string_id_echoes_the_capture_it_repeated() {
        let group: Vec<MockConfig> = (101..=104)
            .map(|n| {
                recorded(
                    &format!("/v2/files/{n}"),
                    &format!(r#"{{"id":"{n}","type":"file"}}"#),
                )
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/files/{id}"),
            vec![capture("id", "id", true)],
            "a quoted id is the common case and was the one that never fired"
        );
    }

    #[test]
    fn a_number_echoes_without_quoting_it() {
        let group: Vec<MockConfig> = (101..=104)
            .map(|n| recorded(&format!("/v2/files/{n}"), &format!(r#"{{"id":{n}}}"#)))
            .collect();

        assert_eq!(
            echoes(&group, "/v2/files/{id}"),
            vec![capture("id", "id", false)],
            "quoting a JSON number would change its type in the answer"
        );
    }

    #[test]
    fn a_uuid_capture_echoes_as_readily_as_a_numeric_one() {
        let ids = [
            "550e8400-e29b-41d4-a716-446655440001",
            "550e8400-e29b-41d4-a716-446655440002",
            "550e8400-e29b-41d4-a716-446655440003",
        ];
        let group: Vec<MockConfig> = ids
            .iter()
            .map(|id| recorded(&format!("/v2/users/{id}"), &format!(r#"{{"id":"{id}"}}"#)))
            .collect();

        assert_eq!(
            echoes(&group, "/v2/users/{uuid}"),
            vec![capture("id", "uuid", true)]
        );
    }

    #[test]
    fn any_field_may_echo_not_only_one_called_id() {
        let group: Vec<MockConfig> = (7..=10)
            .map(|n| {
                recorded(
                    &format!("/v2/folders/{n}/items"),
                    &format!(r#"{{"parent_folder_id":"{n}","total":3}}"#),
                )
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/folders/{id}/items"),
            vec![capture("parent_folder_id", "id", true)]
        );
    }

    #[test]
    fn a_field_that_only_sometimes_matches_is_a_coincidence_not_an_echo() {
        // One recording agreeing is chance; the endpoint is only echoing when
        // every recording of it did.
        let group = vec![
            recorded("/v2/files/1", r#"{"id":"1","rev":"1"}"#),
            recorded("/v2/files/2", r#"{"id":"2","rev":"9"}"#),
            recorded("/v2/files/3", r#"{"id":"3","rev":"4"}"#),
        ];

        let found = echoes(&group, "/v2/files/{id}");
        assert!(
            found.iter().all(|(field, ..)| field != "rev"),
            "`rev` matched the capture once and must not be read as an echo: {found:?}"
        );
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn a_lone_recording_says_nothing_about_what_varies() {
        let group = vec![recorded("/v2/files/1", r#"{"id":"1"}"#)];
        assert!(echoes(&group, "/v2/files/{id}").is_empty());
    }

    #[test]
    fn a_query_parameter_echoes_the_same_way_a_capture_does() {
        // Nothing about the correspondence changes because the client put the
        // id after a `?` instead of in a path segment.
        let group: Vec<MockConfig> = (7001..=7004)
            .map(|n| {
                recorded(
                    &format!("/v2/search?folder_id={n}&limit=20"),
                    &format!(r#"{{"folder_id":"{n}","found":2}}"#),
                )
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/search"),
            vec![(
                "folder_id".to_string(),
                EchoSource::Query,
                "folder_id".to_string(),
                true
            )]
        );
    }

    #[test]
    fn a_query_parameter_pinned_one_by_one_is_read_too() {
        // The converter moves a query out of the URL and into `query` when it
        // can pin the parameters separately.
        let group: Vec<MockConfig> = (7001..=7003)
            .map(|n| {
                let mut mock = recorded("/v2/search", &format!(r#"{{"folder_id":"{n}"}}"#));
                mock.id = format!("search-{n}").into();
                if let Some(match_config) = mock.match_config.as_mut() {
                    match_config
                        .query
                        .insert("folder_id".to_string(), n.to_string());
                }
                mock
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/search"),
            vec![(
                "folder_id".to_string(),
                EchoSource::Query,
                "folder_id".to_string(),
                true
            )]
        );
    }

    #[test]
    fn a_graphql_variable_echoes_into_the_field_that_repeated_it() {
        let group: Vec<MockConfig> = (9848..=9851)
            .map(|n| {
                let mut mock = recorded(
                    "/app-api/graphql",
                    &format!(r#"{{"data":{{"folder":{{"id":"{n}","name":"f"}}}}}}"#),
                );
                mock.id = format!("folder-{n}").into();
                if let Some(match_config) = mock.match_config.as_mut() {
                    let mut variables = rustc_hash::FxHashMap::default();
                    variables.insert(
                        "folderID".to_string(),
                        serde_json::Value::String(n.to_string()),
                    );
                    match_config.graphql = Some(GraphQLMatchConfig::Structured {
                        operation: Some("GetFolder".to_string()),
                        query: None,
                        mutation: None,
                        subscription: None,
                        introspection: None,
                        variables,
                    });
                }
                mock
            })
            .collect();

        assert_eq!(
            echoes(&group, "/app-api/graphql"),
            vec![(
                "data.folder.id".to_string(),
                EchoSource::Body,
                "variables.folderID".to_string(),
                true
            )],
            "the id lives three levels down, which is where GraphQL puts it"
        );
    }

    #[test]
    fn a_header_the_endpoint_hands_back_is_an_echo() {
        let group: Vec<MockConfig> = ["req-a1", "req-b2", "req-c3"]
            .iter()
            .map(|token| {
                let mut mock = recorded("/v2/events", &format!(r#"{{"request_id":"{token}"}}"#));
                mock.id = (*token).to_string().into();
                if let Some(match_config) = mock.match_config.as_mut() {
                    match_config.headers.insert(
                        "X-Request-Id".to_string(),
                        HeaderMatchConfig::Exact((*token).to_string()),
                    );
                }
                mock
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/events"),
            vec![(
                "request_id".to_string(),
                EchoSource::Header,
                "x-request-id".to_string(),
                true
            )],
            "headers reach a template lowercased"
        );
    }

    #[test]
    fn a_value_nested_under_the_top_level_is_reached() {
        let group: Vec<MockConfig> = (7..=10)
            .map(|n| {
                recorded(
                    &format!("/v2/folders/{n}/items"),
                    &format!(r#"{{"parent":{{"id":"{n}","type":"folder"}},"total":3}}"#),
                )
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/folders/{id}/items"),
            vec![capture("parent.id", "id", true)]
        );
    }

    #[test]
    fn a_value_every_element_of_an_array_repeats_is_an_echo() {
        let group: Vec<MockConfig> = (7..=10)
            .map(|n| {
                recorded(
                    &format!("/v2/folders/{n}/items"),
                    &format!(
                        r#"{{"entries":[{{"id":"a","parent":{{"id":"{n}"}}}},{{"id":"b","parent":{{"id":"{n}"}}}}]}}"#
                    ),
                )
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/folders/{id}/items"),
            vec![capture("entries[].parent.id", "id", true)],
            "every entry named the folder that was asked for; their own ids did not"
        );
    }

    #[test]
    fn an_array_whose_elements_disagree_echoes_nothing() {
        // One entry matching the capture says nothing about the position: the
        // others at the same path carried something else.
        let group: Vec<MockConfig> = (7..=10)
            .map(|n| {
                recorded(
                    &format!("/v2/folders/{n}/items"),
                    &format!(r#"{{"entries":[{{"id":"{n}"}},{{"id":"other"}}]}}"#),
                )
            })
            .collect();

        assert!(echoes(&group, "/v2/folders/{id}/items").is_empty());
    }

    #[test]
    fn a_capture_outranks_a_query_parameter_carrying_the_same_value() {
        let group: Vec<MockConfig> = (101..=104)
            .map(|n| {
                recorded(
                    &format!("/v2/files/{n}?id={n}"),
                    &format!(r#"{{"id":"{n}"}}"#),
                )
            })
            .collect();

        assert_eq!(
            echoes(&group, "/v2/files/{id}"),
            vec![capture("id", "id", true)],
            "the path is the most direct statement of what was asked for"
        );
    }

    #[test]
    fn a_field_that_wraps_the_id_echoes_it_inside_the_wrapper() {
        // Some APIs write a folder's typed id as `d_<id>`. Answering it with a value
        // of its own contradicts the request as plainly as getting the id wrong.
        let group: Vec<MockConfig> = (9_848_115_997_u64..=9_848_116_000)
            .map(|n| {
                recorded(
                    &format!("/v2/folders/{n}"),
                    &format!(r#"{{"id":"{n}","typedId":"d_{n}"}}"#),
                )
            })
            .collect();

        let found = echoes(&group, "/v2/folders/{id}");
        let typed = found
            .iter()
            .find(|(path, ..)| path == "typedId")
            .expect("the typed id repeats the folder that was asked for");
        assert_eq!((typed.1, typed.2.as_str()), (EchoSource::Capture, "id"));

        let template = crate::codegen::EchoedField {
            source: typed.1,
            name: typed.2.clone(),
            quoted: typed.3,
            prefix: "d_".to_string(),
            suffix: String::new(),
        }
        .expression();
        assert_eq!(template, "\"d_{{ captures.id }}\"");
    }

    #[test]
    fn a_wrapper_is_not_read_around_a_value_short_enough_to_appear_by_chance() {
        // `7` turns up inside half the strings an API returns.
        let group: Vec<MockConfig> = (7..=10)
            .map(|n| {
                recorded(
                    &format!("/v2/folders/{n}"),
                    &format!(r#"{{"label":"item {n} of 12"}}"#),
                )
            })
            .collect();

        assert!(echoes(&group, "/v2/folders/{id}").is_empty());
    }

    #[test]
    fn a_repeated_query_parameter_names_no_single_value() {
        let group: Vec<MockConfig> = (1..=3)
            .map(|n| {
                recorded(
                    &format!("/v2/batch?ids=a{n}&ids=b{n}"),
                    &format!(r#"{{"first":"a{n}"}}"#),
                )
            })
            .collect();

        assert!(
            echoes(&group, "/v2/batch").is_empty(),
            "`ids` carried two values; a template reading it back would get one of them"
        );
    }
}

/// Reading a lone recording for what its values are, rather than reproducing it.
mod generalizing {
    use super::*;

    fn generalized(mocks: Vec<MockConfig>) -> MockCollectionConfig {
        let collection = MockCollectionConfig {
            name: None,
            description: None,
            enabled: true,
            vars: None,
            mocks,
            world: None,
            machines: None,
        };
        MockConsolidator::with_options(ConsolidatorOptions {
            generalize: true,
            ..ConsolidatorOptions::default()
        })
        .consolidate(collection)
        .expect("consolidates")
    }

    fn template_of(collection: &MockCollectionConfig) -> String {
        collection
            .mocks
            .first()
            .and_then(|mock| mock.response_config.as_ref())
            .and_then(|response| response.template())
            .map(ToString::to_string)
            .unwrap_or_default()
    }

    fn url_of(collection: &MockCollectionConfig) -> String {
        collection
            .mocks
            .first()
            .and_then(|mock| mock.match_config.as_ref())
            .and_then(|match_config| match_config.urls.first().or(match_config.url.as_ref()))
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn the_shape_the_recording_usually_had_is_the_shape_it_answers_with() {
        // A field null in most samples is answered null, and one most samples
        // did not carry is answered without it. Both minimise the divergence a
        // client sees, because most of what it checks against is the common
        // shape.
        let group = vec![
            create_test_mock(
                "a",
                "GET",
                "exact:/v2/items/1",
                r#"{"thumb":null,"rare":1}"#,
            ),
            create_test_mock("b", "GET", "exact:/v2/items/2", r#"{"thumb":null}"#),
            create_test_mock("c", "GET", "exact:/v2/items/3", r#"{"thumb":"u"}"#),
        ];
        let analysis = crate::consolidator::analysis::ResponseAnalyzer::new(false)
            .analyze_response_patterns(&group, "/v2/items/{id}")
            .expect("analyses");

        assert!(
            analysis
                .constant_fields
                .iter()
                .any(|(field, value)| field == "thumb" && value.is_null()),
            "two of three recordings had nothing there: {:?}",
            analysis.constant_fields
        );
        assert!(
            !analysis
                .varying_fields
                .iter()
                .any(|(field, _)| field == "rare")
                && !analysis
                    .constant_fields
                    .iter()
                    .any(|(field, _)| field == "rare"),
            "a field one recording in three carried is not part of the answer"
        );
    }

    #[test]
    fn one_recording_becomes_a_template_of_what_its_values_are() {
        // Left alone, a lone recording is reproduced exactly: every field agrees
        // with itself and so reads as fixed.
        let consolidated = generalized(vec![create_test_mock(
            "file",
            "GET",
            "exact:/v2/files/27977065362",
            r#"{"id":"27977065362","name":"Report","created_at":"2024-04-16T09:25:57Z","type":"file"}"#,
        )]);

        let template = template_of(&consolidated);
        assert!(
            template.contains("fake_timestamp"),
            "a timestamp is a timestamp whether or not it was seen twice: {template}"
        );
        assert!(
            template.contains(r#""type": "file""#),
            "a value the detector cannot place stays as it was recorded: {template}"
        );
        assert_eq!(
            url_of(&consolidated),
            "/v2/files/{id}",
            "the mock has to answer the family of requests, not the one it saw"
        );
        assert!(
            template.contains("captures.id"),
            "the id the URL names is the id the answer carries: {template}"
        );
    }

    #[test]
    fn a_cache_buster_is_not_something_to_wait_for() {
        // The app regenerates `_` on every load, so a mock pinned to the
        // recorded one answers the recording and nothing afterwards.
        let consolidated = generalized(vec![create_test_mock(
            "notes",
            "GET",
            "exact:/inbox_notes?limit=30&_=1786715224166",
            r#"{"count":3}"#,
        )]);

        let mock = consolidated.mocks.first().expect("one mock");
        let match_config = mock.match_config.as_ref().expect("a matcher");
        assert_eq!(url_of(&consolidated), "/inbox_notes");
        assert_eq!(
            match_config.query.get("limit").map(String::as_str),
            Some("30"),
            "the parameter that narrows the search still has to hold"
        );
        assert!(
            !match_config.query.contains_key("_"),
            "the cache buster names the moment, not the request: {:?}",
            match_config.query
        );
    }

    #[test]
    fn a_query_worth_keeping_is_left_exactly_as_it_was() {
        // Moving a query out of the URL trades an exact match for a subset one.
        // With nothing to throw away there is nothing to pay for that.
        let consolidated = generalized(vec![create_test_mock(
            "search",
            "GET",
            "exact:/v2/search?folder_id=7001&limit=20",
            r#"{"found":2}"#,
        )]);

        assert_eq!(url_of(&consolidated), "/v2/search?folder_id=7001&limit=20");
    }

    #[test]
    fn a_short_number_in_a_path_is_not_an_identifier() {
        // An API version and a page number are as numeric as an id. Widening
        // them would let one mock claim every endpoint under the prefix.
        let consolidated = generalized(vec![create_test_mock(
            "versioned",
            "GET",
            "exact:/api/2/status",
            r#"{"uptime":"2024-04-16T09:25:57Z"}"#,
        )]);

        assert_eq!(url_of(&consolidated), "/api/2/status");
    }

    #[test]
    fn a_lone_recording_still_reproduces_what_it_cannot_generalize() {
        let consolidated = generalized(vec![create_test_mock(
            "flags",
            "GET",
            "exact:/v2/flags",
            r#"{"enabled":true,"tier":"premium","note":null,"items":[]}"#,
        )]);

        let answer = consolidated
            .mocks
            .first()
            .and_then(|mock| mock.response_config.as_ref())
            .and_then(|response| response.template().or_else(|| response.body()))
            .map(ToString::to_string)
            .unwrap_or_default();

        for kept in [
            r#""enabled": true"#,
            r#""tier": "premium""#,
            r#""note": null"#,
        ] {
            assert!(
                answer.contains(kept) || answer.contains(&kept.replace(": ", ":")),
                "{kept} is not something to invent, and was lost: {answer}"
            );
        }
    }
}
