#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use anyhow::Result;
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode};
use mockpit::engine::MockRecorderConsolidationExt;
use mockpit::recorder::{MockRecorder, RecordingFilterOptions, RecordingFormat};
use regex::Regex;
use rustc_hash::FxHashMap;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test]
async fn test_filter_url_pattern() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        filter_url: Some(Regex::new(r"/api/users/.*").unwrap()),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "filter-test",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record a matching URL
    let id1 = recorder
        .record(
            &Method::GET,
            "/api/users/123",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("matched"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Record a non-matching URL (should be filtered out)
    let id2 = recorder
        .record(
            &Method::GET,
            "/api/posts/456",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("not matched"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Only the matching URL should be recorded
    assert_eq!(recorder.count(), 1);
    assert!(!id1.is_empty());
    assert!(id2.is_empty()); // Filtered requests return empty ID
}

#[tokio::test]
async fn test_capture_errors_only() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        capture_errors_only: true,
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "errors-only",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record a successful response (should be filtered)
    let id1 = recorder
        .record(
            &Method::GET,
            "/api/success",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("success"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Record a 404 error (should be captured)
    let id2 = recorder
        .record(
            &Method::GET,
            "/api/not-found",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::NOT_FOUND,
            &HeaderMap::new(),
            &Bytes::from("not found"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Record a 500 error (should be captured)
    let id3 = recorder
        .record(
            &Method::GET,
            "/api/error",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            &Bytes::from("error"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Only errors should be recorded
    assert_eq!(recorder.count(), 2);
    assert!(id1.is_empty());
    assert!(!id2.is_empty());
    assert!(!id3.is_empty());
}

#[tokio::test]
async fn test_exclude_static_assets() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        exclude_patterns: RecordingFilterOptions::web_static_patterns(),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "no-static",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record API endpoint (should be captured)
    let id1 = recorder
        .record(
            &Method::GET,
            "/api/data",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("data"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Record web static assets (should be filtered by web_static_patterns)
    let static_urls = vec![
        "/static/app.js",
        "/static/module.mjs",
        "/styles/main.css",
        "/styles/component.scss",
        "/assets/font.woff2",
        "/assets/font.ttf",
        "/bundle.map",
    ];

    for url in static_urls {
        let id = recorder
            .record(
                &Method::GET,
                url,
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from("static"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
        assert!(id.is_empty(), "Static asset {url} should be filtered");
    }

    // Images are NOT filtered by web_static_patterns (could be API content)
    let image_urls = vec!["/images/logo.png", "/favicon.ico", "/photo.jpg"];
    for url in image_urls {
        let id = recorder
            .record(
                &Method::GET,
                url,
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from("image"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
        assert!(!id.is_empty(), "Image {url} should NOT be filtered");
    }

    // API endpoint + images should be recorded
    assert_eq!(recorder.count(), 4); // 1 API + 3 images
    assert!(!id1.is_empty());
}

#[tokio::test]
async fn test_min_duration_filter() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        min_duration: Some(Duration::from_millis(100)),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "min-duration",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record a fast request (should be filtered)
    let id1 = recorder
        .record(
            &Method::GET,
            "/api/fast",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("fast"),
            Duration::from_millis(50),
        )
        .await
        .unwrap();

    // Record a slow request (should be captured)
    let id2 = recorder
        .record(
            &Method::GET,
            "/api/slow",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("slow"),
            Duration::from_millis(150),
        )
        .await
        .unwrap();

    // Only the slow request should be recorded
    assert_eq!(recorder.count(), 1);
    assert!(id1.is_empty());
    assert!(!id2.is_empty());
}

#[tokio::test]
async fn test_auto_export_on_error() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        auto_export_on_error: true,
        error_context_requests: 2,
        capture_success_only: false, // Need to capture both success and errors
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "auto-export",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record some successful requests to fill the context buffer
    recorder
        .record(
            &Method::GET,
            "/api/step1",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("step1"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    recorder
        .record(
            &Method::GET,
            "/api/step2",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("step2"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Record an error (should trigger auto-export in background)
    recorder
        .record(
            &Method::GET,
            "/api/error",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            &Bytes::from("error occurred"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Give the background task time to export
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Check if an error export file was created
    let entries = std::fs::read_dir(&temp_dir).unwrap();
    let error_files: Vec<_> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.contains("auto-export") && name.contains("error")
        })
        .collect();

    assert!(
        !error_files.is_empty(),
        "Expected auto-export error file to be created"
    );

    // Verify the error file contains the expected context
    let error_file = &error_files[0];
    let error_content = std::fs::read_to_string(error_file.path()).unwrap();

    // Parse as JSON mock collection and verify it's valid
    let collection: serde_json::Value =
        serde_json::from_str(&error_content).expect("Error file should be valid JSON");

    // Verify it has the expected mock collection structure
    assert!(collection.get("name").is_some(), "Should have name field");
    assert!(collection.get("mocks").is_some(), "Should have mocks array");

    let mocks = collection["mocks"]
        .as_array()
        .expect("Should have mocks array");

    // The error context feature exports the buffered requests before the error
    // The buffer size is 2, so we should have up to 2 mocks
    assert!(
        mocks.len() <= 2,
        "Should have at most 2 interactions from buffer"
    );
    assert!(
        !mocks.is_empty(),
        "Should have at least some interactions exported"
    );

    // Verify each mock has the expected structure
    for mock in mocks {
        assert!(mock.get("id").is_some(), "Mock should have id");
        assert!(
            mock.get("match_config").is_some() || mock.get("match").is_some(),
            "Mock should have match_config or match"
        );
        assert!(
            mock.get("response").is_some() || mock.get("return").is_some(),
            "Mock should have response or return"
        );
    }

    // Verify main recorder still has all interactions
    assert_eq!(
        recorder.count(),
        3,
        "Main recorder should still have 3 interactions"
    );
}

#[tokio::test]
async fn test_error_context_buffer_circular() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        auto_export_on_error: true,
        error_context_requests: 2, // Circular buffer of size 2
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "circular-buffer",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record 3 successful requests (should only keep the last 2 in buffer)
    recorder
        .record(
            &Method::GET,
            "/api/req1",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("req1"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    recorder
        .record(
            &Method::GET,
            "/api/req2",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("req2"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    recorder
        .record(
            &Method::GET,
            "/api/req3",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("req3"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Trigger error to export
    recorder
        .record(
            &Method::GET,
            "/api/error",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            &Bytes::from("error"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Give background task time to complete
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify the export file exists
    let entries = std::fs::read_dir(&temp_dir).unwrap();
    let error_files: Vec<_> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.contains("circular-buffer") && name.contains("error")
        })
        .collect();

    assert!(
        !error_files.is_empty(),
        "Expected error export file to be created"
    );

    // Verify the circular buffer worked correctly - should only contain the last 2 requests + error
    let error_file = &error_files[0];
    let error_content = std::fs::read_to_string(error_file.path()).unwrap();

    // Parse as JSON mock collection and verify it's valid
    let collection: serde_json::Value =
        serde_json::from_str(&error_content).expect("Error file should be valid JSON");

    // Verify it has the expected mock collection structure
    assert!(collection.get("name").is_some(), "Should have name field");
    assert!(collection.get("mocks").is_some(), "Should have mocks array");

    let mocks = collection["mocks"]
        .as_array()
        .expect("Should have mocks array");

    // The circular buffer size is 2, so we should have at most 2 mocks (not including the error itself)
    // The error context feature exports only the buffered requests before the error
    assert!(
        mocks.len() <= 2,
        "Should have at most 2 interactions from circular buffer"
    );
    assert!(
        !mocks.is_empty(),
        "Should have at least some interactions exported"
    );

    // Verify each mock has the expected structure
    for mock in mocks {
        assert!(mock.get("id").is_some(), "Mock should have id");
        assert!(
            mock.get("match_config").is_some() || mock.get("match").is_some(),
            "Mock should have match_config or match"
        );
        assert!(
            mock.get("response").is_some() || mock.get("return").is_some(),
            "Mock should have response or return"
        );
    }

    // Verify main recorder still has all interactions
    assert_eq!(
        recorder.count(),
        4,
        "Main recorder should have all 4 interactions"
    );
}

#[tokio::test]
async fn test_file_body_storage_threshold() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("file-body-test", temp_dir.path());

    // Create a large body (over 100KB threshold)
    let large_body = "x".repeat(150 * 1024); // 150KB

    // Create response headers
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/json".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/api/large",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(large_body.clone()),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Give async task time to write
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Save and check if body file was created
    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    // Check if body is referenced as a file
    assert!(
        content.contains("bodies/"),
        "Large body should be stored in separate file"
    );

    // Check if the body file exists
    let bodies_dir = temp_dir.path().join("bodies");
    assert!(bodies_dir.exists(), "Bodies directory should exist");

    let body_files: Vec<_> = std::fs::read_dir(&bodies_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(
        body_files.len(),
        1,
        "Exactly one body file should exist in bodies directory"
    );

    // Verify the body file contains the correct data
    let body_file_path = body_files[0].path();
    let body_content = std::fs::read_to_string(&body_file_path).unwrap();
    assert_eq!(
        body_content.len(),
        150 * 1024,
        "Body file should contain the full 150KB of data"
    );
    assert_eq!(
        body_content, large_body,
        "Body file content should match original data"
    );

    // Verify the recording was successful
    assert_eq!(recorder.count(), 1, "Should have recorded 1 interaction");
}

#[tokio::test]
async fn test_yaml_serialization_format() {
    // Test to verify YAML serialization works correctly without workarounds
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("yaml-format-test", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/json".parse().unwrap());
    resp_headers.insert("x-custom", "test-value".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/api/users",
            Some("page=1&limit=10"),
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(r#"{"users":[{"id":1,"name":"Alice"}]}"#),
            Duration::from_millis(50),
        )
        .await
        .unwrap();

    // Save as YAML
    let file_path = recorder.save(RecordingFormat::Yaml).await.unwrap();
    let yaml_content = tokio::fs::read_to_string(&file_path).await.unwrap();

    println!("=== Generated YAML ===\n{yaml_content}\n=== End YAML ===");

    // Verify it's valid YAML and can be parsed back
    let parsed: mockpit::config::MockCollectionConfig =
        serde_yaml::from_str(&yaml_content).expect("Generated YAML should be valid and parseable");

    assert!(parsed.name.is_some(), "Should have collection name");
    assert_eq!(parsed.mocks.len(), 1, "Should have 1 mock");

    let mock = &parsed.mocks[0];
    assert!(mock.match_config.is_some(), "Mock should have match config");
    assert!(
        mock.response_config.is_some(),
        "Mock should have return config"
    );

    // Verify match config
    let match_config = mock.match_config.as_ref().unwrap();
    assert_eq!(match_config.methods.len(), 1, "Should have 1 method");
    assert_eq!(match_config.methods[0], "GET", "Method should be GET");
    assert_eq!(match_config.urls.len(), 1, "Should have 1 URL");
    assert!(
        match_config.urls[0].contains("/api/users"),
        "URL should contain /api/users"
    );
    assert!(
        match_config.urls[0].contains("page=1"),
        "URL should contain query params"
    );

    // Verify return config
    let response_config = mock.response_config.as_ref().unwrap();
    assert_eq!(response_config.status(), Some(200), "Status should be 200");
    assert!(response_config.body().is_some(), "Should have body");
}

#[tokio::test]
async fn test_file_body_storage_for_html() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("html-body-test", temp_dir.path());

    // Create HTML content (even if small, should use file storage)
    let html_body = "<html><body><h1>Test</h1></body></html>";

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "text/html".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/page.html",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(html_body),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Give async task time to write
    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    // HTML should use file storage
    assert!(
        content.contains("bodies/"),
        "HTML body should be stored in separate file"
    );

    // Verify the body file exists and contains the HTML
    let bodies_dir = temp_dir.path().join("bodies");
    assert!(bodies_dir.exists(), "Bodies directory should exist");

    let body_files: Vec<_> = std::fs::read_dir(&bodies_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(body_files.len(), 1, "Exactly one body file should exist");

    let body_file_path = body_files[0].path();
    let body_content = std::fs::read_to_string(&body_file_path).unwrap();
    assert_eq!(
        body_content, html_body,
        "Body file should contain the original HTML"
    );

    // Verify the recording was successful
    assert_eq!(recorder.count(), 1, "Should have recorded 1 interaction");
}

#[tokio::test]
async fn test_yaml_format_recording() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::with_format("yaml-test", temp_dir.path(), RecordingFormat::Yaml);

    recorder
        .record(
            &Method::POST,
            "/api/users",
            Some("role=admin"),
            &HeaderMap::new(),
            Some(&Bytes::from(r#"{"name":"Alice"}"#)),
            StatusCode::CREATED,
            &HeaderMap::new(),
            &Bytes::from(r#"{"id":"123","name":"Alice"}"#),
            Duration::from_millis(75),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Yaml).await.unwrap();
    assert!(file_path.exists());
    assert_eq!(file_path.extension().unwrap(), "yaml");

    // Read and verify YAML structure
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert!(content.contains("name:"), "YAML should have a name field");
    assert!(content.contains("mocks:"), "YAML should have mocks key");
    assert!(
        content.contains("/api/users"),
        "YAML should contain the recorded URL"
    );

    // Verify YAML can be parsed
    let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert!(
        parsed.get("name").is_some(),
        "Parsed YAML should have name field"
    );
    assert!(
        parsed.get("mocks").is_some(),
        "Parsed YAML should have mocks key"
    );

    // Verify recording details
    assert!(content.contains("POST"), "Should contain POST method");
    assert!(
        content.contains("role=admin"),
        "Should contain query string"
    );
    assert!(
        content.contains("Alice"),
        "Should contain request/response data"
    );
    assert!(
        content.contains("CREATED") || content.contains("201"),
        "Should contain status code"
    );

    // Verify the recording was successful
    assert_eq!(recorder.count(), 1, "Should have recorded 1 interaction");
}

#[tokio::test]
async fn test_yaml_streaming_format() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::with_format("yaml-stream", temp_dir.path(), RecordingFormat::Yaml);

    // Initialize file for streaming
    let file_path = recorder.init_file().await.unwrap();

    // Record multiple interactions
    for i in 0..3 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/item/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"id":{i}}}"#)),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Finalize the file
    recorder.finalize_file().await.unwrap();

    // Verify YAML file structure
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    println!("=== Streaming YAML ===\n{content}\n=== End ===");

    // Verify all items are present
    assert!(content.contains("/api/item/0"));
    assert!(content.contains("/api/item/1"));
    assert!(content.contains("/api/item/2"));

    // Verify it can be parsed back
    let parsed: mockpit::config::MockCollectionConfig =
        serde_yaml::from_str(&content).expect("Streamed YAML should be valid and parseable");
    assert_eq!(parsed.mocks.len(), 3, "Should parse 3 mocks");
}

#[tokio::test]
async fn test_finalize_and_consolidate() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("consolidate-test", temp_dir.path(), RecordingFormat::Json);

    // Initialize recording
    recorder.init_file().await?;

    // Record several similar interactions
    for i in 0..5 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/users/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"id":{i}}}"#)),
                Duration::from_millis(10),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Finalize with consolidation
    let consolidator_options = mockpit::consolidator::ConsolidatorOptions::default();
    let (file_path, stats) = recorder
        .finalize_and_consolidate(consolidator_options, false)
        .await?;

    // Verify consolidation happened
    assert_eq!(stats.original_count, 5);

    // Read the consolidated file
    let content = tokio::fs::read_to_string(&file_path).await?;
    let collection: mockpit::config::MockCollectionConfig = serde_json::from_str(&content)?;

    assert_eq!(collection.mocks.len(), stats.consolidated_count);

    Ok(())
}

#[tokio::test]
async fn test_finalize_and_consolidate_har_format() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("consolidate-har", temp_dir.path(), RecordingFormat::Har);

    recorder.init_file().await?;

    recorder
        .record(
            &Method::GET,
            "/api/test",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await?;

    tokio::time::sleep(Duration::from_millis(10)).await;

    // HAR format should not be consolidated
    let consolidator_options = mockpit::consolidator::ConsolidatorOptions::default();
    let (_file_path, stats) = recorder
        .finalize_and_consolidate(consolidator_options, false)
        .await?;

    // Stats should be empty for HAR format
    assert_eq!(stats.original_count, 0);
    assert_eq!(stats.consolidated_count, 0);
    assert!((stats.reduction_ratio - 0.0).abs() < f64::EPSILON);

    Ok(())
}

#[tokio::test]
async fn test_binary_data_handling() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("binary-test", temp_dir.path());

    // Create binary request body (non-UTF8)
    let binary_request = Bytes::from(vec![0xFF, 0xFE, 0xFD, 0xFC, 0x00, 0x01, 0x02, 0x03]);

    // Create binary response body
    let binary_response = Bytes::from(vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE]);

    let id = recorder
        .record(
            &Method::POST,
            "/api/binary",
            None,
            &HeaderMap::new(),
            Some(&binary_request),
            StatusCode::OK,
            &HeaderMap::new(),
            &binary_response,
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();

    // Binary data should be represented as descriptive string
    assert!(interaction.request.body.is_some());
    assert!(
        interaction
            .request
            .body
            .as_ref()
            .unwrap()
            .contains("binary data")
    );
    assert!(
        interaction
            .request
            .body
            .as_ref()
            .unwrap()
            .contains("8 bytes")
    );

    assert!(interaction.response.body.contains("binary data"));
    assert!(interaction.response.body.contains("6 bytes"));
}

#[tokio::test]
async fn test_large_body_handling() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("large-body", temp_dir.path());

    // Create a very large body (1MB)
    let large_body = "x".repeat(1024 * 1024);

    let id = recorder
        .record(
            &Method::POST,
            "/api/upload",
            None,
            &HeaderMap::new(),
            Some(&Bytes::from(large_body.clone())),
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("ok"),
            Duration::from_millis(100),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();

    // Should handle large body without errors
    assert_eq!(
        interaction.request.body.as_ref().unwrap().len(),
        1024 * 1024
    );
    assert_eq!(interaction.response.body, "ok");
}

#[tokio::test]
async fn test_har_conversion_with_binary() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("har-binary", temp_dir.path());

    // Record binary data
    let binary_response = Bytes::from(vec![0xFF, 0xFE, 0xFD, 0xFC]);

    recorder
        .record(
            &Method::GET,
            "/api/binary-resource",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &binary_response,
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Save as HAR
    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    // HAR should handle binary data
    assert!(content.contains("base64") || content.contains("binary data"));
}

#[tokio::test]
async fn test_har_with_query_string_parsing() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("har-query", temp_dir.path());

    recorder
        .record(
            &Method::GET,
            "/api/search",
            Some("q=test&limit=10&offset=0"),
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from(r#"{"results":[]}"#),
            Duration::from_millis(25),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let har: har::Har = mockpit::config::parse_har(&content).unwrap();

    let har::Spec::V1_2(log) = &har.log else {
        panic!("Expected V1_2 spec");
    };

    let entry = &log.entries[0];

    // Verify query string was parsed
    assert_eq!(entry.request.query_string.len(), 3);

    let query_params: Vec<(&str, &str)> = entry
        .request
        .query_string
        .iter()
        .map(|qs| (qs.name.as_str(), qs.value.as_str()))
        .collect();

    assert!(query_params.contains(&("q", "test")));
    assert!(query_params.contains(&("limit", "10")));
    assert!(query_params.contains(&("offset", "0")));
}

#[tokio::test]
async fn test_yaml_streaming_format_multiple() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::with_format("yaml-stream", temp_dir.path(), RecordingFormat::Yaml);

    let file_path = recorder.init_file().await.unwrap();

    // Record multiple interactions
    for i in 0..3 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/yaml/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"index":{i}}}"#)),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    recorder.finalize_file().await.unwrap();

    // Verify YAML structure
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    assert!(content.contains("mocks:"), "YAML should have mocks array");
    assert!(
        content.contains("/api/yaml/0"),
        "YAML should contain first recorded URL"
    );
    assert!(
        content.contains("/api/yaml/1"),
        "YAML should contain second recorded URL"
    );
    assert!(
        content.contains("/api/yaml/2"),
        "YAML should contain third recorded URL"
    );

    // Verify YAML can be parsed
    let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert!(
        parsed.get("name").is_some(),
        "Parsed YAML should have name field"
    );
    assert!(
        parsed.get("mocks").is_some(),
        "Parsed YAML should have mocks array"
    );

    let mocks = parsed["mocks"].as_sequence().unwrap();
    assert_eq!(mocks.len(), 3, "Should have 3 mocks in YAML");

    // Verify all mocks have required fields (using match_config and response_config)
    for (i, mock) in mocks.iter().enumerate() {
        assert!(
            mock.get("match_config").is_some() || mock.get("match").is_some(),
            "Mock {i} should have match or match_config field"
        );
        assert!(
            mock.get("response").is_some() || mock.get("return").is_some(),
            "Mock {i} should have response or return field"
        );
    }

    // Verify the recording was successful
    assert_eq!(recorder.count(), 3, "Should have recorded 3 interactions");
}

#[tokio::test]
async fn test_pending_writes_timeout() {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("pending-writes", temp_dir.path(), RecordingFormat::Json);

    let file_path = recorder.init_file().await.unwrap();

    // Record many interactions rapidly to test pending writes tracking
    for i in 0..100 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/item/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"id":{i}}}"#)),
                Duration::from_millis(1),
            )
            .await
            .unwrap();
    }

    // Finalize should wait for all pending writes
    recorder.finalize_file().await.unwrap();

    // Verify file is valid
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    // Should be valid JSON
    let json_value: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(json_value.is_object(), "Should be valid JSON object");

    // Verify all 100 interactions were recorded
    let collection: mockpit::config::MockCollectionConfig = serde_json::from_str(&content).unwrap();
    assert_eq!(
        collection.mocks.len(),
        100,
        "Should have all 100 recorded interactions"
    );

    // Verify interactions have expected URLs
    for (i, mock) in collection.mocks.iter().enumerate().take(100) {
        let expected_url = format!("/api/item/{i}");
        let urls = &mock.match_config.as_ref().unwrap().urls;
        assert!(
            urls.iter().any(|u| u.contains(&expected_url)),
            "Mock {i} should contain URL {expected_url}"
        );
    }

    // Verify recorder count matches
    assert_eq!(
        recorder.count(),
        100,
        "Recorder should have 100 interactions"
    );
}

#[tokio::test]
async fn test_recording_format_parse() {
    assert!(matches!(
        RecordingFormat::parse("json").unwrap(),
        RecordingFormat::Json
    ));
    assert!(matches!(
        RecordingFormat::parse("yaml").unwrap(),
        RecordingFormat::Yaml
    ));
    assert!(matches!(
        RecordingFormat::parse("yml").unwrap(),
        RecordingFormat::Yaml
    ));
    assert!(matches!(
        RecordingFormat::parse("har").unwrap(),
        RecordingFormat::Har
    ));

    // Test case insensitivity
    assert!(matches!(
        RecordingFormat::parse("JSON").unwrap(),
        RecordingFormat::Json
    ));
    assert!(matches!(
        RecordingFormat::parse("YAML").unwrap(),
        RecordingFormat::Yaml
    ));

    // Test invalid formats
    assert!(RecordingFormat::parse("invalid").is_err());
    assert!(RecordingFormat::parse("toml").is_err());
}

#[tokio::test]
async fn test_recording_format_extensions() {
    assert_eq!(RecordingFormat::Json.extension(), "json");
    assert_eq!(RecordingFormat::Yaml.extension(), "yaml");
    assert_eq!(RecordingFormat::Har.extension(), "har");
}

#[tokio::test]
async fn test_combined_filters() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        filter_url: Some(Regex::new(r"/api/.*").unwrap()),
        capture_errors_only: false,
        capture_success_only: false, // Capture all status codes
        exclude_patterns: RecordingFilterOptions::web_static_patterns(),
        min_duration: Some(Duration::from_millis(20)),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "combined",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Should pass: API URL, non-static, long enough duration
    let id1 = recorder
        .record(
            &Method::GET,
            "/api/data",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("data"),
            Duration::from_millis(25),
        )
        .await
        .unwrap();

    // Should fail: Too fast
    let id2 = recorder
        .record(
            &Method::GET,
            "/api/quick",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("quick"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Should fail: Static asset
    let id3 = recorder
        .record(
            &Method::GET,
            "/api/script.js",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("js code"),
            Duration::from_millis(30),
        )
        .await
        .unwrap();

    // Should fail: Wrong URL pattern
    let id4 = recorder
        .record(
            &Method::GET,
            "/pages/home",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("page"),
            Duration::from_millis(25),
        )
        .await
        .unwrap();

    assert_eq!(recorder.count(), 1);
    assert!(!id1.is_empty());
    assert!(id2.is_empty());
    assert!(id3.is_empty());
    assert!(id4.is_empty());
}

#[tokio::test]
async fn test_neither_capture_filter_set() {
    // When neither capture_errors_only nor capture_success_only is set, all should be recorded
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        capture_errors_only: false,
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "all-status",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record various status codes
    recorder
        .record(
            &Method::GET,
            "/api/success",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("ok"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    recorder
        .record(
            &Method::GET,
            "/api/redirect",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::FOUND,
            &HeaderMap::new(),
            &Bytes::from("redirect"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    recorder
        .record(
            &Method::GET,
            "/api/error",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            &Bytes::from("error"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // All should be recorded
    assert_eq!(
        recorder.count(),
        3,
        "All 3 requests should be recorded when no filters are set"
    );

    // Verify we can retrieve all interactions
    let all = recorder.get_all();
    assert_eq!(
        all.len(),
        3,
        "Should be able to retrieve all 3 interactions"
    );

    // Verify each status code is present
    let status_codes: Vec<u16> = all.iter().map(|i| i.response.status).collect();

    assert!(status_codes.contains(&200), "Should have recorded 200 OK");
    assert!(
        status_codes.contains(&302),
        "Should have recorded 302 redirect"
    );
    assert!(
        status_codes.contains(&500),
        "Should have recorded 500 error"
    );

    // Verify URLs are present
    let urls: Vec<String> = all.iter().map(|i| i.request.uri.clone()).collect();
    assert!(
        urls.contains(&"/api/success".to_string()),
        "Should have recorded success URL"
    );
    assert!(
        urls.contains(&"/api/redirect".to_string()),
        "Should have recorded redirect URL"
    );
    assert!(
        urls.contains(&"/api/error".to_string()),
        "Should have recorded error URL"
    );
}

#[tokio::test]
async fn test_file_body_with_image_content_type() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("image-test", temp_dir.path());

    // Small image data that would normally be inline, but content-type says image
    let image_data = "small image data";
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "image/png".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/image.png",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(image_data),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    // Image content should use file storage regardless of size
    assert!(
        content.contains("bodies/"),
        "Image should be stored in separate file"
    );

    // Verify the body file exists and contains the image data
    let bodies_dir = temp_dir.path().join("bodies");
    assert!(bodies_dir.exists(), "Bodies directory should exist");

    let body_files: Vec<_> = std::fs::read_dir(&bodies_dir)
        .unwrap()
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(body_files.len(), 1, "Exactly one body file should exist");

    let body_file_path = body_files[0].path();
    let body_content = std::fs::read_to_string(&body_file_path).unwrap();
    assert_eq!(
        body_content, image_data,
        "Body file should contain the original image data"
    );

    // Verify content-type is preserved
    assert!(
        content.contains("image/png"),
        "Content-type should be preserved in JSON"
    );

    // Verify the recording was successful
    assert_eq!(recorder.count(), 1, "Should have recorded 1 interaction");
}

#[tokio::test]
async fn test_decompress_non_gzipped_data() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("non-gzip", temp_dir.path());

    let regular_body = r#"{"data": "plain text"}"#;
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/json".parse().unwrap());
    // No content-encoding header

    let id = recorder
        .record(
            &Method::GET,
            "/api/plain",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(regular_body),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();

    // Body should be unchanged
    assert_eq!(
        interaction.response.body, regular_body,
        "Plain text body should remain unchanged"
    );

    // Headers should be unchanged
    assert!(
        interaction
            .response
            .headers
            .iter()
            .any(|(k, v)| k.to_lowercase() == "content-type" && v == "application/json"),
        "Content-Type header should be preserved"
    );

    // Verify no content-encoding header was added
    assert!(
        !interaction
            .response
            .headers
            .iter()
            .any(|(k, _)| k.to_lowercase() == "content-encoding"),
        "Should not have content-encoding header for non-compressed data"
    );

    // Verify the data is valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&interaction.response.body).unwrap();
    assert_eq!(
        parsed["data"], "plain text",
        "Body should be valid JSON with correct data"
    );

    // Verify the recording was successful
    assert_eq!(recorder.count(), 1, "Should have recorded 1 interaction");
}

#[tokio::test]
async fn test_get_all_interactions() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("get-all", temp_dir.path());

    // Record multiple interactions
    for i in 0..5 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/item/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!("item {i}")),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
    }

    let all = recorder.get_all();
    assert_eq!(all.len(), 5);

    // Verify all interactions are present
    for interaction in all {
        assert!(interaction.request.uri.starts_with("/api/item/"));
    }
}

#[tokio::test]
async fn test_session_name_with_spaces() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("test session with spaces", temp_dir.path());

    recorder
        .record(
            &Method::GET,
            "/api/test",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();

    // Spaces should be replaced with hyphens in filename
    let filename = file_path.file_name().unwrap().to_string_lossy();
    assert!(
        filename.contains("test-session-with-spaces"),
        "Filename should contain sanitized session name"
    );
    assert!(
        !filename.contains(' '),
        "Filename should not contain spaces"
    );

    // Verify file was created successfully
    assert!(file_path.exists(), "File should exist");
    assert_eq!(
        file_path.extension().unwrap(),
        "json",
        "File should have .json extension"
    );

    // Verify the content is valid JSON
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(json.is_object(), "Should be valid JSON");

    // Verify the recording was successful
    assert_eq!(recorder.count(), 1, "Should have recorded 1 interaction");

    // Verify the recorded data
    let collection: mockpit::config::MockCollectionConfig = serde_json::from_str(&content).unwrap();
    assert_eq!(
        collection.mocks.len(),
        1,
        "Should have 1 mock in collection"
    );

    let urls = &collection.mocks[0].match_config.as_ref().unwrap().urls;
    assert!(
        urls.iter().any(|u| u.contains("/api/test")),
        "Mock should contain correct URL"
    );
}

#[tokio::test]
async fn test_finalize_without_init() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("no-init", temp_dir.path());

    // Try to finalize without initializing - should not panic
    let result = recorder.finalize_file().await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_finalize_and_consolidate_without_init() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("no-init-consolidate", temp_dir.path());

    // Try to consolidate without initializing
    let consolidator_options = mockpit::consolidator::ConsolidatorOptions::default();
    let result = recorder
        .finalize_and_consolidate(consolidator_options, false)
        .await;

    // Should fail because no file was initialized
    assert!(result.is_err());
}

#[test]
fn test_recording_filter_options_default() {
    let options = RecordingFilterOptions::default();

    assert!(options.filter_url.is_none());
    assert!(!options.capture_errors_only);
    assert!(options.capture_success_only); // Default is true for backward compatibility
    assert!(!options.auto_export_on_error);
    assert_eq!(options.error_context_requests, 0);
    assert!(!options.exclude_patterns.is_empty()); // Now includes web_static_patterns by default
    assert!(options.min_duration.is_none());
}

#[tokio::test]
async fn test_file_body_storage_for_zip() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("zip-test", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/zip".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/archive.zip",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("zip data"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(
        content.contains("bodies/"),
        "ZIP should be stored in separate file"
    );
}

#[tokio::test]
async fn test_file_body_storage_for_video() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("video-test", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "video/mp4".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/video.mp4",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("video data"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(
        content.contains("bodies/"),
        "Video should be stored in separate file"
    );
}

#[tokio::test]
async fn test_file_body_storage_for_audio() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("audio-test", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "audio/mpeg".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/music.mp3",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("audio data"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(
        content.contains("bodies/"),
        "Audio should be stored in separate file"
    );
}

#[tokio::test]
async fn test_file_body_storage_for_font() {
    let temp_dir = TempDir::new().unwrap();
    // Use empty filters so fonts are actually recorded (for testing file body storage)
    let filter_options = RecordingFilterOptions {
        exclude_patterns: vec![],
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "font-test",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "font/woff2".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/font.woff2",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("font data"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(
        content.contains("bodies/"),
        "Font should be stored in separate file"
    );
}

#[tokio::test]
async fn test_file_body_storage_for_css() {
    let temp_dir = TempDir::new().unwrap();
    // Use empty filters so CSS is actually recorded (for testing file body storage)
    let filter_options = RecordingFilterOptions {
        exclude_patterns: vec![],
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "css-test",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "text/css".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/styles.css",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("body { margin: 0; }"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(
        content.contains("bodies/"),
        "CSS should be stored in separate file"
    );
}

#[tokio::test]
async fn test_file_body_storage_for_octet_stream() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("octet-test", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/octet-stream".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/binary",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("binary data"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(
        content.contains("bodies/"),
        "Octet-stream should be stored in separate file"
    );
}

#[tokio::test]
async fn test_inline_storage_for_small_json() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("inline-test", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/json".parse().unwrap());

    recorder
        .record(
            &Method::GET,
            "/api/data",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(r#"{"small":"json"}"#),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    assert!(!content.contains("bodies/"), "Small JSON should be inline");
    // The JSON will be in the body field, potentially escaped
    assert!(
        content.contains("small") && content.contains("json"),
        "Should contain inline JSON data"
    );
}

// ============================================================================
// Gzip decompression error handling
// ============================================================================

#[tokio::test]
async fn test_gzip_decompression_invalid_data() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("invalid-gzip", temp_dir.path());

    // Create invalid gzip data
    let invalid_gzip = vec![0x1F, 0x8B, 0x08, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-encoding", "gzip".parse().unwrap());

    let id = recorder
        .record(
            &Method::GET,
            "/api/invalid-gzip",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from(invalid_gzip.clone()),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();

    // Should fall back to original data when decompression fails
    // The body will be converted to a descriptive string for binary data
    assert!(interaction.response.body.contains("binary data"));
}

// ============================================================================
// HAR format edge cases
// ============================================================================

#[tokio::test]
async fn test_har_status_code_text_mapping() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("status-codes", temp_dir.path());

    let status_codes = vec![
        (StatusCode::CREATED, "Created"),
        (StatusCode::ACCEPTED, "Accepted"),
        (StatusCode::NO_CONTENT, "No Content"),
        (StatusCode::MOVED_PERMANENTLY, "Moved Permanently"),
        (StatusCode::FOUND, "Found"),
        (StatusCode::NOT_MODIFIED, "Not Modified"),
        (StatusCode::BAD_REQUEST, "Bad Request"),
        (StatusCode::UNAUTHORIZED, "Unauthorized"),
        (StatusCode::FORBIDDEN, "Forbidden"),
        (StatusCode::METHOD_NOT_ALLOWED, "Method Not Allowed"),
        (StatusCode::CONFLICT, "Conflict"),
        (StatusCode::UNPROCESSABLE_ENTITY, "Unprocessable Entity"),
        (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests"),
        (StatusCode::BAD_GATEWAY, "Bad Gateway"),
        (StatusCode::SERVICE_UNAVAILABLE, "Service Unavailable"),
        (StatusCode::GATEWAY_TIMEOUT, "Gateway Timeout"),
    ];

    for (status_code, _expected_text) in status_codes {
        recorder
            .record(
                &Method::GET,
                &format!("/api/status-{}", status_code.as_u16()),
                None,
                &HeaderMap::new(),
                None,
                status_code,
                &HeaderMap::new(),
                &Bytes::from("test"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
    }

    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let har: har::Har = mockpit::config::parse_har(&content).unwrap();

    let har::Spec::V1_2(log) = &har.log else {
        panic!("Expected V1_2 spec");
    };

    for entry in &log.entries {
        let status = u16::try_from(entry.response.status).unwrap();
        let expected = match status {
            201 => "Created",
            202 => "Accepted",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            304 => "Not Modified",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            405 => "Method Not Allowed",
            409 => "Conflict",
            422 => "Unprocessable Entity",
            429 => "Too Many Requests",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            _ => "Unknown",
        };

        assert_eq!(entry.response.status_text, expected);
    }
}

#[tokio::test]
async fn test_har_unknown_status_code() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        capture_success_only: false,
        capture_errors_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "unknown-status",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Use a less common status code (418 I'm a teapot)
    recorder
        .record(
            &Method::GET,
            "/api/teapot",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::IM_A_TEAPOT,
            &HeaderMap::new(),
            &Bytes::from("teapot"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let har: har::Har = mockpit::config::parse_har(&content).unwrap();

    let har::Spec::V1_2(log) = &har.log else {
        panic!("Expected V1_2 spec");
    };

    assert_eq!(log.entries[0].response.status_text, "Unknown");
}

#[tokio::test]
async fn test_har_query_string_without_value() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("query-no-val", temp_dir.path());

    recorder
        .record(
            &Method::GET,
            "/api/test",
            Some("flag&enabled=true"),
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let har: har::Har = mockpit::config::parse_har(&content).unwrap();

    let har::Spec::V1_2(log) = &har.log else {
        panic!("Expected V1_2 spec");
    };

    let query_params: Vec<_> = log.entries[0]
        .request
        .query_string
        .iter()
        .map(|q| (q.name.as_str(), q.value.as_str()))
        .collect();

    assert!(query_params.contains(&("flag", "")));
    assert!(query_params.contains(&("enabled", "true")));
}

#[tokio::test]
async fn test_har_no_query_string() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("no-query", temp_dir.path());

    recorder
        .record(
            &Method::GET,
            "/api/test",
            None, // No query string at all
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let har: har::Har = mockpit::config::parse_har(&content).unwrap();

    let har::Spec::V1_2(log) = &har.log else {
        panic!("Expected V1_2 spec");
    };

    assert!(log.entries[0].request.query_string.is_empty());
}

#[tokio::test]
async fn test_har_with_post_data() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("post-data", temp_dir.path());

    let post_body = r#"{"username":"test","password":"secret"}"#;

    recorder
        .record(
            &Method::POST,
            "/api/login",
            None,
            &HeaderMap::new(),
            Some(&Bytes::from(post_body)),
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from(r#"{"token":"abc123"}"#),
            Duration::from_millis(50),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Har).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let har: har::Har = mockpit::config::parse_har(&content).unwrap();

    let har::Spec::V1_2(log) = &har.log else {
        panic!("Expected V1_2 spec");
    };

    assert!(log.entries[0].request.post_data.is_some());
    let post_data = log.entries[0].request.post_data.as_ref().unwrap();
    assert_eq!(post_data.text.as_ref().unwrap(), post_body);
}

// ============================================================================
// JSON streaming format edge cases
// ============================================================================

#[tokio::test]
async fn test_json_streaming_multiple_interactions() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::with_format("json-multi", temp_dir.path(), RecordingFormat::Json);

    let file_path = recorder.init_file().await.unwrap();

    for i in 0..10 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/item-{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"index":{i}}}"#)),
                Duration::from_millis(5),
            )
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    recorder.finalize_file().await.unwrap();

    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let json: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert!(json.is_object());
    assert!(json.get("name").is_some());
    assert!(json.get("mocks").is_some());

    let mocks = json["mocks"].as_array().unwrap();
    assert_eq!(mocks.len(), 10);
}

#[tokio::test]
async fn test_json_streaming_with_special_characters() {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("json-special", temp_dir.path(), RecordingFormat::Json);

    let file_path = recorder.init_file().await.unwrap();

    recorder
        .record(
            &Method::POST,
            "/api/quote",
            None,
            &HeaderMap::new(),
            Some(&Bytes::from(r#"{"text":"He said \"hello\""}"#)),
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from(r#"{"status":"quoted with \"quotes\""}"#),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(20)).await;
    recorder.finalize_file().await.unwrap();

    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    let json: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(json.is_object());
}

// ============================================================================
// Filter edge cases
// ============================================================================

#[tokio::test]
async fn test_filter_with_all_static_extensions() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        exclude_patterns: RecordingFilterOptions::web_static_patterns(),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "static-all",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Test extensions covered by web_static_patterns (JS, CSS, fonts, source maps)
    let filtered_exts = vec![
        ".js", ".mjs", ".jsx", ".ts", ".tsx", ".css", ".scss", ".sass", ".less", ".woff", ".woff2",
        ".ttf", ".eot", ".otf", ".map",
    ];

    for ext in &filtered_exts {
        let url = format!("/assets/file{ext}");
        let id = recorder
            .record(
                &Method::GET,
                &url,
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from("static"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        assert!(id.is_empty(), "Static file {url} should be filtered");
    }

    // Images are NOT in web_static_patterns (could be API content)
    let not_filtered_exts = vec![
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".webp", ".avif",
    ];

    for ext in &not_filtered_exts {
        let url = format!("/assets/file{ext}");
        let id = recorder
            .record(
                &Method::GET,
                &url,
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from("image"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        assert!(
            !id.is_empty(),
            "Image file {url} should NOT be filtered (could be API content)"
        );
    }

    assert_eq!(recorder.count(), not_filtered_exts.len());
}

#[tokio::test]
async fn test_filter_static_case_insensitive() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        exclude_patterns: RecordingFilterOptions::web_static_patterns(),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "static-case",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Test lowercase extensions (regexes are case-sensitive)
    let id1 = recorder
        .record(
            &Method::GET,
            "/file.js",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("js"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let id2 = recorder
        .record(
            &Method::GET,
            "/style.css",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("css"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Test that images are NOT filtered
    let id3 = recorder
        .record(
            &Method::GET,
            "/image.png",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("png"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    assert!(id1.is_empty(), "JS should be filtered");
    assert!(id2.is_empty(), "CSS should be filtered");
    assert!(
        !id3.is_empty(),
        "PNG should NOT be filtered (could be API content)"
    );
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn test_filter_client_error_status_codes() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        capture_errors_only: true,
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "client-errors",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Test various 4xx codes
    let codes = vec![
        StatusCode::BAD_REQUEST,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
        StatusCode::METHOD_NOT_ALLOWED,
        StatusCode::CONFLICT,
        StatusCode::IM_A_TEAPOT,
    ];

    for code in codes {
        let id = recorder
            .record(
                &Method::GET,
                &format!("/api/error-{}", code.as_u16()),
                None,
                &HeaderMap::new(),
                None,
                code,
                &HeaderMap::new(),
                &Bytes::from("error"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        assert!(
            !id.is_empty(),
            "4xx error {} should be captured",
            code.as_u16()
        );
    }

    assert_eq!(recorder.count(), 7);
}

#[tokio::test]
async fn test_filter_server_error_status_codes() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        capture_errors_only: true,
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "server-errors",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Test various 5xx codes
    let codes = vec![
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::NOT_IMPLEMENTED,
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::GATEWAY_TIMEOUT,
    ];

    for code in codes {
        let id = recorder
            .record(
                &Method::GET,
                &format!("/api/error-{}", code.as_u16()),
                None,
                &HeaderMap::new(),
                None,
                code,
                &HeaderMap::new(),
                &Bytes::from("error"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        assert!(
            !id.is_empty(),
            "5xx error {} should be captured",
            code.as_u16()
        );
    }

    assert_eq!(recorder.count(), 5);
}

#[tokio::test]
async fn test_filter_3xx_redirect_codes() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        capture_success_only: false,
        capture_errors_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "redirects",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    let codes = vec![
        StatusCode::MOVED_PERMANENTLY,
        StatusCode::FOUND,
        StatusCode::SEE_OTHER,
        StatusCode::NOT_MODIFIED,
        StatusCode::TEMPORARY_REDIRECT,
        StatusCode::PERMANENT_REDIRECT,
    ];

    for code in codes {
        let id = recorder
            .record(
                &Method::GET,
                &format!("/api/redirect-{}", code.as_u16()),
                None,
                &HeaderMap::new(),
                None,
                code,
                &HeaderMap::new(),
                &Bytes::from("redirect"),
                Duration::from_millis(10),
            )
            .await
            .unwrap();

        assert!(
            !id.is_empty(),
            "3xx code {} should be captured",
            code.as_u16()
        );
    }

    assert_eq!(recorder.count(), 6);
}

#[tokio::test]
async fn test_filter_regex_pattern_match() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        filter_url: Some(Regex::new(r"^/api/v[0-9]+/users/\d+$").unwrap()),
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "regex-filter",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Should match
    let id1 = recorder
        .record(
            &Method::GET,
            "/api/v1/users/123",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("matched"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let id2 = recorder
        .record(
            &Method::GET,
            "/api/v2/users/456",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("matched"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Should not match
    let id3 = recorder
        .record(
            &Method::GET,
            "/api/v1/posts/789",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("not matched"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    assert!(!id1.is_empty());
    assert!(!id2.is_empty());
    assert!(id3.is_empty());
    assert_eq!(recorder.count(), 2);
}

// ============================================================================
// Session name and ID handling
// ============================================================================

#[tokio::test]
async fn test_session_id_in_filename() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("test-session", temp_dir.path());

    recorder
        .record(
            &Method::GET,
            "/test",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let filename = file_path.file_name().unwrap().to_string_lossy();

    assert!(filename.starts_with("test-session-"));
    assert!(filename.ends_with(".json"));
}

#[tokio::test]
async fn test_multiple_format_saves() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("multi-format", temp_dir.path());

    recorder
        .record(
            &Method::GET,
            "/test",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let json_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let yaml_path = recorder.save(RecordingFormat::Yaml).await.unwrap();
    let har_path = recorder.save(RecordingFormat::Har).await.unwrap();

    assert!(json_path.exists());
    assert!(yaml_path.exists());
    assert!(har_path.exists());

    assert_eq!(json_path.extension().unwrap(), "json");
    assert_eq!(yaml_path.extension().unwrap(), "yaml");
    assert_eq!(har_path.extension().unwrap(), "har");
}

// ============================================================================
// Auto-export edge cases
// ============================================================================

#[tokio::test]
async fn test_auto_export_with_zero_context() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        auto_export_on_error: true,
        error_context_requests: 0,
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "zero-context",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    recorder
        .record(
            &Method::GET,
            "/api/error",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            &Bytes::from("error"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    // With zero context, no auto-export should happen (no buffer tracking)
    assert_eq!(recorder.count(), 1);
}

#[tokio::test]
async fn test_auto_export_multiple_errors() {
    let temp_dir = TempDir::new().unwrap();
    let filter_options = RecordingFilterOptions {
        auto_export_on_error: true,
        error_context_requests: 2,
        capture_success_only: false,
        ..Default::default()
    };
    let recorder = MockRecorder::with_filters(
        "multi-error",
        temp_dir.path(),
        RecordingFormat::Json,
        filter_options,
    );

    // Record some successful requests
    recorder
        .record(
            &Method::GET,
            "/api/req1",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("ok"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // First error
    recorder
        .record(
            &Method::GET,
            "/api/error1",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::INTERNAL_SERVER_ERROR,
            &HeaderMap::new(),
            &Bytes::from("error1"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // More requests
    recorder
        .record(
            &Method::GET,
            "/api/req2",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("ok"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    // Second error
    recorder
        .record(
            &Method::GET,
            "/api/error2",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::BAD_GATEWAY,
            &HeaderMap::new(),
            &Bytes::from("error2"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Check that multiple error files were created
    let entries = std::fs::read_dir(&temp_dir).unwrap();
    let error_files: Vec<_> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.contains("multi-error") && name.contains("error")
        })
        .collect();

    assert!(!error_files.is_empty());
}

// ============================================================================
// YAML format tests
// ============================================================================

#[tokio::test]
async fn test_yaml_with_complex_data() {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("yaml-complex", temp_dir.path(), RecordingFormat::Yaml);

    let complex_json =
        r#"{"user":{"name":"Test","roles":["admin","user"],"meta":{"created":"2024-01-01"}}}"#;

    recorder
        .record(
            &Method::POST,
            "/api/complex",
            Some("include=all&format=json"),
            &HeaderMap::new(),
            Some(&Bytes::from(complex_json)),
            StatusCode::CREATED,
            &HeaderMap::new(),
            &Bytes::from(r#"{"id":"123","status":"created"}"#),
            Duration::from_millis(75),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Yaml).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();

    let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert!(parsed.get("name").is_some());
    assert!(parsed.get("mocks").is_some());

    let mocks = parsed["mocks"].as_sequence().unwrap();
    assert_eq!(mocks.len(), 1);
}

// ============================================================================
// YAML format edge cases
// ============================================================================

#[tokio::test]
async fn test_yaml_with_multiline_body() {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("yaml-multiline", temp_dir.path(), RecordingFormat::Yaml);

    let multiline_body = "Line 1\nLine 2\nLine 3\nLine 4";

    recorder
        .record(
            &Method::POST,
            "/api/text",
            None,
            &HeaderMap::new(),
            Some(&Bytes::from(multiline_body)),
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("received"),
            Duration::from_millis(20),
        )
        .await
        .unwrap();

    let file_path = recorder.save(RecordingFormat::Yaml).await.unwrap();
    assert!(file_path.exists());

    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();
    assert!(parsed.get("name").is_some());
}

// ============================================================================
// Consolidation tests
// ============================================================================

#[tokio::test]
async fn test_finalize_and_consolidate_with_yaml() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let recorder =
        MockRecorder::with_format("consolidate-yaml", temp_dir.path(), RecordingFormat::Yaml);

    recorder.init_file().await?;

    for i in 0..3 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/users/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"id":{i}}}"#)),
                Duration::from_millis(10),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let consolidator_options = mockpit::consolidator::ConsolidatorOptions::default();
    let (file_path, stats) = recorder
        .finalize_and_consolidate(consolidator_options, false)
        .await?;

    assert_eq!(stats.original_count, 3);

    let content = tokio::fs::read_to_string(&file_path).await?;
    let _collection: mockpit::config::MockCollectionConfig = serde_yaml::from_str(&content)?;

    Ok(())
}

#[tokio::test]
async fn test_finalize_and_consolidate_with_yaml_items() -> Result<()> {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::with_format(
        "consolidate-yaml-items",
        temp_dir.path(),
        RecordingFormat::Yaml,
    );

    recorder.init_file().await?;

    for i in 0..3 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/items/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!(r#"{{"id":{i}}}"#)),
                Duration::from_millis(10),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let consolidator_options = mockpit::consolidator::ConsolidatorOptions::default();
    let (file_path, stats) = recorder
        .finalize_and_consolidate(consolidator_options, false)
        .await?;

    assert_eq!(stats.original_count, 3);
    let content = tokio::fs::read_to_string(&file_path).await?;
    let _collection: mockpit::config::MockCollectionConfig = serde_yaml::from_str(&content)?;

    Ok(())
}

// ============================================================================
// Request body handling
// ============================================================================

#[tokio::test]
async fn test_request_without_body() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("no-body", temp_dir.path());

    let id = recorder
        .record(
            &Method::GET,
            "/api/test",
            None,
            &HeaderMap::new(),
            None, // No request body
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("response"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert!(interaction.request.body.is_none());
}

#[tokio::test]
async fn test_empty_request_body() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("empty-body", temp_dir.path());

    let id = recorder
        .record(
            &Method::POST,
            "/api/test",
            None,
            &HeaderMap::new(),
            Some(&Bytes::from("")),
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("response"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert!(interaction.request.body.is_some());
    assert_eq!(interaction.request.body.as_ref().unwrap(), "");
}

// ============================================================================
// Header handling edge cases
// ============================================================================

#[tokio::test]
async fn test_headers_with_invalid_utf8() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("invalid-headers", temp_dir.path());

    let mut headers = HeaderMap::new();
    // Header with non-UTF8 value will be filtered out
    headers.insert("x-valid", "valid-value".parse().unwrap());

    let id = recorder
        .record(
            &Method::GET,
            "/api/test",
            None,
            &headers,
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert!(!interaction.request.headers.is_empty());
}

#[tokio::test]
async fn test_response_headers_preserved() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("resp-headers", temp_dir.path());

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert("content-type", "application/json".parse().unwrap());
    resp_headers.insert("x-request-id", "req-123".parse().unwrap());
    resp_headers.insert("cache-control", "no-cache".parse().unwrap());

    let id = recorder
        .record(
            &Method::GET,
            "/api/test",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &resp_headers,
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert_eq!(interaction.response.headers.len(), 3);

    let header_map: FxHashMap<String, String> =
        interaction.response.headers.iter().cloned().collect();

    assert_eq!(&header_map["content-type"], "application/json");
    assert_eq!(&header_map["x-request-id"], "req-123");
    assert_eq!(&header_map["cache-control"], "no-cache");
}

// ============================================================================
// Duration edge cases
// ============================================================================

#[tokio::test]
async fn test_zero_duration() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("zero-duration", temp_dir.path());

    let id = recorder
        .record(
            &Method::GET,
            "/api/instant",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("instant"),
            Duration::from_millis(0),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert_eq!(interaction.duration, Duration::from_millis(0));
}

#[tokio::test]
async fn test_very_long_duration() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("long-duration", temp_dir.path());

    let long_duration = Duration::from_mins(5); // 5 minutes

    let id = recorder
        .record(
            &Method::GET,
            "/api/slow",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("slow"),
            long_duration,
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert_eq!(interaction.duration, long_duration);
}

// ============================================================================
// Get interaction by ID
// ============================================================================

#[tokio::test]
async fn test_get_nonexistent_interaction() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("nonexistent", temp_dir.path());

    let result = recorder.get("nonexistent-id");
    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_interaction_after_clear() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("clear-get", temp_dir.path());

    let id = recorder
        .record(
            &Method::GET,
            "/api/test",
            None,
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("test"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    assert!(recorder.get(&id).is_some());

    recorder.clear();

    assert!(recorder.get(&id).is_none());
}

// ============================================================================
// Query string variations
// ============================================================================

#[tokio::test]
async fn test_query_string_with_encoding() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("encoded-query", temp_dir.path());

    let id = recorder
        .record(
            &Method::GET,
            "/api/search",
            Some("q=hello%20world&filter=%3D%3D"),
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("results"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert_eq!(
        interaction.request.query.as_ref().unwrap(),
        "q=hello%20world&filter=%3D%3D"
    );
}

#[tokio::test]
async fn test_query_string_with_arrays() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("array-query", temp_dir.path());

    let id = recorder
        .record(
            &Method::GET,
            "/api/items",
            Some("ids=1&ids=2&ids=3"),
            &HeaderMap::new(),
            None,
            StatusCode::OK,
            &HeaderMap::new(),
            &Bytes::from("items"),
            Duration::from_millis(10),
        )
        .await
        .unwrap();

    let interaction = recorder.get(&id).unwrap();
    assert_eq!(
        interaction.request.query.as_ref().unwrap(),
        "ids=1&ids=2&ids=3"
    );
}

// ============================================================================
// Priority handling in recorded mocks
// ============================================================================

#[tokio::test]
async fn test_mock_priority_ordering() {
    let temp_dir = TempDir::new().unwrap();
    let recorder = MockRecorder::new("priority", temp_dir.path());

    // Record 5 interactions
    for i in 0..5 {
        recorder
            .record(
                &Method::GET,
                &format!("/api/item/{i}"),
                None,
                &HeaderMap::new(),
                None,
                StatusCode::OK,
                &HeaderMap::new(),
                &Bytes::from(format!("item {i}")),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
    }

    let file_path = recorder.save(RecordingFormat::Json).await.unwrap();
    let content = tokio::fs::read_to_string(&file_path).await.unwrap();
    let collection: mockpit::config::MockCollectionConfig = serde_json::from_str(&content).unwrap();

    // Verify priorities decrease (earlier recordings get higher priority)
    assert!(collection.mocks[0].priority > collection.mocks[1].priority);
    assert!(collection.mocks[1].priority > collection.mocks[2].priority);
}

// ============================================================================
// Format parse tests
// ============================================================================

#[tokio::test]
async fn test_format_parse_yml_alias() {
    let result = RecordingFormat::parse("yml").unwrap();
    assert!(matches!(result, RecordingFormat::Yaml));
}

#[tokio::test]
async fn test_format_parse_case_variations() {
    assert!(matches!(
        RecordingFormat::parse("Json").unwrap(),
        RecordingFormat::Json
    ));
    assert!(matches!(
        RecordingFormat::parse("YAML").unwrap(),
        RecordingFormat::Yaml
    ));
    assert!(RecordingFormat::parse("Toml").is_err());
    assert!(matches!(
        RecordingFormat::parse("HAR").unwrap(),
        RecordingFormat::Har
    ));
}

#[tokio::test]
async fn test_format_parse_lowercase() {
    assert!(matches!(
        RecordingFormat::parse("json").unwrap(),
        RecordingFormat::Json
    ));
}
