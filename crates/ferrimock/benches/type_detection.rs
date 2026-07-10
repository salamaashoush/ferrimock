use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ferrimock::type_detector::{TypeDetector, features};
use serde_json::{Value as JsonValue, json};
use std::hint::black_box;

// ============================================================================
// Helper Functions
// ============================================================================

fn as_refs(values: &[JsonValue]) -> Vec<&JsonValue> {
    values.iter().collect()
}

fn generate_sample_values<F>(count: usize, generator: F) -> Vec<JsonValue>
where
    F: Fn(usize) -> JsonValue,
{
    (0..count).map(generator).collect()
}

// ============================================================================
// Benchmark 1: Single Field Type Detection (Various Types)
// ============================================================================

fn bench_single_field_url(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("https://example.com/api/v1/users"),
        json!("https://api.github.com/repos/owner/repo"),
        json!("https://sign-stg.example.com/api/v1/documents"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/url", |b| {
        b.iter(|| detector.detect_type(black_box("next"), black_box(&value_refs)));
    });
}

fn bench_single_field_uuid(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("550e8400-e29b-41d4-a716-446655440000"),
        json!("6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
        json!("f47ac10b-58cc-4372-a567-0e02b2c3d479"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/uuid", |b| {
        b.iter(|| detector.detect_type(black_box("id"), black_box(&value_refs)));
    });
}

fn bench_single_field_email(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("user@example.com"),
        json!("admin@test.org"),
        json!("contact@company.co.uk"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/email", |b| {
        b.iter(|| detector.detect_type(black_box("email"), black_box(&value_refs)));
    });
}

fn bench_single_field_numeric_string_id(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("23930793379"),
        json!("12345678901234"),
        json!("98765432109876"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/numeric_string_id", |b| {
        b.iter(|| detector.detect_type(black_box("user_id"), black_box(&value_refs)));
    });
}

fn bench_single_field_etag(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![json!("123"), json!("456"), json!("\"789\"")];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/etag", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&value_refs)));
    });
}

fn bench_single_field_timestamp(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("2024-01-15T10:30:00Z"),
        json!("2024-01-16T14:22:33Z"),
        json!("2024-01-17T08:15:42Z"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/timestamp", |b| {
        b.iter(|| detector.detect_type(black_box("created_at"), black_box(&value_refs)));
    });
}

fn bench_single_field_filename(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("document.pdf"),
        json!("image.jpg"),
        json!("report.xlsx"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/filename", |b| {
        b.iter(|| detector.detect_type(black_box("file_name"), black_box(&value_refs)));
    });
}

fn bench_single_field_ip_address(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![json!("192.168.1.1"), json!("10.0.0.1"), json!("172.16.0.1")];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/ip_address", |b| {
        b.iter(|| detector.detect_type(black_box("ip"), black_box(&value_refs)));
    });
}

fn bench_single_field_semver(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![json!("1.0.0"), json!("2.3.4"), json!("3.0.0-beta.1")];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/semver", |b| {
        b.iter(|| detector.detect_type(black_box("version"), black_box(&value_refs)));
    });
}

fn bench_single_field_base64(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("SGVsbG8gV29ybGQhIFRoaXMgaXMgYSBiYXNlNjQgZW5jb2RlZCBzdHJpbmc="),
        json!("VGhpcyBpcyBhbm90aGVyIGJhc2U2NCBzdHJpbmcgd2l0aCBtb3JlIGNvbnRlbnQ="),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/base64", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&value_refs)));
    });
}

fn bench_single_field_mime_type(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("application/json"),
        json!("text/html"),
        json!("image/png"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("single_field/mime_type", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&value_refs)));
    });
}

// ============================================================================
// Benchmark 2: Multi-Sample Detection (10, 100, 500 samples)
// ============================================================================

fn bench_multi_sample_url(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let mut group = c.benchmark_group("multi_sample/url");

    for &size in &[10, 50, 100, 500] {
        let values =
            generate_sample_values(size, |i| json!(format!("https://example.com/page/{}", i)));
        let value_refs = as_refs(&values);

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| detector.detect_type(black_box("next"), black_box(&value_refs)));
        });
    }

    group.finish();
}

fn bench_multi_sample_uuid(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let mut group = c.benchmark_group("multi_sample/uuid");

    for &size in &[10, 50, 100, 500] {
        let values = generate_sample_values(size, |i| {
            json!(format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
                i * 1000,
                i % 10000,
                i % 10000,
                i % 10000,
                i * 100_000
            ))
        });
        let value_refs = as_refs(&values);

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| detector.detect_type(black_box("id"), black_box(&value_refs)));
        });
    }

    group.finish();
}

fn bench_multi_sample_email(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let mut group = c.benchmark_group("multi_sample/email");

    for &size in &[10, 50, 100, 500] {
        let values = generate_sample_values(size, |i| json!(format!("user{}@example.com", i)));
        let value_refs = as_refs(&values);

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| detector.detect_type(black_box("email"), black_box(&value_refs)));
        });
    }

    group.finish();
}

fn bench_multi_sample_numeric_id(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let mut group = c.benchmark_group("multi_sample/numeric_id");

    for &size in &[10, 50, 100, 500] {
        let values =
            generate_sample_values(size, |i| json!(format!("{:015}", i + 1_000_000_000_000)));
        let value_refs = as_refs(&values);

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| detector.detect_type(black_box("user_id"), black_box(&value_refs)));
        });
    }

    group.finish();
}

fn bench_multi_sample_timestamp(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let mut group = c.benchmark_group("multi_sample/timestamp");

    for &size in &[10, 50, 100, 500] {
        let values = generate_sample_values(size, |i| {
            json!(format!("2024-01-{:02}T10:30:00Z", (i % 28) + 1))
        });
        let value_refs = as_refs(&values);

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| detector.detect_type(black_box("created_at"), black_box(&value_refs)));
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark 3: Semantic Context Performance
// ============================================================================

fn bench_semantic_context_with_hint(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("https://example.com/page1"),
        json!("https://example.com/page2"),
        json!("https://example.com/page3"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("semantic_context/with_hint", |b| {
        b.iter(|| detector.detect_type(black_box("next"), black_box(&value_refs)));
    });
}

fn bench_semantic_context_without_hint(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("https://example.com/page1"),
        json!("https://example.com/page2"),
        json!("https://example.com/page3"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("semantic_context/without_hint", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&value_refs)));
    });
}

fn bench_semantic_context_id_field(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!("12345678901234"),
        json!("98765432109876"),
        json!("11111111111111"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("semantic_context/id_field_hint", |b| {
        b.iter(|| detector.detect_type(black_box("user_id"), black_box(&value_refs)));
    });
}

// ============================================================================
// Benchmark 4: Statistical Feature Extraction
// ============================================================================

fn bench_feature_extraction_short_strings(c: &mut Criterion) {
    let values: Vec<&str> = vec!["test1", "test2", "test3"];

    c.bench_function("feature_extraction/short_strings", |b| {
        b.iter(|| features::extract_features(black_box(&values)));
    });
}

fn bench_feature_extraction_long_strings(c: &mut Criterion) {
    let values: Vec<&str> = vec![
        "https://example.com/api/v1/users/123/documents/456?page=1&limit=50",
        "https://example.com/api/v1/users/789/documents/012?page=2&limit=50",
        "https://example.com/api/v1/users/345/documents/678?page=3&limit=50",
    ];

    c.bench_function("feature_extraction/long_strings", |b| {
        b.iter(|| features::extract_features(black_box(&values)));
    });
}

fn bench_feature_extraction_many_samples(c: &mut Criterion) {
    let mut group = c.benchmark_group("feature_extraction/sample_count");

    for &size in &[10, 50, 100, 500] {
        let values: Vec<String> = (0..size)
            .map(|i| format!("https://example.com/page/{i}"))
            .collect();
        let value_refs: Vec<&str> = values.iter().map(String::as_str).collect();

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| features::extract_features(black_box(&value_refs)));
        });
    }

    group.finish();
}

// ============================================================================
// Benchmark 5: Shannon Entropy Calculation
// ============================================================================

fn bench_shannon_entropy(c: &mut Criterion) {
    let mut group = c.benchmark_group("shannon_entropy");

    // Low entropy (repetitive)
    let low_entropy_values: Vec<&str> = vec!["aaaaaaa", "aaaaaab", "aaaaaac"];
    group.bench_function("low_entropy", |b| {
        b.iter(|| features::extract_features(black_box(&low_entropy_values)));
    });

    // High entropy (random-looking)
    let high_entropy_values: Vec<&str> =
        vec!["a1B2c3D4e5F6g7H8", "x9Y8z7W6v5U4t3S2", "m1N2o3P4q5R6s7T8"];
    group.bench_function("high_entropy", |b| {
        b.iter(|| features::extract_features(black_box(&high_entropy_values)));
    });

    group.finish();
}

// ============================================================================
// Benchmark 6: Real-World Data Patterns
// ============================================================================

fn bench_real_world_api_response(c: &mut Criterion) {
    let detector = TypeDetector::new();

    // Simulate a typical API response field set
    let url_values = vec![
        json!("https://sign-stg.dev.example.com/api/v1/documents-search/?page=2"),
        json!("https://sign-stg.dev.example.com/api/v1/documents-search/?page=3"),
    ];
    let id_values = vec![json!("23930793379"), json!("23930793380")];
    let timestamp_values = vec![json!("2024-01-15T10:30:00Z"), json!("2024-01-16T14:22:33Z")];
    let etag_values = vec![json!("123"), json!("456")];

    let url_refs = as_refs(&url_values);
    let id_refs = as_refs(&id_values);
    let timestamp_refs = as_refs(&timestamp_values);
    let etag_refs = as_refs(&etag_values);

    let mut group = c.benchmark_group("real_world/api_response");

    group.bench_function("url_field", |b| {
        b.iter(|| detector.detect_type(black_box("next"), black_box(&url_refs)));
    });

    group.bench_function("id_field", |b| {
        b.iter(|| detector.detect_type(black_box("id"), black_box(&id_refs)));
    });

    group.bench_function("timestamp_field", |b| {
        b.iter(|| detector.detect_type(black_box("created_at"), black_box(&timestamp_refs)));
    });

    group.bench_function("etag_field", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&etag_refs)));
    });

    group.finish();
}

fn bench_real_world_mixed_types(c: &mut Criterion) {
    let detector = TypeDetector::new();

    // Mixed type detection across multiple fields
    let fields = vec![
        (
            "next",
            vec![
                json!("https://example.com/page2"),
                json!("https://example.com/page3"),
            ],
        ),
        (
            "user_id",
            vec![json!("12345678901234"), json!("98765432109876")],
        ),
        (
            "email",
            vec![json!("user@example.com"), json!("admin@test.org")],
        ),
        (
            "created_at",
            vec![json!("2024-01-15T10:30:00Z"), json!("2024-01-16T14:22:33Z")],
        ),
        ("version", vec![json!("1.0.0"), json!("1.0.1")]),
    ];

    c.bench_function("real_world/mixed_5_fields", |b| {
        b.iter(|| {
            for (field_name, values) in &fields {
                let value_refs = as_refs(values);
                detector.detect_type(black_box(field_name), black_box(&value_refs));
            }
        });
    });
}

// ============================================================================
// Benchmark 7: Complex Type Detection
// ============================================================================

fn bench_complex_array_detection(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!(["test1@example.com", "test2@example.com"]),
        json!(["user@test.org", "admin@test.org"]),
        json!(["contact@company.co.uk"]),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("complex/array_of_emails", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&value_refs)));
    });
}

fn bench_complex_object_detection(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![
        json!({"name": "Alice", "age": 30, "email": "alice@example.com"}),
        json!({"name": "Bob", "age": 25, "email": "bob@example.com"}),
        json!({"name": "Charlie", "age": 35, "email": "charlie@example.com"}),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("complex/nested_object", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&value_refs)));
    });
}

// ============================================================================
// Benchmark 8: Edge Cases and Ambiguous Patterns
// ============================================================================

fn bench_edge_case_mixed_confidence(c: &mut Criterion) {
    let detector = TypeDetector::new();

    // 70% match - should trigger confidence thresholds
    let values = vec![
        json!("test@example.com"),
        json!("user@test.org"),
        json!("admin@company.co.uk"),
        json!("not-an-email"),
        json!("another-non-email"),
    ];
    let value_refs = as_refs(&values);

    c.bench_function("edge_case/mixed_confidence_70pct", |b| {
        b.iter(|| detector.detect_type(black_box("email"), black_box(&value_refs)));
    });
}

fn bench_edge_case_empty_values(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values: Vec<&JsonValue> = vec![];

    c.bench_function("edge_case/empty_values", |b| {
        b.iter(|| detector.detect_type_from_values(black_box(&values)));
    });
}

fn bench_edge_case_single_value(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let values = vec![json!("https://example.com/test")];
    let value_refs = as_refs(&values);

    c.bench_function("edge_case/single_value", |b| {
        b.iter(|| detector.detect_type(black_box("next"), black_box(&value_refs)));
    });
}

// ============================================================================
// Benchmark 9: End-to-End Performance
// ============================================================================

fn bench_end_to_end_full_detection(c: &mut Criterion) {
    let detector = TypeDetector::new();

    // Simulate detecting all field types in a typical API response
    let fields = vec![
        ("next", vec![json!("https://api.example.com/users?page=2")]),
        ("id", vec![json!("12345678901234")]),
        ("user_id", vec![json!("98765432109876")]),
        ("email", vec![json!("user@example.com")]),
        ("created_at", vec![json!("2024-01-15T10:30:00Z")]),
        ("updated_at", vec![json!("2024-01-16T14:22:33Z")]),
        ("version", vec![json!("1.0.0")]),
        ("file_name", vec![json!("document.pdf")]),
        ("ip", vec![json!("192.168.1.1")]),
        ("etag", vec![json!("\"123\"")]),
    ];

    c.bench_function("end_to_end/10_fields_detection", |b| {
        b.iter(|| {
            for (field_name, values) in &fields {
                let value_refs = as_refs(values);
                detector.detect_type(black_box(field_name), black_box(&value_refs));
            }
        });
    });
}

// ============================================================================
// Benchmark 10: BigQuery-Equivalent Performance (500 samples)
// ============================================================================

fn bench_bigquery_equivalent(c: &mut Criterion) {
    let detector = TypeDetector::new();
    let mut group = c.benchmark_group("bigquery_equivalent");
    group.sample_size(100); // Reduce sample size for 500-sample tests

    // URL detection with 500 samples (BigQuery's sample size)
    let url_values = generate_sample_values(500, |i| {
        json!(format!("https://example.com/api/v1/users/{}", i))
    });
    let url_refs = as_refs(&url_values);

    group.bench_function("500_urls", |b| {
        b.iter(|| detector.detect_type(black_box("next"), black_box(&url_refs)));
    });

    // UUID detection with 500 samples
    let uuid_values = generate_sample_values(500, |i| {
        json!(format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            i * 1000,
            i % 10000,
            i % 10000,
            i % 10000,
            i * 100_000
        ))
    });
    let uuid_refs = as_refs(&uuid_values);

    group.bench_function("500_uuids", |b| {
        b.iter(|| detector.detect_type(black_box("id"), black_box(&uuid_refs)));
    });

    // Email detection with 500 samples
    let email_values = generate_sample_values(500, |i| json!(format!("user{}@example.com", i)));
    let email_refs = as_refs(&email_values);

    group.bench_function("500_emails", |b| {
        b.iter(|| detector.detect_type(black_box("email"), black_box(&email_refs)));
    });

    group.finish();
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    single_field_benches,
    bench_single_field_url,
    bench_single_field_uuid,
    bench_single_field_email,
    bench_single_field_numeric_string_id,
    bench_single_field_etag,
    bench_single_field_timestamp,
    bench_single_field_filename,
    bench_single_field_ip_address,
    bench_single_field_semver,
    bench_single_field_base64,
    bench_single_field_mime_type,
);

criterion_group!(
    multi_sample_benches,
    bench_multi_sample_url,
    bench_multi_sample_uuid,
    bench_multi_sample_email,
    bench_multi_sample_numeric_id,
    bench_multi_sample_timestamp,
);

criterion_group!(
    semantic_context_benches,
    bench_semantic_context_with_hint,
    bench_semantic_context_without_hint,
    bench_semantic_context_id_field,
);

criterion_group!(
    feature_extraction_benches,
    bench_feature_extraction_short_strings,
    bench_feature_extraction_long_strings,
    bench_feature_extraction_many_samples,
    bench_shannon_entropy,
);

criterion_group!(
    real_world_benches,
    bench_real_world_api_response,
    bench_real_world_mixed_types,
);

criterion_group!(
    complex_benches,
    bench_complex_array_detection,
    bench_complex_object_detection,
);

criterion_group!(
    edge_case_benches,
    bench_edge_case_mixed_confidence,
    bench_edge_case_empty_values,
    bench_edge_case_single_value,
);

criterion_group!(end_to_end_benches, bench_end_to_end_full_detection,);

criterion_group!(bigquery_benches, bench_bigquery_equivalent,);

criterion_main!(
    single_field_benches,
    multi_sample_benches,
    semantic_context_benches,
    feature_extraction_benches,
    real_world_benches,
    complex_benches,
    edge_case_benches,
    end_to_end_benches,
    bigquery_benches,
);
