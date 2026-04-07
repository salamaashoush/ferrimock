//! Core type definitions for type detection system

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value as JsonValue;
use url::Url;

use super::constants::{CURSOR_KEYS, LIMIT_KEYS, PAGE_KEYS};

/// Extended field type enumeration with specialized types
#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    /// Sequential number (1, 2, 3, 4...)
    SequentialNumber { start: i64, step: i64 },
    /// Random/varying number with optional range from sample data
    RandomNumber { min: Option<i64>, max: Option<i64> },
    /// Random floating point number with optional range from sample data
    RandomFloat { min: Option<f64>, max: Option<f64> },
    /// UUID pattern (v4 format)
    Uuid,
    /// ISO 8601 timestamp
    Timestamp,
    /// Email address
    Email,
    /// Username/login (alphanumeric identifier without spaces)
    Username,
    /// Person name (contains space, starts with capital)
    Name,
    /// Single sentence of text (ends with punctuation, not too long)
    Sentence,
    /// Multiple sentences forming a paragraph
    Paragraph,
    /// URL/URI with protocol
    Url,
    /// Image URL (avatar, icon, thumbnail, photo, etc.)
    ImageUrl,
    /// IP address (IPv4)
    IpAddress,
    /// Phone number
    PhoneNumber,
    /// File name with extension
    FileName,
    /// File size in bytes
    FileSize,
    /// Very long download URL - stores sample URL to detect file type
    DownloadUrl { sample_url: Option<String> },
    /// Data URI (data:image/png;base64,... or data:application/pdf;base64,...)
    DataUri { mime_type: Option<String> },
    /// Authentication token or JWT
    Token,
    /// HTTP ETag header value
    ETag,
    /// MIME type (content-type)
    MimeType,
    /// Random string (no clear pattern)
    RandomString,
    /// Boolean value that varies
    Boolean,
    /// Constant value (same across all responses)
    Constant(JsonValue),
    /// Array of items with homogeneous structure
    Array(Box<ArrayPattern>),
    /// Nested object with analyzed structure
    Object(Box<ObjectAnalysis>),
    /// Numeric string ID (long digit-only strings)
    NumericStringId,
    /// Pagination URL (URLs with page/limit params) - stores pattern for smart generation
    PaginationUrl(Box<PaginationUrlPattern>),
    /// API endpoint (relative paths)
    ApiEndpoint,
    /// ISO date without time
    IsoDate,
    /// Unix timestamp (numeric, seconds)
    UnixTimestamp,
    /// Unix timestamp in milliseconds
    MillisecondTimestamp,
    /// Unix timestamp in microseconds
    MicrosecondTimestamp,
    /// Semantic version string
    Semver,
    /// Hexadecimal string
    HexString,
    /// Base64-encoded data
    Base64,
    /// Latitude coordinate (-90 to 90)
    Latitude,
    /// Longitude coordinate (-180 to 180)
    Longitude,
    /// Categorical/Enum string (low cardinality)
    Categorical { values: Vec<String> },
    /// ISO 3166-1 alpha-2 country code
    CountryCode,
    /// ISO 4217 currency code
    CurrencyCode,
    /// File system path
    FilePath,
    /// Postal/ZIP code (various formats: US, UK, CA, etc.)
    PostalCode,
    /// Locale code (e.g., en-US, fr-FR, ja-JP)
    LocaleCode,
    /// IANA timezone identifier (e.g., America/New_York, Europe/London)
    Timezone,
}

/// Analysis of array patterns
#[derive(Debug, Clone, PartialEq)]
pub struct ArrayPattern {
    /// Element type if all elements have same type
    pub element_type: FieldType,
    /// Whether all elements have the same structure
    pub is_homogeneous: bool,
    /// Sample size for generating arrays
    pub sample_size_range: (usize, usize),
}

/// Analysis of nested object structures
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectAnalysis {
    /// Fields that vary across responses
    pub varying_fields: Vec<(String, FieldType)>,
    /// Fields that are constant across all responses
    pub constant_fields: Vec<(String, JsonValue)>,
}

/// Represents the detected structure of a pagination URL
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginationUrlPattern {
    /// The base URL without query params (e.g., "http://localhost:3003/api/v1/documents-search/")
    pub base_url: String,
    /// Query parameters that were present but did not change across samples
    pub static_params: Vec<(String, String)>,
    /// The pagination strategy detected (page/limit or cursor)
    pub pagination_scheme: PaginationScheme,
}

/// Pagination strategy enumeration
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationScheme {
    /// For page/offset based pagination
    PageBased {
        page_key: String,
        limit_key: Option<String>,
        /// Sample page number to know where to start generating from
        sample_page: u64,
        sample_limit: Option<u64>,
    },
    /// For cursor-based pagination
    CursorBased {
        cursor_key: String,
        /// Sample cursor to use as a placeholder
        sample_cursor: String,
    },
}

/// Direction for pagination URL generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaginationDirection {
    Next,
    Previous,
}

impl PaginationUrlPattern {
    /// Generate a URL for the specified pagination direction
    pub fn generate_url(&self, direction: PaginationDirection) -> String {
        let mut params = self.static_params.clone();

        match &self.pagination_scheme {
            PaginationScheme::PageBased {
                page_key,
                limit_key,
                sample_page,
                sample_limit,
            } => {
                let current_page = *sample_page;
                let new_page = match direction {
                    PaginationDirection::Next => current_page + 1,
                    PaginationDirection::Previous => {
                        if current_page > 1 {
                            current_page - 1
                        } else {
                            1
                        }
                    }
                };
                params.push((page_key.clone(), new_page.to_string()));
                if let (Some(lk), Some(sl)) = (limit_key, sample_limit) {
                    // Remove static limit if it exists to avoid duplication
                    params.retain(|(k, _)| k != lk);
                    params.push((lk.clone(), sl.to_string()));
                }
            }
            PaginationScheme::CursorBased { cursor_key, .. } => {
                // Generate a random placeholder for cursor (can't predict next cursor)
                let new_cursor = format!(
                    "CURSOR_{}",
                    uuid::Uuid::new_v4().to_string().replace('-', "")
                );
                params.push((cursor_key.clone(), new_cursor));
            }
        }

        // Reconstruct the query string
        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        format!("{}?{}", self.base_url.trim_end_matches('?'), query_string)
    }
}

/// Analyzes a set of URL strings to find a pagination pattern.
///
/// This function parses multiple URLs, compares their query parameters,
/// and attempts to identify pagination patterns (page-based or cursor-based).
///
/// # Arguments
/// * `values` - Array of URL strings to analyze
///
/// # Returns
/// * `Some(PaginationUrlPattern)` if a clear pagination pattern is detected
/// * `None` if URLs cannot be parsed or no pagination pattern is found
pub(super) fn analyze_pagination_pattern(values: &[&str]) -> Option<PaginationUrlPattern> {
    // Need at least 2 samples to compare and detect patterns
    if values.len() < 2 {
        return None;
    }

    // Parse all URLs - return None if any fail
    let parsed_urls: Vec<Url> = values.iter().filter_map(|s| Url::parse(s).ok()).collect();
    if parsed_urls.len() != values.len() {
        return None;
    }

    // 1. Check for a common base URL (scheme + host + port + path must match)
    let first_url = parsed_urls.first()?;

    // Build base URL with port if present
    let host_with_port = if let Some(port) = first_url.port() {
        format!("{}:{}", first_url.host_str()?, port)
    } else {
        first_url.host_str()?.to_string()
    };

    let base_url = format!(
        "{}://{}{}",
        first_url.scheme(),
        host_with_port,
        first_url.path()
    );

    // Verify all URLs share the same base
    if !parsed_urls.iter().all(|u| {
        let u_host_with_port = if let Some(port) = u.port() {
            format!("{}:{}", u.host_str().unwrap_or_default(), port)
        } else {
            u.host_str().unwrap_or_default().to_string()
        };
        format!("{}://{}{}", u.scheme(), u_host_with_port, u.path()) == base_url
    }) {
        return None;
    }

    // 2. Collect all query parameters from all URLs
    let mut param_values: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    for url in &parsed_urls {
        for (key, value) in url.query_pairs() {
            param_values
                .entry(key.to_string())
                .or_default()
                .insert(value.to_string());
        }
    }

    // 3. Classify parameters as static (same value) or dynamic (multiple values)
    let mut static_params = Vec::new();
    let mut dynamic_keys = FxHashSet::default();

    for (key, unique_values) in param_values {
        if unique_values.len() == 1 {
            // Only one value ever seen - it's static
            if let Some(value) = unique_values.into_iter().next() {
                static_params.push((key, value));
            }
        } else {
            // Multiple values seen - it's dynamic
            dynamic_keys.insert(key);
        }
    }

    // 4. Identify pagination scheme - check cursor-based first (more specific)
    if let Some(cursor_key) = CURSOR_KEYS.iter().find(|&k| dynamic_keys.contains(*k)) {
        // Cursor-based pagination detected
        let sample_cursor = first_url
            .query_pairs()
            .find(|(k, _)| k == *cursor_key)
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();

        return Some(PaginationUrlPattern {
            base_url,
            static_params,
            pagination_scheme: PaginationScheme::CursorBased {
                cursor_key: (*cursor_key).to_string(),
                sample_cursor,
            },
        });
    }

    // Check for page-based pagination
    if let Some(page_key) = PAGE_KEYS.iter().find(|&k| dynamic_keys.contains(*k)) {
        // Page-based pagination detected
        // Check if limit is static or dynamic
        let limit_key = LIMIT_KEYS.iter().find(|&k| {
            static_params.iter().any(|(sp_k, _)| sp_k == *k) || dynamic_keys.contains(*k)
        });

        let sample_page = first_url
            .query_pairs()
            .find(|(k, _)| k == *page_key)
            .and_then(|(_, v)| v.parse::<u64>().ok())
            .unwrap_or(1);

        let sample_limit = limit_key.and_then(|lk| {
            // Try to find in both static params and URL query
            static_params
                .iter()
                .find(|(sp_k, _)| sp_k == *lk)
                .and_then(|(_, v)| v.parse::<u64>().ok())
                .or_else(|| {
                    first_url
                        .query_pairs()
                        .find(|(k, _)| k == *lk)
                        .and_then(|(_, v)| v.parse::<u64>().ok())
                })
        });

        return Some(PaginationUrlPattern {
            base_url,
            static_params,
            pagination_scheme: PaginationScheme::PageBased {
                page_key: (*page_key).to_string(),
                limit_key: limit_key.map(|s| (*s).to_string()),
                sample_page,
                sample_limit,
            },
        });
    }

    // No recognizable pagination pattern found
    None
}
