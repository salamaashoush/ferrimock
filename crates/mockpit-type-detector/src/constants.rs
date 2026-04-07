//! Constants and compiled regex patterns for type detection

use lazy_static::lazy_static;
use regex::Regex;

// Global compiled regexes using lazy_static for zero-cost abstraction
lazy_static! {
  // URL regex: matches http(s) URLs including localhost, IP addresses, and domains with TLDs
  pub(super) static ref URL_REGEX: Regex = Regex::new(r"^https?://[^\s/]+(/[^\s]*)?$").expect("valid regex");
  pub(super) static ref UUID_REGEX: Regex =
    Regex::new(r"^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$").expect("valid regex");
  pub(super) static ref TIMESTAMP_REGEX: Regex = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}").expect("valid regex");
  pub(super) static ref EMAIL_REGEX: Regex = Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("valid regex");
  pub(super) static ref IP_REGEX: Regex = Regex::new(r"^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}$").expect("valid regex");
  // Phone regex: supports extensions (x1234, ext123, #456) and dots
  pub(super) static ref PHONE_REGEX: Regex = Regex::new(r"^[\d\s\-\(\)\+\.xext#]+$").expect("valid regex");
  pub(super) static ref SEMVER_REGEX: Regex =
    Regex::new(r"^\d+\.\d+\.\d+(-[a-zA-Z0-9.-]+)?(\+[a-zA-Z0-9.-]+)?$").expect("valid regex");
  pub(super) static ref HEX_REGEX: Regex = Regex::new(r"^[a-fA-F0-9]+$").expect("valid regex");
  // HTTP ETag pattern: quoted strings (strong) or W/"..." (weak)
  // Examples: "abc123", W/"123", "686897696a7c876b7e"
  pub(super) static ref ETAG_REGEX: Regex = Regex::new(r#"^(W/)?"[^"]*"$"#).expect("valid regex");
  pub(super) static ref ISO_DATE_REGEX: Regex = Regex::new(r"^\d{4}-\d{2}-\d{2}$").expect("valid regex");
  pub(super) static ref DATA_URI_REGEX: Regex = Regex::new(r"^data:([a-zA-Z0-9]+/[a-zA-Z0-9\-+.]+);base64,").expect("valid regex");
}

/// Confidence thresholds for different field types
pub(super) const CONFIDENCE_DOWNLOAD_URL: f64 = 0.85;
pub(super) const CONFIDENCE_DATA_URI: f64 = 0.95;
pub(super) const CONFIDENCE_URL: f64 = 0.90;
pub(super) const CONFIDENCE_EMAIL: f64 = 0.85;
pub(super) const CONFIDENCE_TIMESTAMP: f64 = 0.85;
pub(super) const CONFIDENCE_NUMERIC_STRING_ID: f64 = 0.80;
pub(super) const CONFIDENCE_UUID: f64 = 0.95;
pub(super) const CONFIDENCE_FILENAME: f64 = 0.75;
pub(super) const CONFIDENCE_ETAG: f64 = 0.70;
pub(super) const CONFIDENCE_TOKEN: f64 = 0.70;
pub(super) const CONFIDENCE_MIME_TYPE: f64 = 0.75;
pub(super) const CONFIDENCE_IP_ADDRESS: f64 = 0.85;
pub(super) const CONFIDENCE_PHONE_NUMBER: f64 = 0.70;
pub(super) const CONFIDENCE_NAME: f64 = 0.65;
pub(super) const CONFIDENCE_SEMVER: f64 = 0.85;
pub(super) const CONFIDENCE_HEX_STRING: f64 = 0.80;
pub(super) const CONFIDENCE_BASE64: f64 = 0.75;

/// Internal threshold constants for match ratios
pub(super) const MIN_MATCH_RATIO_EMAIL: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_DOWNLOAD_URL: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_DATA_URI: f64 = 0.95;
pub(super) const MIN_MATCH_RATIO_URL: f64 = 0.6;
pub(super) const MIN_MATCH_RATIO_UUID: f64 = 0.95;
pub(super) const MIN_MATCH_RATIO_TIMESTAMP: f64 = 0.8;
pub(super) const MIN_MATCH_RATIO_NUMERIC_STRING_ID: f64 = 0.8;
pub(super) const MIN_MATCH_RATIO_PHONE: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_IP_ADDRESS: f64 = 0.8;
pub(super) const MIN_MATCH_RATIO_SEMVER: f64 = 0.8;
pub(super) const MIN_MATCH_RATIO_ETAG: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_BASE64: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_HEX_STRING: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_TOKEN: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_MIME_TYPE: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_NAME: f64 = 0.6;
pub(super) const MIN_MATCH_RATIO_API_ENDPOINT: f64 = 0.7;
pub(super) const MIN_MATCH_RATIO_FILENAME: f64 = 0.5;
pub(super) const MIN_MATCH_RATIO_COUNTRY_CODE: f64 = 0.8;
pub(super) const MIN_MATCH_RATIO_CURRENCY_CODE: f64 = 0.8;
pub(super) const MIN_MATCH_RATIO_FILE_PATH: f64 = 0.7;

/// Common pagination parameter keys
pub(super) const PAGE_KEYS: &[&str] = &["page", "page_number", "p", "offset", "skip"];
pub(super) const LIMIT_KEYS: &[&str] = &["limit", "per_page", "page_size", "size", "l", "count"];
pub(super) const CURSOR_KEYS: &[&str] = &[
    "cursor",
    "next_token",
    "continuation_token",
    "after",
    "before",
    "start",
];
