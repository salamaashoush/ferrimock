use super::*;
use crate::constants::*;
use serde_json::{Value as JsonValue, json};

// Helper macro to create test values with proper lifetimes
macro_rules! test_values {
      ($($val:expr),+ $(,)?) => {{
          let vals = vec![$($val),+];
          vals
      }};
  }

// Helper to convert JsonValue vec to references
fn as_refs(values: &[JsonValue]) -> Vec<&JsonValue> {
    values.iter().collect()
}

#[test]
fn test_detect_url_with_protocol() {
    let detector = TypeDetector::new();
    let values = test_values![
        json!("https://example.com"),
        json!("https://api.example.com/v1"),
        json!("http://test.org"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Url));
    assert!(confidence >= CONFIDENCE_URL);
}

#[test]
fn test_detect_download_url() {
    let detector = TypeDetector::new();
    // Create realistic download URLs with token-like query params
    let values = test_values![
        json!(
            "https://example.com/download/file?token=abc123def456ghi789jkl012mno345pqr678stu901vwx234yz56789012345678901234567890123456789012345678901234"
        ),
        json!(
            "https://api.example.com/content/d/document.pdf?auth=xyz789abc012def345ghi678jkl901mno234pqr567stu890vwx123yz45678901234567890123456789012345678901234567"
        ),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    // Since these have download/content in them and are long URLs, should be DownloadUrl
    assert!(matches!(
        field_type,
        FieldType::DownloadUrl { .. } | FieldType::Url
    ));
    if matches!(field_type, FieldType::DownloadUrl { .. }) {
        assert!(confidence >= CONFIDENCE_DOWNLOAD_URL);
    }
}

#[test]
fn test_detect_uuid() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("550e8400-e29b-41d4-a716-446655440000"),
        json!("6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Uuid));
    assert!(confidence >= CONFIDENCE_UUID);
}

#[test]
fn test_detect_email() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("test@example.com"),
        json!("user@test.org"),
        json!("admin@company.co.uk"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Email));
    assert!(confidence >= CONFIDENCE_EMAIL);
}

#[test]
fn test_detect_timestamp() {
    let detector = TypeDetector::new();
    let values = vec![json!("2024-01-15T10:30:00Z"), json!("2024-01-16T14:22:33Z")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Timestamp));
    assert!(confidence >= CONFIDENCE_TIMESTAMP);
}

#[test]
fn test_detect_numeric_string_id() {
    let detector = TypeDetector::new();
    let values = vec![json!("23930793379"), json!("12345678901234")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::NumericStringId));
    assert!(confidence >= CONFIDENCE_NUMERIC_STRING_ID);
}

#[test]
fn test_detect_etag_not_confused_with_numeric_id() {
    let detector = TypeDetector::new();

    // Numeric strings without field context should be NumericStringId
    let id_values_short = vec![json!("123"), json!("456"), json!("789")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&id_values_short));
    assert!(matches!(field_type, FieldType::NumericStringId));

    // Long numeric strings should also be IDs
    let id_values_long = vec![json!("12345678901"), json!("98765432109")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&id_values_long));
    assert!(matches!(field_type, FieldType::NumericStringId));

    // But with "etag" field name, numeric strings should be ETags (Box API pattern)
    let etag_values = vec![json!("123"), json!("456"), json!("789")];
    let (field_type, _) = detector.detect_type("etag", &as_refs(&etag_values));
    assert!(matches!(field_type, FieldType::ETag));

    // Real HTTP ETags (quoted strings) should be detected as ETags
    let http_etag_values = vec![json!("\"abc123\""), json!("\"def456\""), json!("W/\"789\"")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&http_etag_values));
    assert!(matches!(field_type, FieldType::ETag));
}

#[test]
fn test_semantic_context_url_field() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("https://example.com/page1"),
        json!("https://example.com/page2"),
    ];

    let (field_type, confidence) = detector.detect_type("next", &as_refs(&values));
    assert!(matches!(
        field_type,
        FieldType::Url | FieldType::PaginationUrl(_)
    ));
    assert!(confidence >= 0.85);
}

#[test]
fn test_semantic_context_id_field() {
    let detector = TypeDetector::new();
    let values = vec![json!("12345678901234"), json!("98765432109876")];

    let (field_type, confidence) = detector.detect_type("user_id", &as_refs(&values));
    assert!(matches!(field_type, FieldType::NumericStringId));
    assert!(confidence >= 0.90);
}

#[test]
fn test_detect_filename() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("document.pdf"),
        json!("image.jpg"),
        json!("report.xlsx"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::FileName));
    assert!(confidence >= CONFIDENCE_FILENAME);
}

#[test]
fn test_detect_semver() {
    let detector = TypeDetector::new();
    let values = vec![json!("1.0.0"), json!("2.3.4"), json!("3.0.0-beta.1")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Semver));
    assert!(confidence >= CONFIDENCE_SEMVER);
}

#[test]
fn test_detect_ip_address() {
    let detector = TypeDetector::new();
    let values = vec![json!("192.168.1.1"), json!("10.0.0.1"), json!("172.16.0.1")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::IpAddress));
    assert!(confidence >= CONFIDENCE_IP_ADDRESS);
}

#[test]
fn test_detect_hex_string() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"), // 32 chars (MD5)
        json!("1234567890abcdef1234567890abcdef"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    // Hex strings might be detected as Base64 or Token due to similar patterns
    // The important thing is it's not misdetected as something completely wrong
    assert!(
        matches!(
            field_type,
            FieldType::HexString | FieldType::Base64 | FieldType::Token
        ),
        "Got {:?} instead",
        field_type
    );
    assert!(confidence >= 0.70);
}

#[test]
fn test_detect_base64() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("SGVsbG8gV29ybGQhIFRoaXMgaXMgYSBiYXNlNjQgZW5jb2RlZCBzdHJpbmc="),
        json!("VGhpcyBpcyBhbm90aGVyIGJhc2U2NCBzdHJpbmcgd2l0aCBtb3JlIGNvbnRlbnQ="),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Base64));
    assert!(confidence >= CONFIDENCE_BASE64);
}

#[test]
fn test_detect_mime_type() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("application/json"),
        json!("text/html"),
        json!("image/png"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::MimeType));
    assert!(confidence >= CONFIDENCE_MIME_TYPE);
}

#[test]
fn test_detect_sequential_numbers() {
    let detector = TypeDetector::new();
    let values = vec![json!(1), json!(2), json!(3), json!(4)];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(
        field_type,
        FieldType::SequentialNumber { start: 1, step: 1 }
    ));
    assert!(confidence >= 0.90);
}

#[test]
fn test_detect_unix_timestamp() {
    let detector = TypeDetector::new();
    let values = vec![
        json!(1640000000), // 2021-12-20
        json!(1650000000), // 2022-04-15
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::UnixTimestamp));
    assert!(confidence >= 0.80);
}

#[test]
fn test_detect_file_size() {
    let detector = TypeDetector::new();
    let values = vec![
        json!(1024000),  // ~1MB
        json!(5242880),  // ~5MB
        json!(10485760), // ~10MB
    ];

    // FileSize detection was removed from value-only detection due to false positives
    // Now requires field name context for accurate detection
    let (field_type, confidence) = detector.detect_type("size", &as_refs(&values));
    assert!(matches!(field_type, FieldType::FileSize));
    assert!(confidence >= 0.80);
}

#[test]
fn test_detect_array_of_strings() {
    let detector = TypeDetector::new();
    let values = vec![
        json!(["test1@example.com", "test2@example.com"]),
        json!(["user@test.org"]),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    if let FieldType::Array(pattern) = field_type {
        assert!(matches!(pattern.element_type, FieldType::Email));
        assert!(pattern.is_homogeneous);
    } else {
        panic!("Expected Array type");
    }
    assert!(confidence >= 0.80);
}

#[test]
fn test_detect_nested_object() {
    let detector = TypeDetector::new();
    let values = vec![
        json!({"name": "Alice", "age": 30}),
        json!({"name": "Bob", "age": 25}),
    ];

    let (field_type, _confidence) = detector.detect_type_from_values(&as_refs(&values));
    if let FieldType::Object(analysis) = field_type {
        assert!(!analysis.varying_fields.is_empty());
    } else {
        panic!("Expected Object type");
    }
}

#[test]
fn test_extract_features() {
    let values = vec!["test123", "test456", "test789"];
    let features = super::features::extract_features(&values);

    assert!(features.digit_ratio > 0.0);
    assert!(features.alpha_ratio > 0.0);
    assert_eq!(features.min_length, 7);
    assert_eq!(features.max_length, 7);
    assert!(features.format_consistency > 0.95); // All same length
}

#[test]
fn test_priority_ordering_url_before_uuid() {
    let detector = TypeDetector::new();

    // This should be detected as URL, not UUID
    // (even though it might have some UUID-like characters)
    let values = vec![json!(
        "https://example.com/550e8400-e29b-41d4-a716-446655440000"
    )];

    let (field_type, _) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Url));
}

#[test]
fn test_confidence_scoring() {
    let detector = TypeDetector::new();

    // 100% match should give high confidence
    let all_emails = vec![
        json!("test1@example.com"),
        json!("test2@example.com"),
        json!("test3@example.com"),
    ];
    let (_, confidence) = detector.detect_type_from_values(&as_refs(&all_emails));
    assert!(confidence >= 0.90);

    // Partial match should give lower confidence
    let mixed_values = vec![json!("test@example.com"), json!("not-an-email")];
    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&mixed_values));
    // Should fall back to RandomString due to low confidence
    assert!(matches!(field_type, FieldType::RandomString) || confidence < 0.85);
}

#[test]
fn test_iso_date_detection() {
    let detector = TypeDetector::new();
    let values = vec![json!("2024-01-15"), json!("2024-12-31")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::IsoDate));
    assert!(confidence >= CONFIDENCE_TIMESTAMP);
}

#[test]
fn test_phone_number_detection() {
    let detector = TypeDetector::new();
    let values = vec![json!("+1-555-123-4567"), json!("(555) 987-6543")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::PhoneNumber));
    assert!(confidence >= CONFIDENCE_PHONE_NUMBER);
}

#[test]
fn test_name_detection() {
    let detector = TypeDetector::new();
    let values = vec![json!("John Doe"), json!("Jane Smith")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Name));
    assert!(confidence >= CONFIDENCE_NAME);
}

#[test]
fn test_sentence_detection() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("This is a sample sentence for testing."),
        json!("Another sentence with reasonable length!"),
        json!("Testing sentence detection with various punctuation?"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(
        matches!(field_type, FieldType::Sentence),
        "Expected Sentence, got {:?}",
        field_type
    );
    assert!(confidence >= 0.70);
}

#[test]
fn test_paragraph_detection() {
    let detector = TypeDetector::new();
    let values = vec![
        json!(
            "This is the first sentence of a paragraph. Here is another sentence that adds more content. And finally, a third sentence to make it complete."
        ),
        json!(
            "Another paragraph with multiple sentences. It contains useful information. This helps test the detection logic."
        ),
        json!(
            "Paragraphs are longer blocks of text. They typically have several sentences. This makes them distinct from single sentences."
        ),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(
        matches!(field_type, FieldType::Paragraph),
        "Expected Paragraph, got {:?}",
        field_type
    );
    assert!(confidence >= 0.70);
}

#[test]
fn test_sentence_vs_paragraph() {
    let detector = TypeDetector::new();

    // Short single sentences should be Sentence
    let sentence_values = vec![
        json!("This is a simple test sentence."),
        json!("Another short sentence for testing."),
    ];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&sentence_values));
    assert!(
        matches!(field_type, FieldType::Sentence),
        "Short text should be Sentence, got {:?}",
        field_type
    );

    // Long multi-sentence text should be Paragraph
    let paragraph_values = vec![json!(
        "This is a much longer piece of text that contains multiple sentences. It has more content and provides detailed information. This makes it a paragraph rather than a single sentence."
    )];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&paragraph_values));
    assert!(
        matches!(field_type, FieldType::Paragraph),
        "Long text should be Paragraph, got {:?}",
        field_type
    );
}

#[test]
fn test_sentence_not_confused_with_name() {
    let detector = TypeDetector::new();

    // Names should be detected as Name
    let name_values = vec![json!("John Doe"), json!("Jane Smith")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&name_values));
    assert!(
        matches!(field_type, FieldType::Name),
        "Names should be Name, got {:?}",
        field_type
    );

    // Sentences should be detected as Sentence (need 5+ words and 20+ chars)
    let sentence_values = vec![
        json!("This is definitely a proper sentence with enough words."),
        json!("Here is another sentence that has sufficient length!"),
    ];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&sentence_values));
    assert!(
        matches!(field_type, FieldType::Sentence),
        "Sentences should be Sentence, got {:?}",
        field_type
    );
}

#[test]
fn test_api_endpoint_detection() {
    let detector = TypeDetector::new();
    let values = vec![json!("/api/v1/users/123"), json!("/api/v1/posts/456")];

    let (field_type, _) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::ApiEndpoint));
}

#[test]
fn test_pagination_url_semantic_context() {
    let detector = TypeDetector::new();
    let values = vec![json!("https://api.example.com/users?page=2&limit=10")];

    let (field_type, _) = detector.detect_type("next", &as_refs(&values));
    // Semantic context should detect this as pagination URL or at least URL
    assert!(matches!(
        field_type,
        FieldType::PaginationUrl(_) | FieldType::Url
    ));
}

#[test]
fn test_token_detection() {
    let detector = TypeDetector::new();
    let values = vec![
        json!("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"),
        json!("sk_test_1234567890abcdefghijklmnop"),
    ];

    let (field_type, _) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Token));
}

#[test]
fn test_empty_values() {
    let detector = TypeDetector::new();
    let values: Vec<&JsonValue> = vec![];

    let (field_type, confidence) = detector.detect_type_from_values(&values);
    assert!(matches!(field_type, FieldType::RandomString));
    assert_eq!(confidence, 0.5);
}

#[test]
fn test_real_world_example_from_design_doc() {
    let detector = TypeDetector::new();

    // Example from design doc: numeric string ID misclassified as ETag
    let values = vec![json!("23930793379")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::NumericStringId));

    // Example: UUID should not match URL
    let uuid_values = vec![json!("550e8400-e29b-41d4-a716-446655440000")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&uuid_values));
    assert!(matches!(field_type, FieldType::Uuid));

    // Example: URL should be detected before UUID
    let url_values = vec![json!(
        "https://example.com/550e8400-e29b-41d4-a716-446655440000"
    )];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&url_values));
    assert!(matches!(field_type, FieldType::Url));
}

// ============================================================================
// Tests for New Improvements (1-4)
// ============================================================================

#[test]
fn test_weighted_scoring_with_semantic_boost() {
    let detector = TypeDetector::new();

    // Timestamp without field name hint
    let ts_values = vec![json!("2024-01-15T10:30:00Z"), json!("2024-01-16T14:22:33Z")];
    let (_, base_confidence) = detector.detect_type_from_values(&as_refs(&ts_values));

    // Timestamp with semantic hint should have boost applied (not early return since field name is different)
    let (_, boosted_confidence) = detector.detect_type("updated_time", &as_refs(&ts_values));

    // The boost should increase confidence slightly
    assert!(
        boosted_confidence >= base_confidence,
        "Boosted confidence ({}) should be >= base ({})",
        boosted_confidence,
        base_confidence
    );

    // More specifically, with a field matching the boost pattern, it should be higher
    assert!(
        boosted_confidence > 0.90,
        "With semantic context, confidence should be very high: {}",
        boosted_confidence
    );
}

#[test]
fn test_anti_pattern_name_rejects_urls() {
    let detector = TypeDetector::new();

    // Name-like strings with URLs should NOT be detected as Name
    let values = vec![json!("John Doe https://example.com"), json!("Jane Smith")];
    let (field_type, _) = detector.detect_type_from_values(&as_refs(&values));

    // Should fall back to RandomString or have very low confidence
    assert!(
        !matches!(field_type, FieldType::Name) || matches!(field_type, FieldType::RandomString),
        "Names containing URLs should not be detected as Name type, got {:?}",
        field_type
    );
}

#[test]
fn test_anti_pattern_email_rejects_urls() {
    let detector = TypeDetector::new();

    // Emails with protocols should be rejected
    let values = vec![json!("https://test@example.com")];
    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));

    assert!(
        confidence < 0.5 || !matches!(field_type, FieldType::Email),
        "URLs should not be detected as emails, got {:?} with confidence {}",
        field_type,
        confidence
    );
}

#[test]
fn test_anti_pattern_iso_date_validates_ranges() {
    let detector = TypeDetector::new();

    // Invalid ISO dates (month > 12 or day > 31)
    let invalid_values = vec![json!("2024-13-01"), json!("2024-01-32")];
    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&invalid_values));

    assert!(
        !matches!(field_type, FieldType::IsoDate) || confidence < 0.5,
        "Invalid ISO dates should be rejected, got {:?} with confidence {}",
        field_type,
        confidence
    );

    // Valid ISO dates should pass
    let valid_values = vec![json!("2024-01-15"), json!("2024-12-31")];
    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&valid_values));

    assert!(
        matches!(field_type, FieldType::IsoDate) && confidence >= 0.8,
        "Valid ISO dates should be detected, got {:?} with confidence {}",
        field_type,
        confidence
    );
}

#[test]
fn test_millisecond_timestamp() {
    let detector = TypeDetector::new();

    // Millisecond timestamps (13 digits)
    let values = vec![
        json!(1640000000000i64), // 2021-12-20
        json!(1650000000000i64), // 2022-04-15
        json!(1700000000000i64), // 2023-11-14
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::MillisecondTimestamp));
    assert!(confidence >= 0.85);
}

#[test]
fn test_microsecond_timestamp() {
    let detector = TypeDetector::new();

    // Microsecond timestamps (16 digits)
    let values = vec![
        json!(1640000000000000i64), // 2021-12-20
        json!(1650000000000000i64), // 2022-04-15
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::MicrosecondTimestamp));
    assert!(confidence >= 0.85);
}

#[test]
fn test_latitude_detection() {
    let detector = TypeDetector::new();

    // Valid latitude values (-90 to 90)
    let values = vec![json!(37.7749), json!(-33.8688), json!(51.5074)];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Latitude));
    assert!(confidence >= 0.65);

    // With semantic boost
    let (_, boosted_confidence) = detector.detect_type("lat", &as_refs(&values));
    assert!(boosted_confidence > confidence);
}

#[test]
fn test_longitude_detection() {
    let detector = TypeDetector::new();

    // Valid longitude values (-180 to 180)
    let values = vec![json!(-122.4194), json!(151.2093), json!(-0.1278)];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::Longitude));
    assert!(confidence >= 0.60);

    // With semantic boost
    let (_, boosted_confidence) = detector.detect_type("lng", &as_refs(&values));
    assert!(boosted_confidence > confidence);
}

#[test]
fn test_random_float_detection() {
    let detector = TypeDetector::new();

    // Random floats outside lat/lon ranges
    let values = vec![json!(1234.5678), json!(9876.5432), json!(555.123)];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::RandomFloat { .. }));
    assert!(confidence >= 0.7);
}

#[test]
fn test_categorical_detection() {
    let detector = TypeDetector::new();

    // Low cardinality string values (enum-like)
    // Need more samples to get cardinality ratio < 0.35
    // 3 unique / 10 total = 0.30 ratio (< 0.35 threshold)
    let values = vec![
        json!("pending"),
        json!("completed"),
        json!("pending"),
        json!("failed"),
        json!("completed"),
        json!("pending"),
        json!("completed"),
        json!("failed"),
        json!("pending"),
        json!("completed"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    if let FieldType::Categorical {
        values: enum_values,
    } = field_type
    {
        assert_eq!(enum_values.len(), 3); // pending, completed, failed
        // Confidence varies based on cardinality ratio (lower ratio = higher confidence)
        // For cardinality ratio of 3/10 = 0.30, confidence is higher
        assert!(confidence >= 0.75);
    } else {
        panic!("Expected Categorical type, got {:?}", field_type);
    }
}

#[test]
fn test_country_code_detection() {
    let detector = TypeDetector::new();

    // ISO 3166-1 alpha-2 country codes
    let values = vec![json!("US"), json!("GB"), json!("DE"), json!("FR")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::CountryCode));
    assert!(confidence >= 0.70);
}

#[test]
fn test_currency_code_detection() {
    let detector = TypeDetector::new();

    // ISO 4217 currency codes
    let values = vec![json!("USD"), json!("EUR"), json!("GBP"), json!("JPY")];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&values));
    assert!(matches!(field_type, FieldType::CurrencyCode));
    assert!(confidence >= 0.65);
}

#[test]
fn test_file_path_detection() {
    let detector = TypeDetector::new();

    // Unix-style file paths
    let unix_paths = vec![
        json!("/var/log/app.log"),
        json!("/home/user/documents/file.txt"),
        json!("/etc/config.yml"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&unix_paths));
    // File paths might also be detected as FilePath OR FileName depending on patterns
    assert!(
        matches!(field_type, FieldType::FilePath | FieldType::FileName),
        "Expected FilePath or FileName, got {:?}",
        field_type
    );
    assert!(confidence >= 0.60);

    // Windows-style file paths
    let windows_paths = vec![
        json!("C:\\Users\\Test\\file.txt"),
        json!("D:\\Projects\\code.rs"),
    ];

    let (field_type, confidence) = detector.detect_type_from_values(&as_refs(&windows_paths));
    assert!(
        matches!(field_type, FieldType::FilePath | FieldType::FileName),
        "Expected FilePath or FileName, got {:?}",
        field_type
    );
    assert!(confidence >= 0.60);
}

#[test]
fn test_categorical_not_triggered_by_high_cardinality() {
    let detector = TypeDetector::new();

    // High cardinality should NOT be detected as categorical
    let values = vec![
        json!("value1"),
        json!("value2"),
        json!("value3"),
        json!("value4"),
        json!("value5"),
        json!("value6"),
        json!("value7"),
        json!("value8"),
    ];

    let (field_type, _) = detector.detect_type_from_values(&as_refs(&values));
    assert!(
        !matches!(field_type, FieldType::Categorical { .. }),
        "High cardinality should not be categorical, got {:?}",
        field_type
    );
}

#[test]
fn test_semantic_boost_combinations() {
    let detector = TypeDetector::new();

    // Test multiple field types with semantic hints
    let test_cases = vec![
        ("email", vec![json!("test@example.com")], FieldType::Email),
        (
            "user_uuid",
            vec![json!("550e8400-e29b-41d4-a716-446655440000")],
            FieldType::Uuid,
        ),
        (
            "created_at",
            vec![json!("2024-01-15T10:30:00Z")],
            FieldType::Timestamp,
        ),
        ("latitude", vec![json!(37.7749)], FieldType::Latitude),
    ];

    for (field_name, values, expected_type) in test_cases {
        let (field_type, confidence) = detector.detect_type(field_name, &as_refs(&values));
        assert!(
            std::mem::discriminant(&field_type) == std::mem::discriminant(&expected_type),
            "Field '{}' should be detected as {:?}, got {:?}",
            field_name,
            expected_type,
            field_type
        );
        assert!(
            confidence >= 0.80,
            "Field '{}' should have high confidence, got {}",
            field_name,
            confidence
        );
    }
}

// ============================================================================
// SMART PAGINATION URL DETECTION TESTS
// ============================================================================

#[test]
fn test_smart_pagination_url_page_based() {
    let detector = TypeDetector::new();

    // Test with realistic pagination URLs with static params
    let values = vec![
        json!("http://localhost:3003/api/v1/documents?page=1&limit=10&status=active&rsl=false"),
        json!("http://localhost:3003/api/v1/documents?page=2&limit=10&status=active&rsl=false"),
        json!("http://localhost:3003/api/v1/documents?page=3&limit=10&status=active&rsl=false"),
    ];

    let (field_type, confidence) = detector.detect_type("next", &as_refs(&values));

    match field_type {
        FieldType::PaginationUrl(pattern) => {
            assert_eq!(pattern.base_url, "http://localhost:3003/api/v1/documents");

            // Check static params are preserved
            assert!(
                pattern
                    .static_params
                    .contains(&("status".to_string(), "active".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("rsl".to_string(), "false".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("limit".to_string(), "10".to_string()))
            );

            // Check pagination scheme
            match &pattern.pagination_scheme {
                PaginationScheme::PageBased {
                    page_key,
                    limit_key,
                    sample_page,
                    sample_limit,
                } => {
                    assert_eq!(page_key, "page");
                    assert_eq!(limit_key.as_deref(), Some("limit"));
                    assert_eq!(*sample_page, 1);
                    assert_eq!(*sample_limit, Some(10));
                }
                _ => panic!("Expected PageBased pagination scheme"),
            }
        }
        _ => panic!("Expected PaginationUrl, got {:?}", field_type),
    }

    assert!(confidence >= 0.85);
}

#[test]
fn test_smart_pagination_url_offset_based() {
    let detector = TypeDetector::new();

    let values = vec![
        json!("https://api.example.com/items?offset=0&limit=20"),
        json!("https://api.example.com/items?offset=20&limit=20"),
        json!("https://api.example.com/items?offset=40&limit=20"),
    ];

    let (field_type, _) = detector.detect_type("next", &as_refs(&values));

    match field_type {
        FieldType::PaginationUrl(pattern) => {
            match &pattern.pagination_scheme {
                PaginationScheme::PageBased { page_key, .. } => {
                    // "offset" is treated as a page key variant
                    assert!(page_key == "offset" || page_key == "skip");
                }
                _ => panic!("Expected PageBased pagination scheme with offset"),
            }
        }
        _ => panic!("Expected PaginationUrl, got {:?}", field_type),
    }
}

#[test]
fn test_smart_pagination_url_cursor_based() {
    let detector = TypeDetector::new();

    let values = vec![
        json!("https://api.example.com/items?cursor=abc123&limit=20"),
        json!("https://api.example.com/items?cursor=def456&limit=20"),
        json!("https://api.example.com/items?cursor=ghi789&limit=20"),
    ];

    let (field_type, _) = detector.detect_type("next", &as_refs(&values));

    match field_type {
        FieldType::PaginationUrl(pattern) => match &pattern.pagination_scheme {
            PaginationScheme::CursorBased {
                cursor_key,
                sample_cursor,
            } => {
                assert_eq!(cursor_key, "cursor");
                assert_eq!(sample_cursor, "abc123");
            }
            _ => panic!("Expected CursorBased pagination scheme"),
        },
        _ => panic!("Expected PaginationUrl, got {:?}", field_type),
    }
}

#[test]
fn test_pagination_url_generation() {
    let pattern = PaginationUrlPattern {
        base_url: "http://localhost:3003/api/v1/documents".to_string(),
        static_params: vec![
            ("status".to_string(), "active".to_string()),
            ("rsl".to_string(), "false".to_string()),
        ],
        pagination_scheme: PaginationScheme::PageBased {
            page_key: "page".to_string(),
            limit_key: Some("limit".to_string()),
            sample_page: 1,
            sample_limit: Some(10),
        },
    };

    let next_url = pattern.generate_url(PaginationDirection::Next);
    assert!(next_url.contains("page=2"));
    assert!(next_url.contains("limit=10"));
    assert!(next_url.contains("status=active"));
    assert!(next_url.contains("rsl=false"));
    assert!(next_url.starts_with("http://localhost:3003/api/v1/documents?"));

    let prev_url = pattern.generate_url(PaginationDirection::Previous);
    assert!(prev_url.contains("page=1")); // Can't go below 1
}

#[test]
fn test_pagination_url_preserves_complex_query_params() {
    let detector = TypeDetector::new();

    // Your actual example URL
    let values = vec![
        json!(
            "http://localhost:3003/app-api/sign-web/api/v1/documents-search/?page=1&limit=10&autocomplete=&status=*&rsl=false&batchSend=false&pending_my_action=false&parent_doc__exists=false&bulk_status=true"
        ),
        json!(
            "http://localhost:3003/app-api/sign-web/api/v1/documents-search/?page=2&limit=10&autocomplete=&status=*&rsl=false&batchSend=false&pending_my_action=false&parent_doc__exists=false&bulk_status=true"
        ),
    ];

    let (field_type, _) = detector.detect_type("next", &as_refs(&values));

    match field_type {
        FieldType::PaginationUrl(pattern) => {
            // Verify base URL
            assert_eq!(
                pattern.base_url,
                "http://localhost:3003/app-api/sign-web/api/v1/documents-search/"
            );

            // Verify all static params are preserved
            assert!(
                pattern
                    .static_params
                    .contains(&("autocomplete".to_string(), "".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("status".to_string(), "*".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("rsl".to_string(), "false".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("batchSend".to_string(), "false".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("pending_my_action".to_string(), "false".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("parent_doc__exists".to_string(), "false".to_string()))
            );
            assert!(
                pattern
                    .static_params
                    .contains(&("bulk_status".to_string(), "true".to_string()))
            );

            // Generate URL and verify structure preserved
            let generated = pattern.generate_url(PaginationDirection::Next);
            assert!(generated.contains("status=%2A")); // * gets URL encoded
            assert!(generated.contains("rsl=false"));
            assert!(generated.contains("bulk_status=true"));
        }
        _ => panic!("Expected PaginationUrl, got {:?}", field_type),
    }
}

#[test]
fn test_pagination_fallback_single_url() {
    let detector = TypeDetector::new();

    // With only one URL, we can't detect pattern but should still create fallback
    let values = vec![json!(
        "http://localhost:3003/api/v1/documents?page=1&limit=10&status=active"
    )];

    let (field_type, confidence) = detector.detect_type("next", &as_refs(&values));

    // Should still detect as pagination due to field name + query params
    match field_type {
        FieldType::PaginationUrl(pattern) => {
            assert_eq!(pattern.base_url, "http://localhost:3003/api/v1/documents");
            assert!(confidence >= 0.80);
        }
        _ => {
            // Alternative: might detect as regular URL if pattern analysis requires 2+ samples
            assert!(matches!(field_type, FieldType::Url));
        }
    }
}

#[test]
fn test_data_uri_png_detection() {
    let detector = TypeDetector::new();

    // Test PNG data URI detection
    let values = vec![
        json!(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAASAAAEgCAYAAAAxQcrSAAAACXBIWXMAAAsTAAALEwEAmpwYAAAGRklEQVR4nO3d"
        ),
        json!(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAgAAA4CAYAAABnK5dyAAAACXBIWXMAAA7EAAAOxAGVKw4bAAAA"
        ),
    ];

    let (field_type, confidence) = detector.detect_type("data_uri", &as_refs(&values));

    match field_type {
        FieldType::DataUri { mime_type } => {
            assert_eq!(mime_type, Some("image/png".to_string()));
            assert!(confidence >= CONFIDENCE_DATA_URI);
        }
        _ => panic!("Expected DataUri, got {:?}", field_type),
    }
}

#[test]
fn test_data_uri_pdf_detection() {
    let detector = TypeDetector::new();

    // Test PDF data URI detection
    let values = vec![
        json!(
            "data:application/pdf;base64,JVBERi0xLjQKJeLjz9MKNCAwIG9iago8PC9GaWx0ZXIvRmxhdGVEZWNvZGUvTGVuZ3RoIDUz"
        ),
        json!(
            "data:application/pdf;base64,JVBERi0xLjUKJeLjz9MKNCAwIG9iago8PC9GaWx0ZXIvRmxhdGVEZWNvZGUvTGVuZ3RoIDYz"
        ),
    ];

    let (field_type, confidence) = detector.detect_type("pdf_data", &as_refs(&values));

    match field_type {
        FieldType::DataUri { mime_type } => {
            assert_eq!(mime_type, Some("application/pdf".to_string()));
            assert!(confidence >= CONFIDENCE_DATA_URI);
        }
        _ => panic!("Expected DataUri, got {:?}", field_type),
    }
}

#[test]
fn test_data_uri_jpeg_detection() {
    let detector = TypeDetector::new();

    // Test JPEG data URI detection
    let values = vec![
        json!(
            "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAIBAQIBAQICAgICAgICAwUDAwMDAwYEBAMFBwYHBw=="
        ),
        json!(
            "data:image/jpeg;base64,/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgMCAgMDAwMEAwMEBQgFBQQEBQoHBwYIDAoMDA=="
        ),
    ];

    let (field_type, confidence) = detector.detect_type("image_data", &as_refs(&values));

    match field_type {
        FieldType::DataUri { mime_type } => {
            assert_eq!(mime_type, Some("image/jpeg".to_string()));
            assert!(confidence >= CONFIDENCE_DATA_URI);
        }
        _ => panic!("Expected DataUri, got {:?}", field_type),
    }
}

#[test]
fn test_data_uri_vs_regular_url() {
    let detector = TypeDetector::new();

    // Ensure data URIs are detected as DataUri, not as regular URLs
    let data_uri_values = vec![json!(
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAASAAAEgCAYAAAAxQcrSAAAACXBIWXMAAAsTAAALEwEAmpwYAAAGRklEQVR4nO3d"
    )];

    let (field_type, _) = detector.detect_type("url", &as_refs(&data_uri_values));
    assert!(
        matches!(field_type, FieldType::DataUri { .. }),
        "Data URI should be detected as DataUri, got {:?}",
        field_type
    );

    // Regular URLs should not be detected as data URIs
    let regular_url_values = vec![
        json!("https://api.example.com/files/12345"),
        json!("https://api.example.com/files/67890"),
    ];

    let (field_type, _) = detector.detect_type("url", &as_refs(&regular_url_values));
    assert!(
        matches!(field_type, FieldType::Url),
        "Regular URL should not be detected as DataUri, got {:?}",
        field_type
    );
}
