#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::string_slice
)]
//! Comprehensive tests for mock consolidation engine
//!
//! Tests cover:
//! - Pattern detection (pagination, IDs, search queries)
//! - Response deduplication
//! - Template extraction
//! - Sequence generation
//! - End-to-end consolidation workflows

use mockpit_config::{MockCollectionConfig, MockConfig};
use mockpit_consolidator::{ConsolidatorOptions, MockConsolidator};
use rustc_hash::FxHashMap;

/// Helper to create a test mock with given URL
fn create_mock(id: &str, method: &str, url: &str, status: u16, body: &str) -> MockConfig {
    use mockpit_config::{MatchConfig, ReturnConfig};

    MockConfig {
        id: id.into(),
        description: None,
        priority: 100,
        enabled: true,
        scope: None,
        match_config: Some(MatchConfig {
            method: None,
            methods: vec![method.to_string()],
            url: Some(format!("exact:{url}")),
            urls: vec![],
            headers: FxHashMap::default(),
            query: FxHashMap::default(),
            graphql: None,
            body: FxHashMap::default(),
        }),
        request: None,
        vars: None,
        response_config: Some(ReturnConfig::Structured {
            status: Some(status),
            headers: FxHashMap::default(),
            body: Some(body.to_string()),
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
async fn test_pagination_pattern_detection() {
    println!("\n=== TEST: Pagination Pattern Detection ===");

    let mocks = vec![
        create_mock(
            "1",
            "GET",
            "/api/users?page=1",
            200,
            r#"{"users":[{"id":1}]}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/users?page=2",
            200,
            r#"{"users":[{"id":2}]}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/users?page=3",
            200,
            r#"{"users":[{"id":3}]}"#,
        ),
        create_mock(
            "4",
            "GET",
            "/api/users?page=4",
            200,
            r#"{"users":[{"id":4}]}"#,
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
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 4");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!(
        "Patterns detected: {}",
        consolidator.stats().patterns_detected
    );

    // Should consolidate pagination into fewer mocks
    assert!(
        result.mocks.len() < 4,
        "Should consolidate pagination patterns"
    );
    assert!(
        consolidator.stats().patterns_detected > 0,
        "Should detect pagination pattern"
    );

    // Check that URL pattern uses prefix or template
    let consolidated_mock = &result.mocks[0];
    let match_config = consolidated_mock.match_config.as_ref().unwrap();
    let response_config = consolidated_mock.response_config.as_ref().unwrap();

    let has_prefix_pattern = match_config
        .url
        .as_ref()
        .is_some_and(|u| u.contains("prefix:"))
        || match_config.urls.iter().any(|u| u.contains("prefix:"));

    let is_template = response_config.template().is_some();

    assert!(
        has_prefix_pattern || is_template,
        "Should use prefix pattern or template for pagination"
    );

    println!("✅ Pagination pattern detected and consolidated");
}

#[tokio::test]
async fn test_id_based_pattern_detection() {
    println!("\n=== TEST: ID-Based Pattern Detection ===");

    let mocks = vec![
        create_mock(
            "1",
            "GET",
            "/api/users/123",
            200,
            r#"{"id":123,"name":"Alice"}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/users/456",
            200,
            r#"{"id":456,"name":"Bob"}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/users/789",
            200,
            r#"{"id":789,"name":"Charlie"}"#,
        ),
        create_mock(
            "4",
            "GET",
            "/api/users/999",
            200,
            r#"{"id":999,"name":"David"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("ID Pattern Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 4");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!(
        "ID patterns detected: {}",
        consolidator.stats().patterns_detected
    );

    // Should consolidate ID-based patterns
    assert!(result.mocks.len() < 4, "Should consolidate ID patterns");
    assert!(
        consolidator.stats().patterns_detected > 0,
        "Should detect ID pattern"
    );

    // Check that URL pattern uses regex or template
    let consolidated_mock = &result.mocks[0];
    let match_config = consolidated_mock.match_config.as_ref().unwrap();
    let response_config = consolidated_mock.response_config.as_ref().unwrap();

    let has_regex_pattern = match_config
        .url
        .as_ref()
        .is_some_and(|u| u.contains("regex:"))
        || match_config.urls.iter().any(|u| u.contains("regex:"));

    let is_template = response_config.template().is_some();

    assert!(
        has_regex_pattern || is_template,
        "Should use regex pattern or template for ID-based paths"
    );

    println!("✅ ID-based pattern detected and consolidated");
}

#[tokio::test]
async fn test_uuid_pattern_detection() {
    println!("\n=== TEST: UUID Pattern Detection ===");

    let mocks = vec![
        create_mock(
            "1",
            "GET",
            "/api/files/550e8400-e29b-41d4-a716-446655440000",
            200,
            r#"{"file":"doc1.pdf"}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/files/6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            200,
            r#"{"file":"doc2.pdf"}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/files/7c9e6679-7425-40de-944b-e07fc1f90ae7",
            200,
            r#"{"file":"doc3.pdf"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("UUID Pattern Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 3");
    println!("Consolidated mocks: {}", result.mocks.len());

    // Should consolidate UUID patterns
    assert!(result.mocks.len() < 3, "Should consolidate UUID patterns");
    assert!(
        consolidator.stats().patterns_detected > 0,
        "Should detect UUID pattern"
    );

    println!("✅ UUID pattern detected and consolidated");
}

#[tokio::test]
async fn test_search_query_pattern() {
    println!("\n=== TEST: Search Query Pattern Detection ===");

    let mocks = vec![
        create_mock(
            "1",
            "GET",
            "/api/search?q=foo",
            200,
            r#"{"results":["foo1","foo2"]}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/search?q=bar",
            200,
            r#"{"results":["bar1","bar2"]}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/search?q=baz",
            200,
            r#"{"results":["baz1","baz2"]}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Search Query Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 3");
    println!("Consolidated mocks: {}", result.mocks.len());

    // Should consolidate search queries
    assert!(
        result.mocks.len() < 3,
        "Should consolidate search query patterns"
    );

    println!("✅ Search query pattern detected and consolidated");
}

#[tokio::test]
async fn test_duplicate_removal() {
    println!("\n=== TEST: Duplicate Request Removal ===");

    let mocks = vec![
        create_mock("1", "GET", "/api/status", 200, r#"{"status":"ok"}"#),
        create_mock("2", "GET", "/api/status", 200, r#"{"status":"ok"}"#),
        create_mock("3", "GET", "/api/status", 200, r#"{"status":"ok"}"#),
        create_mock("4", "GET", "/api/status", 200, r#"{"status":"ok"}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Duplicate Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 4");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!(
        "Duplicates removed: {}",
        consolidator.stats().duplicates_removed
    );

    // Should remove all duplicates, keeping only one
    assert_eq!(
        result.mocks.len(),
        1,
        "Should keep only one mock after deduplication"
    );
    assert!(
        consolidator.stats().duplicates_removed >= 3,
        "Should remove 3 duplicates"
    );

    println!("✅ Duplicates removed successfully");
}

#[tokio::test]
async fn test_variable_response_template() {
    println!("\n=== TEST: Variable Response Template Generation ===");

    let mocks = vec![
        create_mock("1", "GET", "/api/random", 200, r#"{"value":1}"#),
        create_mock("2", "GET", "/api/random", 200, r#"{"value":2}"#),
        create_mock("3", "GET", "/api/random", 200, r#"{"value":3}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Variable Response Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 3");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!(
        "Templates created: {}",
        consolidator.stats().templates_created
    );

    // Should create a template for variable responses
    assert!(result.mocks.len() <= 1, "Should consolidate into template");
    assert!(
        consolidator.stats().templates_created > 0,
        "Should create template"
    );

    // Verify template was created
    if !result.mocks.is_empty() {
        let mock = &result.mocks[0];
        let response_config = mock.response_config.as_ref().unwrap();
        let is_template = response_config.template().is_some();
        assert!(is_template, "Should have template in response");
    }

    println!("✅ Variable response template created");
}

#[tokio::test]
async fn test_mixed_patterns() {
    println!("\n=== TEST: Mixed Pattern Consolidation ===");

    let mocks = vec![
        // Pagination group
        create_mock("p1", "GET", "/api/users?page=1", 200, r#"{"page":1}"#),
        create_mock("p2", "GET", "/api/users?page=2", 200, r#"{"page":2}"#),
        create_mock("p3", "GET", "/api/users?page=3", 200, r#"{"page":3}"#),
        // ID-based group
        create_mock("id1", "GET", "/api/posts/123", 200, r#"{"id":123}"#),
        create_mock("id2", "GET", "/api/posts/456", 200, r#"{"id":456}"#),
        create_mock("id3", "GET", "/api/posts/789", 200, r#"{"id":789}"#),
        // Unique requests (no pattern)
        create_mock("u1", "GET", "/api/unique/endpoint1", 200, r#"{"data":"a"}"#),
        create_mock(
            "u2",
            "POST",
            "/api/unique/endpoint2",
            201,
            r#"{"data":"b"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Mixed Patterns Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 8");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!(
        "Patterns detected: {}",
        consolidator.stats().patterns_detected
    );

    // Should consolidate pagination + ID patterns, keep unique ones
    assert!(result.mocks.len() < 8, "Should consolidate patterns");
    assert!(
        result.mocks.len() >= 4,
        "Should keep unique requests separate"
    );
    assert!(
        consolidator.stats().patterns_detected >= 2,
        "Should detect multiple patterns"
    );

    println!("✅ Mixed patterns handled correctly");
}

#[tokio::test]
async fn test_no_consolidation_for_single_mock() {
    println!("\n=== TEST: No Consolidation for Single Mocks ===");

    let mocks = vec![create_mock(
        "1",
        "GET",
        "/api/unique",
        200,
        r#"{"status":"ok"}"#,
    )];

    let collection = MockCollectionConfig {
        name: Some("Single Mock Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 1");
    println!("Consolidated mocks: {}", result.mocks.len());

    // Should not modify single mocks
    assert_eq!(result.mocks.len(), 1, "Should keep single mock unchanged");
    assert_eq!(
        consolidator.stats().patterns_detected,
        0,
        "Should not detect patterns"
    );

    println!("✅ Single mock preserved unchanged");
}

#[tokio::test]
async fn test_min_pattern_threshold() {
    println!("\n=== TEST: Minimum Pattern Threshold ===");

    let mocks = vec![
        create_mock("1", "GET", "/api/users?page=1", 200, r#"{"page":1}"#),
        create_mock("2", "GET", "/api/users?page=2", 200, r#"{"page":2}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Threshold Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    // With threshold of 3, should NOT consolidate (only 2 mocks)
    let options = ConsolidatorOptions {
        enable_consolidation: true,
        enable_templates: true,
        min_pattern_threshold: 3,
        enable_stateful_pagination: true,
        pagination_storage_key_template: "api.{path}.total".to_string(),
    };

    let mut consolidator = MockConsolidator::with_options(options);
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 2");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!("Min threshold: 3");

    // Should keep both mocks because threshold is not met
    assert_eq!(
        result.mocks.len(),
        2,
        "Should not consolidate below threshold"
    );

    println!("✅ Minimum threshold respected");
}

#[tokio::test]
async fn test_disable_consolidation() {
    println!("\n=== TEST: Disable Consolidation ===");

    let mocks = vec![
        create_mock("1", "GET", "/api/users?page=1", 200, r#"{"page":1}"#),
        create_mock("2", "GET", "/api/users?page=2", 200, r#"{"page":2}"#),
        create_mock("3", "GET", "/api/users?page=3", 200, r#"{"page":3}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Disabled Consolidation Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let options = ConsolidatorOptions {
        enable_consolidation: false, // Disabled
        enable_templates: true,
        min_pattern_threshold: 3,
        enable_stateful_pagination: true,
        pagination_storage_key_template: "api.{path}.total".to_string(),
    };

    let mut consolidator = MockConsolidator::with_options(options);
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 3");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!("Consolidation: disabled");

    // Should not consolidate when disabled (but deduplication still happens)
    assert_eq!(
        result.mocks.len(),
        3,
        "Should not consolidate when disabled"
    );

    println!("✅ Consolidation successfully disabled");
}

#[tokio::test]
async fn test_consolidation_stats() {
    println!("\n=== TEST: Consolidation Statistics ===");

    let mocks = vec![
        // Pagination (will be consolidated)
        create_mock("p1", "GET", "/api/users?page=1", 200, r#"{"page":1}"#),
        create_mock("p2", "GET", "/api/users?page=2", 200, r#"{"page":2}"#),
        create_mock("p3", "GET", "/api/users?page=3", 200, r#"{"page":3}"#),
        // Duplicates (will be removed)
        create_mock("d1", "GET", "/api/status", 200, r#"{"ok":true}"#),
        create_mock("d2", "GET", "/api/status", 200, r#"{"ok":true}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Stats Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let _result = consolidator.consolidate(collection).unwrap();

    let stats = consolidator.stats();

    println!("Original count: {}", stats.original_count);
    println!("Consolidated count: {}", stats.consolidated_count);
    println!("Reduction ratio: {:.1}%", stats.reduction_ratio * 100.0);
    println!("Patterns detected: {}", stats.patterns_detected);
    println!("Duplicates removed: {}", stats.duplicates_removed);

    // Verify stats
    assert_eq!(stats.original_count, 5, "Should track original count");
    assert!(stats.consolidated_count < 5, "Should reduce mock count");
    assert!(stats.reduction_ratio > 0.0, "Should have reduction ratio");
    assert!(stats.patterns_detected > 0, "Should detect patterns");

    println!("✅ Statistics tracked correctly");
}

#[tokio::test]
async fn test_complex_real_world_scenario() {
    println!("\n=== TEST: Complex Real-World Scenario ===");

    let mut mocks = vec![];

    // Simulate infinite scroll pagination (20 pages)
    for i in 1..=20 {
        mocks.push(create_mock(
            &format!("page-{i}"),
            "GET",
            &format!("/api/feed?offset={}&limit=10", (i - 1) * 10),
            200,
            &format!(r#"{{"items":[{{"id":{i}}}],"hasMore":true}}"#),
        ));
    }

    // Simulate file downloads (15 different files)
    for i in 1..=15 {
        mocks.push(create_mock(
            &format!("file-{i}"),
            "GET",
            &format!("/api/files/{}/download", 100 + i),
            200,
            r"<binary data>",
        ));
    }

    // Simulate search queries (10 different searches)
    for term in [
        "rust",
        "python",
        "java",
        "go",
        "typescript",
        "swift",
        "kotlin",
        "scala",
        "ruby",
        "perl",
    ] {
        mocks.push(create_mock(
            &format!("search-{term}"),
            "GET",
            &format!("/api/search?q={term}&type=code"),
            200,
            &format!(r#"{{"query":"{term}","results":[]}}"#),
        ));
    }

    // Add some duplicates (5 health checks)
    for i in 1..=5 {
        mocks.push(create_mock(
            &format!("health-{i}"),
            "GET",
            "/api/health",
            200,
            r#"{"status":"healthy"}"#,
        ));
    }

    let original_count = mocks.len();
    println!("Simulating real-world recording with {original_count} mocks:");
    println!("  - 20 pagination requests");
    println!("  - 15 file downloads");
    println!("  - 10 search queries");
    println!("  - 5 duplicate health checks");

    let collection = MockCollectionConfig {
        name: Some("Real World Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    let stats = consolidator.stats();

    println!("\nConsolidation Results:");
    println!("  Original: {} mocks", stats.original_count);
    println!("  Consolidated: {} mocks", stats.consolidated_count);
    println!("  Reduction: {:.1}%", stats.reduction_ratio * 100.0);
    println!("  Patterns detected: {}", stats.patterns_detected);
    println!("  Duplicates removed: {}", stats.duplicates_removed);
    println!("  Templates created: {}", stats.templates_created);

    // Verify dramatic reduction
    assert_eq!(stats.original_count, original_count);
    assert!(
        stats.consolidated_count < original_count / 2,
        "Should reduce by at least 50%"
    );
    assert!(stats.reduction_ratio > 0.5, "Should have >50% reduction");
    assert!(
        stats.patterns_detected >= 3,
        "Should detect multiple patterns"
    );
    assert!(
        stats.duplicates_removed >= 4,
        "Should remove health check duplicates"
    );

    println!("\n✅ Real-world scenario consolidated successfully");
    println!(
        "   {} mocks → {} mocks ({:.1}% reduction)",
        original_count,
        result.mocks.len(),
        stats.reduction_ratio * 100.0
    );
}

#[tokio::test]
async fn test_preserves_different_methods() {
    println!("\n=== TEST: Preserves Different HTTP Methods ===");

    let mocks = vec![
        create_mock("1", "GET", "/api/users/123", 200, r#"{"id":123}"#),
        create_mock("2", "POST", "/api/users/123", 201, r#"{"created":true}"#),
        create_mock("3", "PUT", "/api/users/123", 200, r#"{"updated":true}"#),
        create_mock("4", "DELETE", "/api/users/123", 204, ""),
    ];

    let collection = MockCollectionConfig {
        name: Some("Methods Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 4 (different methods, same path)");
    println!("Consolidated mocks: {}", result.mocks.len());

    // Should keep different methods separate
    assert_eq!(
        result.mocks.len(),
        4,
        "Should not consolidate different HTTP methods"
    );

    println!("✅ Different HTTP methods preserved");
}

#[tokio::test]
async fn test_path_normalization_via_grouping() {
    println!("\n=== TEST: Path Normalization Via Grouping ===");

    // Test that numeric IDs get grouped together
    let mocks = vec![
        create_mock("1", "GET", "/users/123", 200, r#"{"id":123}"#),
        create_mock("2", "GET", "/users/456", 200, r#"{"id":456}"#),
        create_mock("3", "GET", "/users/789", 200, r#"{"id":789}"#),
    ];

    let collection = MockCollectionConfig {
        name: Some("Path Normalization Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    // If path normalization works, these should be grouped
    assert!(
        result.mocks.len() < 3,
        "Should group similar paths together"
    );

    println!("✅ Path normalization groups similar paths correctly");
}

#[tokio::test]
async fn test_cursor_based_pagination() {
    println!("\n=== TEST: Cursor-Based Pagination ===");

    let mocks = vec![
        create_mock(
            "1",
            "GET",
            "/api/items?cursor=abc123&limit=10",
            200,
            r#"{"items":[1,2],"nextCursor":"def456"}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/items?cursor=def456&limit=10",
            200,
            r#"{"items":[3,4],"nextCursor":"ghi789"}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/items?cursor=ghi789&limit=10",
            200,
            r#"{"items":[5,6],"nextCursor":null}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Cursor Pagination Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 3");
    println!("Consolidated mocks: {}", result.mocks.len());

    // Should consolidate cursor-based pagination
    assert!(
        result.mocks.len() < 3,
        "Should consolidate cursor pagination"
    );
    assert!(
        consolidator.stats().patterns_detected > 0,
        "Should detect cursor pattern"
    );

    println!("✅ Cursor-based pagination detected and consolidated");
}

#[tokio::test]
async fn test_complex_query_params() {
    println!("\n=== TEST: Complex Query Parameters with Multiple Constant Params ===");

    // Simulates your documents-search example:
    // Only 'page' varies, but there are many constant params
    let mocks = vec![
        create_mock(
            "1",
            "GET",
            "/api/v1/documents-search/?page=1&limit=10&autocomplete=&status=*&rsl=false&batchSend=false",
            200,
            r#"{"docs":[{"id":1}]}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/v1/documents-search/?page=2&limit=10&autocomplete=&status=*&rsl=false&batchSend=false",
            200,
            r#"{"docs":[{"id":2}]}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/v1/documents-search/?page=3&limit=10&autocomplete=&status=*&rsl=false&batchSend=false",
            200,
            r#"{"docs":[{"id":3}]}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: Some("Complex Query Params Test".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 3");
    println!("Consolidated mocks: {}", result.mocks.len());
    println!(
        "Patterns detected: {}",
        consolidator.stats().patterns_detected
    );

    // Should consolidate even with many constant params
    assert!(
        result.mocks.len() < 3,
        "Should consolidate complex query params"
    );
    assert!(
        consolidator.stats().patterns_detected > 0,
        "Should detect pattern"
    );

    println!("✅ Complex query parameters with only some varying detected and consolidated");
}

#[tokio::test]
async fn test_download_url_file_type_detection() {
    println!("\n=== TEST: Download URL File Type Detection ===");

    // Create mocks with download URLs for different file types
    let mocks = vec![
        // PDF documents - using "download" keyword and long URLs
        create_mock(
            "1",
            "GET",
            "/api/documents/1",
            200,
            r#"{"id":1,"name":"Document 1","download_url":"https://storage.example.com/download/files/verylong/path/to/document/12345abcdef67890/report.pdf?token=abc123def456ghi789jkl012mno345pqr678stu901vwx234yz567890abcdef1234567890"}"#,
        ),
        create_mock(
            "2",
            "GET",
            "/api/documents/2",
            200,
            r#"{"id":2,"name":"Document 2","download_url":"https://storage.example.com/download/files/verylong/path/to/document/98765zyxwvu43210/invoice.pdf?token=xyz987wvu654tsr321qpo098nml765kji432hgf109edc876bac543zyxwvu2109876543"}"#,
        ),
        create_mock(
            "3",
            "GET",
            "/api/documents/3",
            200,
            r#"{"id":3,"name":"Document 3","download_url":"https://storage.example.com/download/files/verylong/path/to/document/abcdef1234567890/contract.pdf?token=pqr678stu901vwx234yz567890abcdef1234567890123456789012345678901234567890"}"#,
        ),
        // PNG images - using "content" keyword and long URLs
        create_mock(
            "4",
            "GET",
            "/api/images/1",
            200,
            r#"{"id":1,"name":"Image 1","download_url":"https://cdn.example.com/content/images/very/long/complex/path/to/some/image/file/12345abcdef67890ghij/avatar.png?size=large&quality=high&token=abc123def456ghi789"}"#,
        ),
        create_mock(
            "5",
            "GET",
            "/api/images/2",
            200,
            r#"{"id":2,"name":"Image 2","download_url":"https://cdn.example.com/content/images/very/long/complex/path/to/some/image/file/67890ghijklmno12345/logo.png?size=large&quality=high&token=xyz789abc456def789"}"#,
        ),
        create_mock(
            "6",
            "GET",
            "/api/images/3",
            200,
            r#"{"id":3,"name":"Image 3","download_url":"https://cdn.example.com/content/images/very/long/complex/path/to/some/image/file/mnopqrs123456789/icon.png?size=large&quality=high&token=pqr456stu789vwx012"}"#,
        ),
    ];

    let collection = MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let mut consolidator = MockConsolidator::new();
    let result = consolidator.consolidate(collection).unwrap();

    println!("Original mocks: 6");
    println!("Consolidated mocks: {}", result.mocks.len());

    // Find the consolidated mocks and check their templates
    let mut found_pdf = false;
    let mut found_png = false;

    for mock in &result.mocks {
        if let Some(ref response_config) = mock.response_config
            && let Some(tmpl) = response_config.template()
        {
            println!("\nMock ID: {}", mock.id);
            println!("Body template snippet: {}", &tmpl[..200.min(tmpl.len())]);

            // Check if download URLs are detected and appropriate helpers are used
            if tmpl.contains("download_url") {
                // This is a templated mock
                if tmpl.contains("fake_pdf_data_uri") {
                    println!("  PDF file detected - using fake_pdf_data_uri()");
                    found_pdf = true;
                } else if tmpl.contains("fake_png_data_uri") {
                    println!("  PNG file detected - using fake_png_data_uri()");
                    found_png = true;
                } else if tmpl.contains("fake_jpeg_data_uri") {
                    println!("  JPEG file detected - using fake_jpeg_data_uri()");
                }
            }
        }
    }

    // We should have consolidated and detected at least some file types
    assert!(
        found_pdf || found_png,
        "Should detect and use data URI helpers for file types"
    );

    println!("\n✅ Download URL file type detection working correctly");
}
