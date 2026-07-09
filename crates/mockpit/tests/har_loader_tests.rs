#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use har::{Har, Spec, v1_2};
use http::{Method, StatusCode};
use mockpit::config::{HarLoadOptions, HarLoader, ReturnConfig};
use mockpit::engine::har_export::export_mocks_to_har;
use mockpit::engine::{BodySource, MockDefinition, RequestMatcher, ResponseGenerator, UrlPattern};
use rustc_hash::FxHashMap;
use smallvec::smallvec;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_exclude_redirects() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/old", 301), // Redirect
                create_test_entry("GET", "https://api.example.com/temp", 302), // Redirect
                create_test_entry("GET", "https://api.example.com/final", 200), // OK
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_redirects: true,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Should only include the 200 response
    assert_eq!(mocks.len(), 1);
    let response_config = mocks[0].response_config.as_ref().unwrap();
    assert_eq!(response_config.status().unwrap(), 200);
}

#[tokio::test]
async fn test_include_redirects() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/old", 301),
                create_test_entry("GET", "https://api.example.com/final", 200),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_redirects: false,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Should include both
    assert_eq!(mocks.len(), 2);
}

#[tokio::test]
async fn test_strip_browser_headers() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.headers = vec![
                    v1_2::Headers {
                        name: "content-type".to_string(),
                        value: "application/json".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "user-agent".to_string(),
                        value: "Mozilla/5.0".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "accept-language".to_string(),
                        value: "en-US".to_string(),
                        comment: None,
                    },
                ];
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        strip_browser_headers: true,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Should only have content-type header
    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 1);
            assert!(headers.contains_key("content-type"));
            assert!(!headers.contains_key("user-agent"));
            assert!(!headers.contains_key("accept-language"));
        }
        _ => panic!("Expected Structured return config"),
    }
}

#[tokio::test]
async fn test_keep_all_headers() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.headers = vec![
                    v1_2::Headers {
                        name: "content-type".to_string(),
                        value: "application/json".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "user-agent".to_string(),
                        value: "Mozilla/5.0".to_string(),
                        comment: None,
                    },
                ];
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        strip_browser_headers: false,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Should have all headers
    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 2);
            assert!(headers.contains_key("content-type"));
            assert!(headers.contains_key("user-agent"));
        }
        _ => panic!("Expected Structured return config"),
    }
}

#[tokio::test]
async fn test_delay_extraction() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/slow", 200);
                entry.timings.wait = 250.0;
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Delay is now at the top level of MockConfig
    assert_eq!(mocks[0].delay.as_deref(), Some("250ms"));
}

#[tokio::test]
async fn test_zero_delay() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/fast", 200);
                entry.timings.wait = 0.0;
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Zero/negative delay means no delay at MockConfig level
    assert!(mocks[0].delay.is_none());
}

#[tokio::test]
async fn test_priority_assignment() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/first", 200),
                create_test_entry("GET", "https://api.example.com/second", 200),
                create_test_entry("GET", "https://api.example.com/third", 200),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Priority should decrease for later entries
    assert_eq!(mocks[0].priority, 100);
    assert_eq!(mocks[1].priority, 99);
    assert_eq!(mocks[2].priority, 98);
}

#[tokio::test]
async fn test_mock_id_generation() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/one", 200),
                create_test_entry("POST", "https://api.example.com/two", 201),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks[0].id, "har-entry-1");
    assert_eq!(mocks[1].id, "har-entry-2");
}

#[tokio::test]
async fn test_empty_har() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 0);
}

#[tokio::test]
async fn test_invalid_har_file() {
    let temp_dir = TempDir::new().unwrap();
    let har_path = temp_dir.path().join("invalid.har");

    tokio::fs::write(&har_path, "not json").await.unwrap();

    let loader = HarLoader::new();
    let result = loader.load_from_file(&har_path).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_missing_file() {
    let loader = HarLoader::new();
    let result = loader.load_from_file("/nonexistent/file.har").await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_default_options() {
    let options = HarLoadOptions::default();
    assert!(options.exclude_preflight);
    assert!(options.exclude_redirects);
    assert!(options.strip_browser_headers);
}

#[tokio::test]
async fn test_loader_default() {
    let _loader1 = HarLoader::new();
    let _loader2 = HarLoader::default();

    // Both loaders can be created (options are private, so we just verify construction)
}

#[tokio::test]
async fn test_multiple_methods() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/test", 200),
                create_test_entry("POST", "https://api.example.com/test", 201),
                create_test_entry("DELETE", "https://api.example.com/test", 204),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 3);
    assert_eq!(mocks[0].match_config.as_ref().unwrap().methods[0], "GET");
    assert_eq!(mocks[1].match_config.as_ref().unwrap().methods[0], "POST");
    assert_eq!(mocks[2].match_config.as_ref().unwrap().methods[0], "DELETE");
}

fn create_test_entry(method: &str, url: &str, status: i64) -> v1_2::Entries {
    v1_2::Entries {
        pageref: None,
        started_date_time: "2025-10-07T12:00:00.000Z".to_string(),
        time: 50.0,
        request: v1_2::Request {
            method: method.to_string(),
            url: url.to_string(),
            http_version: "HTTP/1.1".to_string(),
            cookies: vec![],
            headers: vec![],
            query_string: vec![],
            post_data: None,
            headers_size: -1,
            body_size: 0,
            comment: None,
        },
        response: v1_2::Response {
            status,
            status_text: "OK".to_string(),
            http_version: "HTTP/1.1".to_string(),
            cookies: vec![],
            headers: vec![],
            content: v1_2::Content {
                size: 0,
                compression: None,
                mime_type: Some("application/json".to_string()),
                text: Some("{}".to_string()),
                encoding: None,
                comment: None,
            },
            redirect_url: Some(String::new()),
            headers_size: -1,
            body_size: 0,
            comment: None,
        },
        cache: v1_2::Cache {
            before_request: None,
            after_request: None,
        },
        timings: v1_2::Timings {
            blocked: None,
            dns: None,
            connect: None,
            send: 0.0,
            wait: 50.0,
            receive: 0.0,
            ssl: None,
            comment: None,
        },
        server_ip_address: None,
        connection: None,
        comment: None,
    }
}

#[tokio::test]
async fn test_missing_response_body_text() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.content.text = None; // Missing body text
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 1);
    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { body, .. } => {
            assert_eq!(body.as_deref(), Some("")); // Should default to empty string
        }
        _ => panic!("Expected Structured return config"),
    }
}

#[tokio::test]
async fn test_negative_wait_time() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.timings.wait = -10.0; // Negative delay
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Negative delay should be treated as no delay
    assert!(mocks[0].delay.is_none());
}

#[tokio::test]
async fn test_very_large_delay() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/slow", 200);
                entry.timings.wait = 5000.0; // 5 seconds
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Delay is now at the top level of MockConfig
    assert_eq!(mocks[0].delay.as_deref(), Some("5000ms"));
}

#[tokio::test]
async fn test_fractional_delay() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.timings.wait = 123.7; // Fractional ms
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    // Delay is now at the top level of MockConfig
    assert_eq!(mocks[0].delay.as_deref(), Some("123ms")); // Should truncate to integer
}

// ============================================================================
// Response Header Conversion Tests
// ============================================================================

#[tokio::test]
async fn test_empty_response_headers() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.headers = vec![]; // No headers
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 0);
        }
        _ => panic!("Expected Structured return config"),
    }
}

#[tokio::test]
async fn test_multiple_headers() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.headers = vec![
                    v1_2::Headers {
                        name: "content-type".to_string(),
                        value: "application/json".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "x-request-id".to_string(),
                        value: "abc123".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "x-rate-limit".to_string(),
                        value: "100".to_string(),
                        comment: None,
                    },
                ];
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 3);
            assert_eq!(headers.get("content-type").unwrap(), "application/json");
            assert_eq!(headers.get("x-request-id").unwrap(), "abc123");
            assert_eq!(headers.get("x-rate-limit").unwrap(), "100");
        }
        _ => panic!("Expected Structured return config"),
    }
}

#[tokio::test]
async fn test_strip_all_browser_headers() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.headers = vec![
                    v1_2::Headers {
                        name: "user-agent".to_string(),
                        value: "Mozilla".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "accept-encoding".to_string(),
                        value: "gzip".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "cache-control".to_string(),
                        value: "no-cache".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "connection".to_string(),
                        value: "keep-alive".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "upgrade-insecure-requests".to_string(),
                        value: "1".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "sec-fetch-site".to_string(),
                        value: "same-origin".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "sec-fetch-mode".to_string(),
                        value: "navigate".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "sec-fetch-dest".to_string(),
                        value: "document".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "sec-ch-ua".to_string(),
                        value: "Chrome".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "sec-ch-ua-mobile".to_string(),
                        value: "?0".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "sec-ch-ua-platform".to_string(),
                        value: "macOS".to_string(),
                        comment: None,
                    },
                ];
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        strip_browser_headers: true,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 0); // All browser headers should be stripped
        }
        _ => panic!("Expected Structured return config"),
    }
}

#[tokio::test]
async fn test_strip_case_insensitive_headers() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![{
                let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                entry.response.headers = vec![
                    v1_2::Headers {
                        name: "User-Agent".to_string(),
                        value: "Mozilla".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "ACCEPT-LANGUAGE".to_string(),
                        value: "en-US".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "Cache-Control".to_string(),
                        value: "no-cache".to_string(),
                        comment: None,
                    },
                    v1_2::Headers {
                        name: "content-type".to_string(),
                        value: "application/json".to_string(),
                        comment: None,
                    },
                ];
                entry
            }],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        strip_browser_headers: true,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    let response_config = mocks[0].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert_eq!(headers.len(), 1); // Only content-type should remain
            assert_eq!(headers.get("content-type").unwrap(), "application/json");
        }
        _ => panic!("Expected Structured return config"),
    }
}

// ============================================================================
// URL Pattern and Method Tests
// ============================================================================

#[tokio::test]
async fn test_url_with_query_string() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![create_test_entry(
                "GET",
                "https://api.example.com/users?page=1&limit=10",
                200,
            )],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    let match_config = mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.urls[0], "exact:/users?page=1&limit=10");
}

#[tokio::test]
async fn test_url_with_fragment() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![create_test_entry(
                "GET",
                "https://api.example.com/page#section",
                200,
            )],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    let match_config = mocks[0].match_config.as_ref().unwrap();
    assert_eq!(match_config.urls[0], "exact:/page");
}

#[tokio::test]
async fn test_various_http_methods() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/test", 200),
                create_test_entry("POST", "https://api.example.com/test", 201),
                create_test_entry("PUT", "https://api.example.com/test", 200),
                create_test_entry("PATCH", "https://api.example.com/test", 200),
                create_test_entry("DELETE", "https://api.example.com/test", 204),
                create_test_entry("HEAD", "https://api.example.com/test", 200),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 6);
    assert_eq!(mocks[0].match_config.as_ref().unwrap().methods[0], "GET");
    assert_eq!(mocks[1].match_config.as_ref().unwrap().methods[0], "POST");
    assert_eq!(mocks[2].match_config.as_ref().unwrap().methods[0], "PUT");
    assert_eq!(mocks[3].match_config.as_ref().unwrap().methods[0], "PATCH");
    assert_eq!(mocks[4].match_config.as_ref().unwrap().methods[0], "DELETE");
    assert_eq!(mocks[5].match_config.as_ref().unwrap().methods[0], "HEAD");
}

// ============================================================================
// Response Status Code Tests
// ============================================================================

#[tokio::test]
async fn test_various_2xx_status_codes() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/test1", 200),
                create_test_entry("POST", "https://api.example.com/test2", 201),
                create_test_entry("PUT", "https://api.example.com/test3", 202),
                create_test_entry("DELETE", "https://api.example.com/test4", 204),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 4);
    assert_eq!(
        mocks[0].response_config.as_ref().unwrap().status().unwrap(),
        200
    );
    assert_eq!(
        mocks[1].response_config.as_ref().unwrap().status().unwrap(),
        201
    );
    assert_eq!(
        mocks[2].response_config.as_ref().unwrap().status().unwrap(),
        202
    );
    assert_eq!(
        mocks[3].response_config.as_ref().unwrap().status().unwrap(),
        204
    );
}

#[tokio::test]
async fn test_3xx_redirect_codes() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/301", 301),
                create_test_entry("GET", "https://api.example.com/302", 302),
                create_test_entry("GET", "https://api.example.com/304", 304),
                create_test_entry("GET", "https://api.example.com/307", 307),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_redirects: false,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 4);
    assert_eq!(
        mocks[0].response_config.as_ref().unwrap().status().unwrap(),
        301
    );
    assert_eq!(
        mocks[1].response_config.as_ref().unwrap().status().unwrap(),
        302
    );
    assert_eq!(
        mocks[2].response_config.as_ref().unwrap().status().unwrap(),
        304
    );
    assert_eq!(
        mocks[3].response_config.as_ref().unwrap().status().unwrap(),
        307
    );
}

#[tokio::test]
async fn test_4xx_client_error_codes() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/400", 400),
                create_test_entry("GET", "https://api.example.com/401", 401),
                create_test_entry("GET", "https://api.example.com/403", 403),
                create_test_entry("GET", "https://api.example.com/404", 404),
                create_test_entry("GET", "https://api.example.com/429", 429),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 5);
    assert_eq!(
        mocks[0].response_config.as_ref().unwrap().status().unwrap(),
        400
    );
    assert_eq!(
        mocks[1].response_config.as_ref().unwrap().status().unwrap(),
        401
    );
    assert_eq!(
        mocks[2].response_config.as_ref().unwrap().status().unwrap(),
        403
    );
    assert_eq!(
        mocks[3].response_config.as_ref().unwrap().status().unwrap(),
        404
    );
    assert_eq!(
        mocks[4].response_config.as_ref().unwrap().status().unwrap(),
        429
    );
}

#[tokio::test]
async fn test_5xx_server_error_codes() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/500", 500),
                create_test_entry("GET", "https://api.example.com/502", 502),
                create_test_entry("GET", "https://api.example.com/503", 503),
                create_test_entry("GET", "https://api.example.com/504", 504),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 4);
    assert_eq!(
        mocks[0].response_config.as_ref().unwrap().status().unwrap(),
        500
    );
    assert_eq!(
        mocks[1].response_config.as_ref().unwrap().status().unwrap(),
        502
    );
    assert_eq!(
        mocks[2].response_config.as_ref().unwrap().status().unwrap(),
        503
    );
    assert_eq!(
        mocks[3].response_config.as_ref().unwrap().status().unwrap(),
        504
    );
}

#[tokio::test]
async fn test_redirect_boundary_299() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![create_test_entry("GET", "https://api.example.com/299", 299)],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_redirects: true,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 1); // 299 is not a redirect, should be included
}

#[tokio::test]
async fn test_redirect_boundary_400() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![create_test_entry("GET", "https://api.example.com/400", 400)],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_redirects: true,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 1); // 400 is not a redirect, should be included
}

// ============================================================================
// Export Mocks to HAR Tests
// ============================================================================

#[tokio::test]
async fn test_export_single_mock_to_har() {
    let mock = MockDefinition {
        id: "test-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/users/123")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(
            StatusCode::OK,
            BodySource::inline(r#"{"id":"123","name":"Test"}"#),
        ),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            assert_eq!(log.entries[0].request.method, "GET");
            assert_eq!(
                log.entries[0].request.url,
                "https://api.example.com/users/123"
            );
            assert_eq!(log.entries[0].response.status, 200);
            assert_eq!(log.creator.name, mockpit::core::app_name());
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_multiple_mocks_to_har() {
    let mocks = vec![
        MockDefinition {
            id: "mock-1".into(),
            priority: 100,
            enabled: true,
            once: false,
            source_file: None,
            scope: None,
            request_transforms: None,

            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact("https://api.example.com/users")],
                header_matchers: smallvec![],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
            vars: None,
            streaming: None,
        },
        MockDefinition {
            id: "mock-2".into(),
            priority: 90,
            enabled: false,
            once: false,
            source_file: None,
            scope: Some("test-scope".into()),
            request_transforms: None,
            request: RequestMatcher {
                methods: smallvec![Method::POST],
                url_patterns: smallvec![UrlPattern::prefix("https://api.example.com/files")],
                header_matchers: smallvec![],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator::new(StatusCode::CREATED, BodySource::inline("{}")),
            vars: None,
            streaming: None,
        },
    ];

    let arc_mocks: Vec<Arc<MockDefinition>> = mocks.into_iter().map(Arc::new).collect();
    let har = export_mocks_to_har(&arc_mocks).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 2);
            assert_eq!(log.entries[0].request.method, "GET");
            assert_eq!(log.entries[1].request.method, "POST");
            assert_eq!(log.entries[0].response.status, 200);
            assert_eq!(log.entries[1].response.status, 201);
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_delay() {
    let mock = MockDefinition {
        id: "delayed-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/slow")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}"))
            .with_delay(std::time::Duration::from_millis(250)),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            assert!((log.entries[0].timings.wait - 250.0).abs() < f64::EPSILON);
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_headers() {
    let mock = MockDefinition {
        id: "header-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/test")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}"))
            .with_header("content-type", "application/json")
            .with_header("x-custom", "value"),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            assert_eq!(log.entries[0].response.headers.len(), 2);

            let headers: FxHashMap<_, _> = log.entries[0]
                .response
                .headers
                .iter()
                .map(|h| (h.name.as_str(), h.value.as_str()))
                .collect();

            assert_eq!(headers.get("content-type"), Some(&"application/json"));
            assert_eq!(headers.get("x-custom"), Some(&"value"));
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_regex_pattern() {
    let mock = MockDefinition {
        id: "regex-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::regex(r"^/api/users/\d+$").unwrap()],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            // Regex patterns are exported with special format
            assert!(log.entries[0].request.url.contains("^/api/users/\\d+$"));
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_prefix_pattern() {
    let mock = MockDefinition {
        id: "prefix-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::prefix("https://api.example.com/v2/")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            assert_eq!(log.entries[0].request.url, "https://api.example.com/v2/");
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_file_body() {
    let mock = MockDefinition {
        id: "file-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/data")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::file("/path/to/file.json")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            let body_text = log.entries[0].response.content.text.as_ref().unwrap();
            assert!(body_text.contains("<file: /path/to/file.json>"));
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_template_body() {
    let mock = MockDefinition {
        id: "template-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/test")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::template("{{ user.name }}")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            let body_text = log.entries[0].response.content.text.as_ref().unwrap();
            assert!(body_text.contains("<template: {{ user.name }}>"));
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_no_methods() {
    let mock = MockDefinition {
        id: "no-method-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![], // No methods specified
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/test")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            assert_eq!(log.entries[0].request.method, "GET"); // Should default to GET
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_with_multiple_methods() {
    let mock = MockDefinition {
        id: "multi-method-mock".into(),
        priority: 100,
        enabled: true,
        once: false,
        source_file: None,
        scope: None,
        request_transforms: None,

        request: RequestMatcher {
            methods: smallvec![Method::GET, Method::POST],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/test")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            assert_eq!(log.entries[0].request.method, "GET"); // Should use first method
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_empty_mocks_list() {
    let har = export_mocks_to_har(&[]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 0);
            assert_eq!(log.creator.name, mockpit::core::app_name());
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

#[tokio::test]
async fn test_export_mock_includes_metadata() {
    let mock = MockDefinition {
        id: "metadata-mock".into(),
        priority: 75,
        enabled: false,
        once: false,
        source_file: None,
        scope: Some("test".into()),
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact("https://api.example.com/test")],
            header_matchers: smallvec![],
            query_matchers: smallvec![],
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline("{}")),
        vars: None,
        streaming: None,
    };

    let har = export_mocks_to_har(&[Arc::new(mock)]).unwrap();

    match &har.log {
        Spec::V1_2(log) => {
            assert_eq!(log.entries.len(), 1);
            let request_comment = log.entries[0].request.comment.as_ref().unwrap();
            let response_comment = log.entries[0].response.comment.as_ref().unwrap();
            let entry_comment = log.entries[0].comment.as_ref().unwrap();

            assert!(request_comment.contains("metadata-mock"));
            assert!(response_comment.contains("75"));
            assert!(entry_comment.contains("enabled: false"));
        }
        Spec::V1_3(_) => panic!("Expected HAR v1.2"),
    }
}

// ============================================================================
// HAR Load Options Tests
// ============================================================================

#[tokio::test]
async fn test_include_preflight_option() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("OPTIONS", "https://api.example.com/test", 204),
                create_test_entry("GET", "https://api.example.com/test", 200),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_preflight: false,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 2); // Should include OPTIONS
}

#[tokio::test]
async fn test_all_options_disabled() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("OPTIONS", "https://api.example.com/test", 204),
                create_test_entry("GET", "https://api.example.com/old", 301),
                {
                    let mut entry = create_test_entry("GET", "https://api.example.com/test", 200);
                    entry.response.headers = vec![v1_2::Headers {
                        name: "user-agent".to_string(),
                        value: "Mozilla".to_string(),
                        comment: None,
                    }];
                    entry
                },
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::with_options(HarLoadOptions {
        exclude_preflight: false,
        exclude_redirects: false,
        strip_browser_headers: false,
        ..Default::default()
    });
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 3); // Should include all entries

    // Check that browser headers are preserved
    let response_config = mocks[2].response_config.as_ref().unwrap();
    match response_config {
        ReturnConfig::Structured { headers, .. } => {
            assert!(headers.contains_key("user-agent"));
        }
        _ => panic!("Expected Structured return config"),
    }
}

// ============================================================================
// Priority Assignment Tests
// ============================================================================

#[tokio::test]
async fn test_priority_large_number_of_entries() {
    let entries: Vec<_> = (0..150)
        .map(|i| create_test_entry("GET", &format!("https://api.example.com/test{i}"), 200))
        .collect();

    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries,
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks.len(), 150);
    assert_eq!(mocks[0].priority, 100);
    assert_eq!(mocks[1].priority, 99);
    assert_eq!(mocks[99].priority, 1);
    assert_eq!(mocks[100].priority, 0); // Wraps to 0
    assert_eq!(mocks[149].priority, 0); // All after 100 are 0
}

// ============================================================================
// Mock ID Generation Tests
// ============================================================================

#[tokio::test]
async fn test_mock_id_sequential() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![
                create_test_entry("GET", "https://api.example.com/a", 200),
                create_test_entry("GET", "https://api.example.com/b", 200),
                create_test_entry("GET", "https://api.example.com/c", 200),
            ],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert_eq!(mocks[0].id, "har-entry-1");
    assert_eq!(mocks[1].id, "har-entry-2");
    assert_eq!(mocks[2].id, "har-entry-3");
}

// ============================================================================
// Mock Enabled and Scope Tests
// ============================================================================

#[tokio::test]
async fn test_mocks_enabled_by_default() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![create_test_entry(
                "GET",
                "https://api.example.com/test",
                200,
            )],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert!(mocks[0].enabled);
}

#[tokio::test]
async fn test_mocks_no_scope_by_default() {
    let har = Har {
        log: Spec::V1_2(v1_2::Log {
            creator: v1_2::Creator {
                name: "test".to_string(),
                version: "1.0".to_string(),
                comment: None,
            },
            browser: None,
            pages: None,
            entries: vec![create_test_entry(
                "GET",
                "https://api.example.com/test",
                200,
            )],
            comment: None,
        }),
    };

    let loader = HarLoader::new();
    let mocks = loader.convert_har_to_mocks(har).await.unwrap();

    assert!(mocks[0].scope.is_none());
}

// ============================================================================
// Chrome DevTools _webSocketMessages Tests
// ============================================================================

fn ws_har_fixture(messages: &str) -> String {
    format!(
        r#"{{
  "log": {{
    "version": "1.2",
    "creator": {{ "name": "WebInspector", "version": "537.36" }},
    "entries": [
      {{
        "startedDateTime": "2026-07-01T10:00:00.000Z",
        "time": 1.0,
        "request": {{
          "method": "GET",
          "url": "wss://chat.example.com/socket",
          "httpVersion": "HTTP/1.1",
          "cookies": [],
          "headers": [],
          "queryString": [],
          "headersSize": -1,
          "bodySize": -1
        }},
        "response": {{
          "status": 101,
          "statusText": "Switching Protocols",
          "httpVersion": "HTTP/1.1",
          "cookies": [],
          "headers": [],
          "content": {{ "size": 0, "mimeType": "x-unknown" }},
          "redirectURL": "",
          "headersSize": -1,
          "bodySize": -1
        }},
        "cache": {{}},
        "timings": {{ "send": 0, "wait": 0, "receive": 0 }},
        "_resourceType": "websocket",
        "_webSocketMessages": [ {messages} ]
      }}
    ]
  }}
}}"#
    )
}

async fn load_har_string(content: &str) -> Vec<mockpit::config::MockConfig> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("capture.har");
    std::fs::write(&path, content).unwrap();
    HarLoader::new().load_from_file(&path).await.unwrap()
}

#[tokio::test]
async fn test_websocket_messages_become_a_ws_mock() {
    let mocks = load_har_string(&ws_har_fixture(
        r#"
        { "type": "receive", "time": 1751364000.1, "opcode": 1, "data": "welcome" },
        { "type": "send", "time": 1751364000.5, "opcode": 1, "data": "ping" },
        { "type": "receive", "time": 1751364000.62, "opcode": 1, "data": "pong" },
        { "type": "send", "time": 1751364001.0, "opcode": 2, "data": "AAEC" },
        { "type": "receive", "time": 1751364001.2, "opcode": 2, "data": "//79" }
"#,
    ))
    .await;

    // The 101 entry itself must NOT become an HTTP mock.
    assert_eq!(mocks.len(), 1, "{mocks:?}");
    let mock = &mocks[0];
    assert_eq!(mock.id, "har-ws-1");
    let ws = mock.ws.as_ref().expect("ws config");

    // Pre-client server frame replays on connect.
    assert_eq!(ws.on_connect.len(), 1);
    let yaml = serde_yaml::to_string(&ws.on_connect).unwrap();
    assert!(yaml.contains("welcome"), "{yaml}");

    // Text pairing: exact ping -> delay + pong.
    assert_eq!(ws.on_message.len(), 2);
    let rules = serde_yaml::to_string(&ws.on_message).unwrap();
    assert!(rules.contains("exact: ping"), "{rules}");
    assert!(rules.contains("delay: 120ms"), "{rules}");
    assert!(rules.contains("send: pong"), "{rules}");

    // Binary pairing: binary_base64 match -> send_binary reply.
    assert!(rules.contains("binary_base64: AAEC"), "{rules}");
    assert!(rules.contains("send_binary: //79"), "{rules}");
    assert!(rules.contains("delay: 200ms"), "{rules}");

    // The lowered definition is a real streaming ws mock (GET + upgrade).
    let def = mock.clone().into_mock_definition().await.unwrap();
    assert!(
        def.streaming
            .as_ref()
            .is_some_and(mockpit::types::StreamingResponse::is_ws)
    );
    assert_eq!(
        def.request.methods,
        smallvec::SmallVec::<[http::Method; 2]>::from_elem(http::Method::GET, 1)
    );
}

#[tokio::test]
async fn test_ambiguous_websocket_pairing_folds_into_connect_sequence() {
    // "ping" recurs with different replies -> exact rules would be wrong.
    let mocks = load_har_string(&ws_har_fixture(
        r#"
        { "type": "send", "time": 1751364000.0, "opcode": 1, "data": "ping" },
        { "type": "receive", "time": 1751364000.1, "opcode": 1, "data": "pong-1" },
        { "type": "send", "time": 1751364001.0, "opcode": 1, "data": "ping" },
        { "type": "receive", "time": 1751364001.1, "opcode": 1, "data": "pong-2" }
"#,
    ))
    .await;

    assert_eq!(mocks.len(), 1);
    let ws = mocks[0].ws.as_ref().expect("ws config");
    assert!(
        ws.on_message.is_empty(),
        "ambiguous pairing must not build rules"
    );
    let yaml = serde_yaml::to_string(&ws.on_connect).unwrap();
    assert!(yaml.contains("pong-1"), "{yaml}");
    assert!(yaml.contains("pong-2"), "{yaml}");
    // Inter-frame delay between the two server frames (1751364001.1 - 1751364000.1).
    assert!(yaml.contains("1000ms"), "{yaml}");
}
