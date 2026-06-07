//! End-to-end tests for scope-based test isolation
//!
//! These tests verify that scopes work correctly for:
//! - Creating isolated test environments
//! - Automatic cleanup via scope deletion
//! - TTL-based expiration
//! - Scope filtering and queries
//! - Integration with existing mock functionality

use http::{Method, StatusCode};
use lean_string::LeanString;
use mockpit::engine::{
    BodySource, MockDefinition, MockRegistry, RequestMatcher, ResponseGenerator, UrlPattern,
};
use smallvec::smallvec;
use std::time::Duration;

#[test]
fn test_scope_isolation_between_tests() {
    println!("\n=== Testing Scope Isolation ===");

    let registry = MockRegistry::new();

    // Test Suite 1: Create scope and add mocks
    println!("Test Suite 1: Creating scope and mocks");
    registry
        .create_scope("test-suite-1".into(), None)
        .expect("Failed to create scope");

    let mock1 = MockDefinition {
        id: "suite1-login".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("test-suite-1".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/api/login")],
            ..Default::default()
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"token":"suite1-token"}"#),
        ),
        vars: None,
    };

    let mock2 = MockDefinition {
        id: "suite1-user".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("test-suite-1".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/user")],
            ..Default::default()
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"id":"user1"}"#)),
        vars: None,
    };

    registry.add_mock(mock1);
    registry.add_mock(mock2);

    // Test Suite 2: Create separate scope with different mocks
    println!("Test Suite 2: Creating separate scope");
    registry
        .create_scope("test-suite-2".into(), None)
        .expect("Failed to create scope");

    let mock3 = MockDefinition {
        id: "suite2-login".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("test-suite-2".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/api/login")],
            ..Default::default()
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"token":"suite2-token"}"#),
        ),
        vars: None,
    };

    registry.add_mock(mock3);

    // Global mock (no scope)
    let global_mock = MockDefinition {
        id: "global-health".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/health")],
            ..Default::default()
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"status":"ok"}"#)),
        vars: None,
    };

    registry.add_mock(global_mock);

    // Verify isolation
    println!("Verifying scope isolation...");
    assert_eq!(registry.len(), 4, "Should have 4 total mocks");

    let suite1_mocks = registry.get_mocks_by_scope("test-suite-1");
    assert_eq!(suite1_mocks.len(), 2, "Suite 1 should have 2 mocks");
    assert!(suite1_mocks.iter().any(|m| m.id == "suite1-login"));
    assert!(suite1_mocks.iter().any(|m| m.id == "suite1-user"));

    let suite2_mocks = registry.get_mocks_by_scope("test-suite-2");
    assert_eq!(suite2_mocks.len(), 1, "Suite 2 should have 1 mock");
    assert!(suite2_mocks.iter().any(|m| m.id == "suite2-login"));

    // Verify global mock is not in any scope
    assert!(registry.get_mock("global-health").unwrap().scope.is_none());

    // Delete suite 1, verify suite 2 and global remain
    println!("Deleting test-suite-1...");
    let deleted = registry
        .delete_scope("test-suite-1")
        .expect("Failed to delete scope");
    assert_eq!(deleted, 2, "Should delete 2 mocks from suite 1");
    assert_eq!(registry.len(), 2, "Should have 2 mocks remaining");

    assert!(registry.get_mock("suite1-login").is_none());
    assert!(registry.get_mock("suite1-user").is_none());
    assert!(registry.get_mock("suite2-login").is_some());
    assert!(registry.get_mock("global-health").is_some());

    // Delete suite 2
    println!("Deleting test-suite-2...");
    registry
        .delete_scope("test-suite-2")
        .expect("Failed to delete scope");
    assert_eq!(registry.len(), 1, "Only global mock should remain");
    assert!(registry.get_mock("global-health").is_some());

    println!("✓ Scope isolation test passed");
}

#[test]
fn test_scope_ttl_expiration() {
    println!("\n=== Testing TTL-based Expiration ===");

    let registry = MockRegistry::new();

    // Create scope with very short TTL
    println!("Creating scope with 10ms TTL");
    registry
        .create_scope("short-lived".into(), Some(Duration::from_millis(10)))
        .expect("Failed to create scope");

    let mock = MockDefinition {
        id: "expiring-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("short-lived".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/temp")],
            ..Default::default()
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"temp":true}"#)),
        vars: None,
    };

    registry.add_mock(mock);

    // Create scope with no TTL
    registry
        .create_scope("permanent".into(), None)
        .expect("Failed to create scope");

    let permanent_mock = MockDefinition {
        id: "permanent-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("permanent".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/permanent")],
            ..Default::default()
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"permanent":true}"#),
        ),
        vars: None,
    };

    registry.add_mock(permanent_mock);

    assert_eq!(registry.len(), 2);
    assert!(registry.scope_exists("short-lived"));
    assert!(registry.scope_exists("permanent"));

    // Wait for expiration
    println!("Waiting for TTL expiration...");
    std::thread::sleep(Duration::from_millis(50));

    // Cleanup expired scopes
    let cleaned = registry.cleanup_expired_scopes();
    assert_eq!(cleaned, 1, "Should clean up 1 expired scope");

    // Verify expired scope and its mock are gone
    assert!(!registry.scope_exists("short-lived"));
    assert!(registry.get_mock("expiring-mock").is_none());

    // Verify permanent scope remains
    assert!(registry.scope_exists("permanent"));
    assert!(registry.get_mock("permanent-mock").is_some());
    assert_eq!(registry.len(), 1);

    println!("✓ TTL expiration test passed");
}

#[test]
fn test_scope_info_queries() {
    println!("\n=== Testing Scope Info Queries ===");

    let registry = MockRegistry::new();

    // Create multiple scopes
    registry
        .create_scope("api-tests".into(), Some(Duration::from_hours(1)))
        .unwrap();
    registry.create_scope("ui-tests".into(), None).unwrap();
    registry
        .create_scope("integration-tests".into(), Some(Duration::from_hours(2)))
        .unwrap();

    // Add mocks to different scopes
    for i in 0..3 {
        let mock = MockDefinition {
            id: format!("api-mock-{i}").into(),
            priority: 100,
            enabled: true,
            once: false,
            source_file: None,
            scope: Some("api-tests".into()),
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact(format!("/api/endpoint{i}"))],
                ..Default::default()
            },
            response: ResponseGenerator::new(
                StatusCode::OK,
                BodySource::inline(r#"{"test":"api"}"#),
            ),
            vars: None,
        };
        registry.add_mock(mock);
    }

    for i in 0..2 {
        let mock = MockDefinition {
            id: format!("ui-mock-{i}").into(),
            priority: 100,
            enabled: true,
            once: false,
            source_file: None,
            scope: Some("ui-tests".into()),
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact(format!("/ui/page{i}"))],
                ..Default::default()
            },
            response: ResponseGenerator::new(
                StatusCode::OK,
                BodySource::inline(r#"{"test":"ui"}"#),
            ),
            vars: None,
        };
        registry.add_mock(mock);
    }

    // Test list_scopes
    let scopes = registry.list_scopes();
    assert_eq!(scopes.len(), 3);
    assert!(scopes.contains(&LeanString::from("api-tests")));
    assert!(scopes.contains(&LeanString::from("ui-tests")));
    assert!(scopes.contains(&LeanString::from("integration-tests")));

    // Test get_scope_info
    let api_info = registry
        .get_scope_info("api-tests")
        .expect("Should get api-tests info");
    assert_eq!(api_info.id, "api-tests");
    assert_eq!(api_info.mock_count, 3);
    assert!(api_info.expires_at.is_some());

    let ui_info = registry
        .get_scope_info("ui-tests")
        .expect("Should get ui-tests info");
    assert_eq!(ui_info.id, "ui-tests");
    assert_eq!(ui_info.mock_count, 2);
    assert!(ui_info.expires_at.is_none()); // No TTL

    let integration_info = registry
        .get_scope_info("integration-tests")
        .expect("Should get integration-tests info");
    assert_eq!(integration_info.mock_count, 0); // No mocks added

    // Test non-existent scope
    assert!(registry.get_scope_info("non-existent").is_none());

    println!("✓ Scope info queries test passed");
}

#[test]
fn test_scope_with_duplicate_mock_ids() {
    println!("\n=== Testing Scopes with Duplicate Mock IDs ===");

    let registry = MockRegistry::new();

    // Create two scopes
    registry.create_scope("scope-a".into(), None).unwrap();
    registry.create_scope("scope-b".into(), None).unwrap();

    // Try to add mocks with same ID in different scopes
    let mock_a = MockDefinition {
        id: "login-endpoint".into(), // Same ID
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("scope-a".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/api/login")],
            ..Default::default()
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"scope":"a"}"#)),
        vars: None,
    };

    registry.add_mock(mock_a);

    // Note: Currently mock IDs are global, not scoped
    // So adding another mock with same ID will overwrite the first one
    // This is expected behavior - mock IDs must be globally unique
    let mock_b = MockDefinition {
        id: "login-endpoint".into(), // Same ID - will overwrite
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("scope-b".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::exact("/api/login")],
            ..Default::default()
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"scope":"b"}"#)),
        vars: None,
    };

    registry.add_mock(mock_b);

    // Verify the second mock overwrote the first
    assert_eq!(registry.len(), 1);
    let mock = registry.get_mock("login-endpoint").unwrap();
    assert_eq!(mock.scope.as_deref(), Some("scope-b"));

    println!("✓ Duplicate ID test passed (mock IDs are globally unique)");
}

#[test]
fn test_scope_with_priority_matching() {
    println!("\n=== Testing Scope Integration with Priority Matching ===");

    let registry = MockRegistry::new();

    // Create scope
    registry.create_scope("priority-test".into(), None).unwrap();

    // Add low priority scoped mock
    let low_priority = MockDefinition {
        id: "low-priority".into(),
        priority: 50,
        enabled: true,
        once: false,
        source_file: None,
        scope: Some("priority-test".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            ..Default::default()
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority":"low","scope":"yes"}"#),
        ),
        vars: None,
    };

    // Add high priority global mock
    let high_priority = MockDefinition {
        id: "high-priority".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None, // Global
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::prefix("/api/")],
            ..Default::default()
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"priority":"high","scope":"no"}"#),
        ),
        vars: None,
    };

    registry.add_mock(low_priority);
    registry.add_mock(high_priority);

    // Get all enabled mocks (should be sorted by priority)
    let mocks = registry.get_enabled_mocks();
    assert_eq!(mocks.len(), 2);
    assert_eq!(mocks[0].id, "high-priority"); // Higher priority first
    assert_eq!(mocks[1].id, "low-priority");

    // Delete scope, verify only global remains
    registry.delete_scope("priority-test").unwrap();
    let mocks = registry.get_enabled_mocks();
    assert_eq!(mocks.len(), 1);
    assert_eq!(mocks[0].id, "high-priority");

    println!("✓ Priority matching integration test passed");
}

#[test]
fn test_realistic_test_suite_workflow() {
    println!("\n=== Testing Realistic Test Suite Workflow ===");

    let registry = MockRegistry::new();

    // Simulate a test suite that needs isolated mocking
    println!("Simulating test suite setup...");

    // Test 1: User login flow
    {
        let scope_id = "test-user-login";
        registry
            .create_scope(LeanString::from(scope_id), Some(Duration::from_mins(1)))
            .unwrap();

        // Setup mocks for login test
        registry.add_mock(MockDefinition {
            id: "login-success".into(),
            priority: 100,
            enabled: true,
            once: false,
            source_file: None,
            scope: Some(scope_id.into()),
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::POST],
                url_patterns: smallvec![UrlPattern::exact("/api/login")],
                ..Default::default()
            },
            response: ResponseGenerator::new(
                StatusCode::OK,
                BodySource::inline(r#"{"token":"test-token","user_id":"123"}"#),
            ),
            vars: None,
        });

        registry.add_mock(MockDefinition {
            id: "get-user-profile".into(),
            priority: 100,
            enabled: true,
            once: false,
            source_file: None,
            scope: Some(scope_id.into()),
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact("/api/users/123")],
                ..Default::default()
            },
            response: ResponseGenerator::new(
                StatusCode::OK,
                BodySource::inline(r#"{"id":"123","name":"Test User"}"#),
            ),
            vars: None,
        });

        assert_eq!(registry.get_mocks_by_scope(scope_id).len(), 2);

        // Test runs here...

        // Cleanup after test
        registry.delete_scope(scope_id).unwrap();
        assert_eq!(registry.len(), 0);
    }

    // Test 2: File upload flow (isolated from test 1)
    {
        let scope_id = "test-file-upload";
        registry
            .create_scope(LeanString::from(scope_id), Some(Duration::from_mins(1)))
            .unwrap();

        registry.add_mock(MockDefinition {
            id: "upload-file".into(),
            priority: 100,
            enabled: true,
            once: false,
            source_file: None,
            scope: Some(scope_id.into()),
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::POST],
                url_patterns: smallvec![UrlPattern::exact("/api/files")],
                ..Default::default()
            },
            response: ResponseGenerator::new(
                StatusCode::CREATED,
                BodySource::inline(r#"{"file_id":"file-456"}"#),
            ),
            vars: None,
        });

        assert_eq!(registry.get_mocks_by_scope(scope_id).len(), 1);

        // Test runs here...

        // Cleanup
        registry.delete_scope(scope_id).unwrap();
        assert_eq!(registry.len(), 0);
    }

    // Test 3: Multiple tests running in parallel (different scopes)
    {
        registry
            .create_scope("parallel-test-1".into(), None)
            .unwrap();
        registry
            .create_scope("parallel-test-2".into(), None)
            .unwrap();
        registry
            .create_scope("parallel-test-3".into(), None)
            .unwrap();

        // Each test has its own mocks
        for i in 1..=3 {
            let scope = format!("parallel-test-{i}");
            registry.add_mock(MockDefinition {
                id: format!("mock-test-{i}").into(),
                priority: 100,
                enabled: true,
                once: false,
                source_file: None,
                scope: Some(scope.into()),
                request_transforms: None,
                request: RequestMatcher {
                    methods: smallvec![Method::GET],
                    url_patterns: smallvec![UrlPattern::exact(format!("/api/test{i}"))],
                    ..Default::default()
                },
                response: ResponseGenerator::new(
                    StatusCode::OK,
                    BodySource::inline(format!(r#"{{"test":{i}}}"#)),
                ),
                vars: None,
            });
        }

        assert_eq!(registry.len(), 3);
        assert_eq!(registry.list_scopes().len(), 3);

        // Each test cleans up independently
        registry.delete_scope("parallel-test-2").unwrap();
        assert_eq!(registry.len(), 2);

        registry.delete_scope("parallel-test-1").unwrap();
        registry.delete_scope("parallel-test-3").unwrap();
        assert_eq!(registry.len(), 0);
    }

    println!("✓ Realistic workflow test passed");
}
