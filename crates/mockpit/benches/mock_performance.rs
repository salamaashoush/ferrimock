#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap, Method, StatusCode};
use mockpit::engine::types::{
    BodySource, MockDefinition, RequestMatcher, ResponseGenerator, UrlPattern,
};
use mockpit::engine::{MockMatcher, MockRegistry, ResponseGeneratorExt};
use smallvec::smallvec;
use std::hint::black_box;
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// Test Data Setup
// ============================================================================

fn create_simple_mock(id: &str, path: &str) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
            once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact(path)],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"status": "ok"}"#)),
        vars: None,
    }
}

fn create_regex_mock(id: &str, pattern: &str) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
            once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET, Method::POST],
            url_patterns: smallvec![UrlPattern::regex(pattern).unwrap()],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"status": "ok"}"#)),
        vars: None,
    }
}

fn create_complex_mock(id: &str) -> MockDefinition {
    use mockpit::engine::types::{BodyMatcher, HeaderMatcher};

    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
            once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::POST],
            url_patterns: smallvec![UrlPattern::regex(r"^/api/v\d+/users/\d+$").unwrap()],
            header_matchers: smallvec![
                HeaderMatcher::exact(HeaderName::from_static("content-type"), "application/json"),
                HeaderMatcher::present(HeaderName::from_static("authorization")),
            ],
            query_matchers: smallvec![],
            body_matcher: Some(BodyMatcher::contains("email")),
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"status": "ok"}"#)),
        vars: None,
    }
}

fn create_file_mock(id: &str, file_path: PathBuf) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
            once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/file")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::file(file_path)),
        vars: None,
    }
}

fn create_template_mock(id: &str) -> MockDefinition {
    MockDefinition {
        id: id.into(),
        priority: 100,
        enabled: true,
            once: false,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("/api/template")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::template(
                r#"{"timestamp": "{{ now() }}", "random": "{{ get_random(start=1, end=101) }}"}"#,
            ),
        ),
        vars: None,
    }
}

// ============================================================================
// Benchmark 1: Simple Mock Matching (Exact Path)
// ============================================================================

fn bench_simple_exact_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("simple_exact_match");
    group.significance_level(0.05).sample_size(1000);

    let registry = MockRegistry::new();
    registry.add_mock(create_simple_mock("test1", "/api/users"));
    registry.add_mock(create_simple_mock("test2", "/api/files"));
    registry.add_mock(create_simple_mock("test3", "/api/folders"));

    let matcher = MockMatcher::new(registry);
    let headers = HeaderMap::new();

    group.bench_function("exact_match_first", |b| {
        b.iter(|| {
            matcher.find_match(
                black_box(&Method::GET),
                black_box("/api/users"),
                None,
                black_box(&headers),
                None,
            )
        });
    });

    group.bench_function("exact_match_last", |b| {
        b.iter(|| {
            matcher.find_match(
                black_box(&Method::GET),
                black_box("/api/folders"),
                None,
                black_box(&headers),
                None,
            )
        });
    });

    group.bench_function("exact_no_match", |b| {
        b.iter(|| {
            matcher.find_match(
                black_box(&Method::GET),
                black_box("/api/nonexistent"),
                None,
                black_box(&headers),
                None,
            )
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 2: Complex Mock Matching (Regex + Headers + Body)
// ============================================================================

fn bench_complex_match(c: &mut Criterion) {
    let mut group = c.benchmark_group("complex_match");
    group.significance_level(0.05).sample_size(500);

    let registry = MockRegistry::new();
    registry.add_mock(create_complex_mock("complex1"));

    let matcher = MockMatcher::new(registry);

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        HeaderName::from_static("authorization"),
        HeaderValue::from_static("Bearer token123"),
    );

    let body = br#"{"email": "test@example.com"}"#;

    group.bench_function("regex_headers_body_match", |b| {
        b.iter(|| {
            matcher.find_match(
                black_box(&Method::POST),
                black_box("/api/v1/users/123"),
                None,
                black_box(&headers),
                black_box(Some(body)),
            )
        });
    });

    group.finish();
}

// ============================================================================
// Benchmark 3: Pattern Matching with Multiple Mocks
// ============================================================================

fn bench_pattern_matching_scale(c: &mut Criterion) {
    let mut group = c.benchmark_group("pattern_matching_scale");
    group.significance_level(0.05).sample_size(500);

    for size in &[10, 50, 100, 200] {
        let registry = MockRegistry::new();

        // Add many mocks with different patterns
        for i in 0..*size {
            registry.add_mock(create_regex_mock(
                &format!("mock{i}"),
                &format!(r"^/api/endpoint{i}(/.*)?$"),
            ));
        }

        let matcher = MockMatcher::new(registry);
        let headers = HeaderMap::new();

        group.bench_with_input(BenchmarkId::new("mocks", size), size, |b, _| {
            b.iter(|| {
                matcher.find_match(
                    black_box(&Method::GET),
                    black_box(&format!("/api/endpoint{}/test", size - 1)), // Match last
                    None,
                    black_box(&headers),
                    None,
                )
            });
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark 3b: Miss Performance at Scale (isolate where time goes)
// ============================================================================

fn bench_miss_at_scale(c: &mut Criterion) {
    let mut group = c.benchmark_group("miss_at_scale");
    group.significance_level(0.05).sample_size(500);

    // Test misses with different mock counts and pattern types
    for size in &[10, 50, 100, 200, 500] {
        // Regex mocks (Express-style patterns compiled to regex)
        let registry = MockRegistry::new();
        for i in 0..*size {
            registry.add_mock(create_regex_mock(
                &format!("regex-{i}"),
                &format!(r"^/api/users/(?P<id>[^/]+)/endpoint{i}$"),
            ));
        }
        let matcher = MockMatcher::new(registry);
        let headers = HeaderMap::new();

        group.bench_with_input(BenchmarkId::new("miss_regex", size), size, |b, _| {
            b.iter(|| {
                matcher.find_match(
                    black_box(&Method::GET),
                    black_box("/completely/different/path/no/match"),
                    None,
                    black_box(&headers),
                    None,
                )
            });
        });
    }

    // Exact mocks (cheapest pattern type)
    for size in &[10, 50, 100, 200, 500] {
        let registry = MockRegistry::new();
        for i in 0..*size {
            registry.add_mock(create_simple_mock(
                &format!("exact-{i}"),
                &format!("/api/exact/endpoint/{i}"),
            ));
        }
        let matcher = MockMatcher::new(registry);
        let headers = HeaderMap::new();

        group.bench_with_input(BenchmarkId::new("miss_exact", size), size, |b, _| {
            b.iter(|| {
                matcher.find_match(
                    black_box(&Method::GET),
                    black_box("/completely/different/path/no/match"),
                    None,
                    black_box(&headers),
                    None,
                )
            });
        });
    }

    // Mixed methods (GET, POST, PUT, DELETE, PATCH) to see method filtering effect
    for size in &[100, 500] {
        let registry = MockRegistry::new();
        let methods = [
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
        ];
        for i in 0..*size {
            let method = methods[i % methods.len()].clone();
            let mock = MockDefinition {
                id: format!("mixed-{i}").into(),
                priority: 100,
                enabled: true,
            once: false,
                scope: None,
                source_file: None,
                request_transforms: None,
                request: RequestMatcher {
                    methods: smallvec![method],
                    url_patterns: smallvec![
                        UrlPattern::regex(&format!(r"^/api/endpoint{i}(/.*)?$")).unwrap()
                    ],
                    header_matchers: smallvec![],
                    query_matchers: smallvec![],
                    body_matcher: None,
                    graphql_matcher: None,
                },
                response: ResponseGenerator::new(
                    StatusCode::OK,
                    BodySource::inline(r#"{"status": "ok"}"#),
                ),
                vars: None,
            };
            registry.add_mock(mock);
        }
        let matcher = MockMatcher::new(registry);
        let headers = HeaderMap::new();

        group.bench_with_input(
            BenchmarkId::new("miss_mixed_methods", size),
            size,
            |b, _| {
                b.iter(|| {
                    matcher.find_match(
                        black_box(&Method::GET),
                        black_box("/completely/different/path/no/match"),
                        None,
                        black_box(&headers),
                        None,
                    )
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark 3c: Isolate individual costs (get_enabled_mocks clone, method check, url check)
// ============================================================================

fn bench_isolate_costs(c: &mut Criterion) {
    let mut group = c.benchmark_group("isolate_costs");
    group.significance_level(0.05).sample_size(500);

    for size in &[100, 500] {
        // Setup
        let registry = MockRegistry::new();
        for i in 0..*size {
            registry.add_mock(create_regex_mock(
                &format!("isolate-{i}"),
                &format!(r"^/api/endpoint{i}(/.*)?$"),
            ));
        }

        // Cost of get_enabled_mocks alone (Vec clone)
        group.bench_with_input(BenchmarkId::new("get_enabled_mocks", size), size, |b, _| {
            b.iter(|| {
                black_box(registry.get_enabled_mocks());
            });
        });

        // Cost of iterating + method check only
        group.bench_with_input(BenchmarkId::new("iter_method_check", size), size, |b, _| {
            let method = Method::GET;
            b.iter(|| {
                let mocks = registry.get_enabled_mocks();
                let count = mocks
                    .iter()
                    .filter(|m| m.request.methods.is_empty() || m.request.methods.contains(&method))
                    .count();
                black_box(count);
            });
        });

        // Cost of iterating + method + URL check (regex is_match)
        let _matcher = MockMatcher::new(registry.clone());
        group.bench_with_input(
            BenchmarkId::new("iter_method_url_check", size),
            size,
            |b, _| {
                let method = Method::GET;
                let path = "/completely/different/path/no/match";
                b.iter(|| {
                    let mocks = registry.get_enabled_mocks();
                    let found = mocks.iter().find(|m| {
                        (m.request.methods.is_empty() || m.request.methods.contains(&method))
                            && m.request.url_patterns.iter().any(|p| p.matches(path))
                    });
                    black_box(found);
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Benchmark 4: Response Generation - Static Inline
// ============================================================================

fn bench_context_construction(c: &mut Criterion) {
    use mockpit::types::RequestContext;
    let mut group = c.benchmark_group("context_construction");
    group.significance_level(0.05).sample_size(1000);

    // Realistic request: 10 headers + a JSON body.
    let mut headers = HeaderMap::new();
    for (k, v) in [
        ("host", "api.example.com"),
        ("accept", "application/json"),
        ("accept-encoding", "gzip, deflate, br"),
        ("user-agent", "mockpit-bench/1.0"),
        ("authorization", "Bearer abc123"),
        ("content-type", "application/json"),
        ("x-request-id", "req-1234567890"),
        ("cache-control", "no-cache"),
        ("connection", "keep-alive"),
        ("x-forwarded-for", "10.0.0.1"),
    ] {
        headers.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
    }
    let body = br#"{"name":"John","email":"john@example.com","age":30}"#;

    // Full materialization (template references headers + body).
    group.bench_function("full_headers_body", |b| {
        b.iter(|| {
            black_box(RequestContext::from_request(
                "POST",
                "/api/users",
                None,
                black_box(&headers),
                Some(black_box(body)),
            ))
        });
    });

    // Lazy: template references neither headers nor body.
    group.bench_function("lazy_skip_headers_body", |b| {
        b.iter(|| {
            black_box(RequestContext::from_request_selective(
                "POST",
                "/api/users",
                None,
                black_box(&headers),
                Some(black_box(body)),
                false,
                false,
            ))
        });
    });

    group.finish();
}

fn bench_static_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("static_response_generation");
    group.significance_level(0.05).sample_size(1000);

    let small_response =
        ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"status": "ok"}"#));

    let medium_response = ResponseGenerator::new(
        StatusCode::OK,
        BodySource::inline(format!(r#"{{"data": [{}]}}"#, "1,".repeat(100))),
    );

    let large_response = ResponseGenerator::new(
        StatusCode::OK,
        BodySource::inline(format!(r#"{{"data": [{}]}}"#, "1,".repeat(1000))),
    );

    group.bench_function("small_20B", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async { small_response.generate().await.unwrap() });
    });

    group.bench_function("medium_500B", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async { medium_response.generate().await.unwrap() });
    });

    group.bench_function("large_5KB", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async { large_response.generate().await.unwrap() });
    });

    group.finish();
}

// ============================================================================
// Benchmark 5: Response Generation - File-Based
// ============================================================================

fn bench_file_response(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_response_generation");
    group.significance_level(0.05).sample_size(200);

    // Create temp files with different sizes
    let temp_dir = TempDir::new().unwrap();

    let small_file = temp_dir.path().join("small.json");
    std::fs::write(&small_file, r#"{"status": "ok"}"#).unwrap();

    let medium_file = temp_dir.path().join("medium.json");
    std::fs::write(
        &medium_file,
        format!(r#"{{"data": [{}]}}"#, "1,".repeat(1000)),
    )
    .unwrap();

    let large_file = temp_dir.path().join("large.json");
    std::fs::write(
        &large_file,
        format!(r#"{{"data": [{}]}}"#, "1,".repeat(10000)),
    )
    .unwrap();

    let small_mock = create_file_mock("small", small_file);
    let medium_mock = create_file_mock("medium", medium_file);
    let large_mock = create_file_mock("large", large_file);

    group.bench_function("file_small_20B", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async { small_mock.response.generate().await.unwrap() });
    });

    group.bench_function("file_medium_5KB", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async { medium_mock.response.generate().await.unwrap() });
    });

    group.bench_function("file_large_50KB", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async { large_mock.response.generate().await.unwrap() });
    });

    group.finish();
}

// ============================================================================
// Benchmark 6: Response Generation - Template
// ============================================================================

fn bench_template_response(c: &mut Criterion) {
    use mockpit::engine::RequestContext;

    let mut group = c.benchmark_group("template_response_generation");
    group.significance_level(0.05).sample_size(500);

    // Use the create_template_mock function
    let template_mock = create_template_mock("template-bench");
    let context = RequestContext::new();

    group.bench_function("template_with_functions", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let resp = template_mock.response.clone();
                let ctx = context.clone();
                async move { resp.generate_with_context(&ctx).await.unwrap() }
            });
    });

    group.finish();
}

// ============================================================================
// Benchmark 7: Throughput Test - Concurrent Requests
// ============================================================================

fn bench_concurrent_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_throughput");
    group.significance_level(0.05).sample_size(100);
    group.throughput(Throughput::Elements(100));

    let registry = MockRegistry::new();
    registry.add_mock(create_simple_mock("test", "/api/test"));
    let matcher = MockMatcher::new(registry);
    let headers = HeaderMap::new();

    group.bench_function("100_concurrent_simple_matches", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| async {
                let mut handles = vec![];
                for _ in 0..100 {
                    let m = matcher.clone();
                    let h = headers.clone();
                    let handle = tokio::spawn(async move {
                        m.find_match(&Method::GET, "/api/test", None, &h, None)
                    });
                    handles.push(handle);
                }
                futures::future::join_all(handles).await
            });
    });

    group.finish();
}

// ============================================================================
// Benchmark 8: Registry Operations
// ============================================================================

fn bench_registry_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("registry_operations");
    group.significance_level(0.05).sample_size(500);

    group.bench_function("add_mock", |b| {
        let registry = MockRegistry::new();
        let mut counter = 0;
        b.iter(|| {
            registry.add_mock(create_simple_mock(&format!("mock{counter}"), "/api/test"));
            counter += 1;
        });
    });

    group.bench_function("get_enabled_mocks_100", |b| {
        let registry = MockRegistry::new();
        for i in 0..100 {
            registry.add_mock(create_simple_mock(&format!("mock{i}"), "/api/test"));
        }
        b.iter(|| registry.get_enabled_mocks());
    });

    group.finish();
}

// ============================================================================
// Benchmark 9: Response Patcher - JSON Batching
// ============================================================================

fn bench_response_patcher(c: &mut Criterion) {
    use bytes::Bytes;
    use http::Response;
    use mockpit::engine::ResponsePatcher;
    use mockpit::engine::types::PatchOperation;

    let mut group = c.benchmark_group("response_patcher");
    group.significance_level(0.05).sample_size(500);

    // Create a realistic JSON body
    let json_body = serde_json::json!({
      "id": "12345",
      "type": "file",
      "name": "test-document.pdf",
      "size": 1024,
      "created_at": "2024-01-01T00:00:00Z",
      "modified_at": "2024-01-01T00:00:00Z",
      "created_by": {
        "id": "user-1",
        "name": "Alice",
        "login": "alice@example.com"
      },
      "modified_by": {
        "id": "user-2",
        "name": "Bob",
        "login": "bob@example.com"
      },
      "tags": ["important", "review"],
      "permissions": {
        "can_download": true,
        "can_preview": true,
        "can_delete": false
      }
    });
    let body_bytes = Bytes::from(serde_json::to_vec(&json_body).unwrap());

    // Single JSONPath patch
    let single_patch = vec![PatchOperation::JsonPath {
        path: "$.created_by.name".to_string(),
        value: serde_json::json!("Charlie"),
    }];

    // Multiple JSONPath patches (benefits from batching)
    let multi_patch = vec![
        PatchOperation::JsonPath {
            path: "$.created_by.name".to_string(),
            value: serde_json::json!("Charlie"),
        },
        PatchOperation::JsonPath {
            path: "$.modified_by.name".to_string(),
            value: serde_json::json!("Diana"),
        },
        PatchOperation::JsonPath {
            path: "$.permissions.can_delete".to_string(),
            value: serde_json::json!(true),
        },
        PatchOperation::JsonPath {
            path: "$.name".to_string(),
            value: serde_json::json!("renamed-document.pdf"),
        },
    ];

    // Mixed operations: JSON + header patches
    let mixed_patch = vec![
        PatchOperation::JsonPath {
            path: "$.created_by.name".to_string(),
            value: serde_json::json!("Charlie"),
        },
        PatchOperation::JsonPath {
            path: "$.permissions.can_delete".to_string(),
            value: serde_json::json!(true),
        },
        PatchOperation::HeaderAdd {
            name: "x-custom-header".to_string(),
            value: "patched".to_string(),
        },
    ];

    group.bench_function("single_jsonpath", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let patcher = ResponsePatcher::new(single_patch.clone());
                let response = Response::builder()
                    .status(StatusCode::OK)
                    .body(body_bytes.clone())
                    .unwrap();
                async move { patcher.apply(response, None).unwrap() }
            });
    });

    group.bench_function("four_jsonpath_batched", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let patcher = ResponsePatcher::new(multi_patch.clone());
                let response = Response::builder()
                    .status(StatusCode::OK)
                    .body(body_bytes.clone())
                    .unwrap();
                async move { patcher.apply(response, None).unwrap() }
            });
    });

    group.bench_function("mixed_json_and_headers", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let patcher = ResponsePatcher::new(mixed_patch.clone());
                let response = Response::builder()
                    .status(StatusCode::OK)
                    .body(body_bytes.clone())
                    .unwrap();
                async move { patcher.apply(response, None).unwrap() }
            });
    });

    group.finish();
}

// ============================================================================
// Benchmark 10: Structured vs Non-Structured Template Response
// ============================================================================

fn bench_structured_response(c: &mut Criterion) {
    use mockpit::engine::RequestContext;
    use rustc_hash::FxHashMap;

    let mut group = c.benchmark_group("structured_response_detection");
    group.significance_level(0.05).sample_size(500);

    let ctx = RequestContext {
        method: "GET".to_string(),
        uri: "/api/users/123".to_string(),
        path: "/api/users/123".to_string(),
        captures: {
            let mut m = FxHashMap::default();
            m.insert("id".to_string(), "123".to_string());
            m
        },
        query: FxHashMap::default(),
        headers: FxHashMap::default(),
        body: None,
        body_json: None,
        vars: None,
    };

    // Plain template (no structured response parsing needed — skips JSON parse)
    let plain_response = ResponseGenerator::new(
        StatusCode::OK,
        BodySource::template(r#"{"id": "{{ captures.id }}", "name": "{{ fake_name() }}"}"#),
    );

    // Structured template (triggers JSON parse for status/headers extraction)
    let structured_response = ResponseGenerator::new(
        StatusCode::CREATED,
        BodySource::template(
            r#"{"status": 201, "headers": {"X-Custom": "value"}, "body": {"id": "{{ captures.id }}"}}"#,
        ),
    );

    group.bench_function("plain_template_skip_parse", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let resp = plain_response.clone();
                let ctx = ctx.clone();
                async move { resp.generate_with_context(&ctx).await.unwrap() }
            });
    });

    group.bench_function("structured_template_with_parse", |b| {
        b.to_async(tokio::runtime::Runtime::new().unwrap())
            .iter(|| {
                let resp = structured_response.clone();
                let ctx = ctx.clone();
                async move { resp.generate_with_context(&ctx).await.unwrap() }
            });
    });

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    benches,
    bench_simple_exact_match,
    bench_complex_match,
    bench_pattern_matching_scale,
    bench_miss_at_scale,
    bench_isolate_costs,
    bench_context_construction,
    bench_static_response,
    bench_file_response,
    bench_template_response,
    bench_concurrent_throughput,
    bench_registry_operations,
    bench_response_patcher,
    bench_structured_response,
);

criterion_main!(benches);
