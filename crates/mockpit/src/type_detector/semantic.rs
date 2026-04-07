//! Semantic analysis based on field names and context

use serde_json::Value as JsonValue;
use url::Url;

use super::checkers::{calculate_email_confidence, calculate_url_confidence};
use super::constants::*;
use super::types::{FieldType, PaginationScheme, PaginationUrlPattern, analyze_pagination_pattern};

/// Helper function for case-insensitive contains check
pub fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Helper function for case-insensitive ends_with check
pub fn ends_with_ignore_case(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().ends_with(&needle.to_lowercase())
}

/// Check if field name matches a pattern in ANY naming convention
///
/// Handles all common naming conventions:
/// - snake_case: "user_name", "updated_at"
/// - camelCase: "userName", "updatedAt"
/// - PascalCase: "UserName", "UpdatedAt"
/// - kebab-case: "user-name", "updated-at"
/// - SCREAMING_SNAKE_CASE: "USER_NAME", "UPDATED_AT"
/// - All lowercase: "username", "updatedat"
/// - All uppercase: "USERNAME", "UPDATEDAT"
///
/// # Examples
/// ```
/// # use mockpit::type_detector::semantic::matches_field_name;
/// assert!(matches_field_name("updated_at", "updated_at"));
/// assert!(matches_field_name("updatedAt", "updated_at"));
/// assert!(matches_field_name("UpdatedAt", "updated_at"));
/// assert!(matches_field_name("updated-at", "updated_at"));
/// assert!(matches_field_name("UPDATED_AT", "updated_at"));
/// assert!(matches_field_name("updatedat", "updated_at"));
/// ```
pub fn matches_field_name(field_name: &str, pattern: &str) -> bool {
    // Normalize both to lowercase, removing _ and - for comparison
    let normalized_field = field_name.to_lowercase().replace(['_', '-'], "");
    let normalized_pattern = pattern.to_lowercase().replace(['_', '-'], "");

    if normalized_field == normalized_pattern {
        return true;
    }

    // Also check exact match (case-insensitive)
    field_name.eq_ignore_ascii_case(pattern)
}

/// Check if field name matches ANY of the provided patterns in ANY naming convention
///
/// This is the main helper to use for field name checking. It handles:
/// - Multiple pattern alternatives (e.g., `["login", "user_name", "username"]`)
/// - All naming conventions for each pattern
///
/// # Examples
/// ```
/// # use mockpit::type_detector::semantic::matches_any_field_name;
/// // Check for login/username fields
/// assert!(matches_any_field_name("login", &["login", "user_name", "username"]));
/// assert!(matches_any_field_name("userName", &["login", "user_name", "username"]));
/// assert!(matches_any_field_name("user-name", &["login", "user_name", "username"]));
/// assert!(matches_any_field_name("USERNAME", &["login", "user_name", "username"]));
/// ```
pub fn matches_any_field_name(field_name: &str, patterns: &[&str]) -> bool {
    patterns
        .iter()
        .any(|pattern| matches_field_name(field_name, pattern))
}

/// Check if field name CONTAINS a pattern in ANY naming convention
///
/// Useful for checking if a field name contains a keyword like "time", "date", "id", etc.
///
/// # Examples
/// ```
/// # use mockpit::type_detector::semantic::contains_field_pattern;
/// assert!(contains_field_pattern("created_at", "created"));
/// assert!(contains_field_pattern("createdAt", "created"));
/// assert!(contains_field_pattern("updated_time", "time"));
/// assert!(contains_field_pattern("updatedTime", "time"));
/// ```
pub fn contains_field_pattern(field_name: &str, pattern: &str) -> bool {
    // Normalize to lowercase for contains check
    let normalized_field = field_name.to_lowercase();
    let normalized_pattern = pattern.to_lowercase();

    // Check if pattern is contained directly
    if normalized_field.contains(&normalized_pattern) {
        return true;
    }

    // Check if pattern is contained when removing separators
    let field_no_sep = normalized_field.replace(['_', '-'], "");
    let pattern_no_sep = normalized_pattern.replace(['_', '-'], "");

    field_no_sep.contains(&pattern_no_sep)
}

/// Check if field name CONTAINS ANY of the provided patterns in ANY naming convention
///
/// Useful for checking if a field name contains any keyword from a list.
///
/// # Examples
/// ```
/// # use mockpit::type_detector::semantic::contains_any_field_pattern;
/// // Check for status-related fields
/// assert!(contains_any_field_pattern("user_status", &["status", "state", "type"]));
/// assert!(contains_any_field_pattern("itemType", &["status", "state", "type"]));
/// assert!(contains_any_field_pattern("current-state", &["status", "state", "type"]));
/// ```
pub fn contains_any_field_pattern(field_name: &str, patterns: &[&str]) -> bool {
    patterns
        .iter()
        .any(|pattern| contains_field_pattern(field_name, pattern))
}

/// Check if field name ENDS WITH a pattern in ANY naming convention
///
/// Useful for suffix matching like "_id", "_at", "_url", etc.
///
/// # Examples
/// ```
/// # use mockpit::type_detector::semantic::ends_with_field_pattern;
/// assert!(ends_with_field_pattern("user_id", "id"));
/// assert!(ends_with_field_pattern("userId", "id"));
/// assert!(ends_with_field_pattern("created_at", "at"));
/// assert!(ends_with_field_pattern("createdAt", "at"));
/// ```
pub fn ends_with_field_pattern(field_name: &str, pattern: &str) -> bool {
    // Normalize to lowercase for ends_with check
    let normalized_field = field_name.to_lowercase();
    let normalized_pattern = pattern.to_lowercase();

    // Check direct suffix
    if normalized_field.ends_with(&normalized_pattern) {
        return true;
    }

    // Check suffix with common separators
    if normalized_field.ends_with(&format!("_{normalized_pattern}"))
        || normalized_field.ends_with(&format!("-{normalized_pattern}"))
    {
        return true;
    }

    // Check camelCase/PascalCase endings (e.g., "userId" ends with "Id")
    let field_no_sep = normalized_field.replace(['_', '-'], "");
    let pattern_no_sep = normalized_pattern.replace(['_', '-'], "");

    field_no_sep.ends_with(&pattern_no_sep)
}

/// Detect field type from field name ONLY (no sample values required)
/// Optimized for GraphQL introspection where we only have schema information
///
/// This function provides field type detection based purely on naming conventions,
/// without requiring actual sample data. It covers common patterns that can be
/// reliably inferred from field names alone.
pub fn detect_from_field_name_only(field_name: &str) -> Option<(FieldType, f64)> {
    // Person names - full names, first names, last names
    // Handles: name, fullName, firstName, lastName, displayName, etc.
    if matches_any_field_name(
        field_name,
        &[
            "name",
            "full_name",
            "fullname",
            "first_name",
            "last_name",
            "display_name",
        ],
    ) || (matches_field_name(field_name, "name")
        && !contains_any_field_pattern(field_name, &["user", "file", "host", "domain"]))
    {
        return Some((FieldType::Name, 0.90));
    }

    // Titles and short text
    // Handles: title, headline, subject, caption, label, heading
    if matches_any_field_name(
        field_name,
        &[
            "title", "headline", "subject", "caption", "label", "heading", "tagline", "slogan",
        ],
    ) {
        return Some((FieldType::Sentence, 0.90));
    }

    // Descriptions and long text
    // Handles: description, bio, about, summary, details, content, overview
    if matches_any_field_name(
        field_name,
        &[
            "description",
            "bio",
            "about",
            "summary",
            "details",
            "overview",
            "narrative",
            "story",
        ],
    ) || (contains_field_pattern(field_name, "description")
        && !contains_field_pattern(field_name, "short"))
    {
        return Some((FieldType::Paragraph, 0.90));
    }

    // Image/Avatar URLs - fields that typically contain image URLs
    // Handles: gravatar, avatar, icon, thumbnail, logo, image, picture, photo
    if matches_any_field_name(
        field_name,
        &[
            "gravatar",
            "avatar",
            "icon",
            "thumbnail",
            "logo",
            "image",
            "picture",
            "photo",
            "portrait",
        ],
    ) || contains_any_field_pattern(
        field_name,
        &[
            "avatar",
            "icon",
            "thumbnail",
            "gravatar",
            "picture",
            "photo",
        ],
    ) {
        return Some((FieldType::ImageUrl, 0.85));
    }

    // Cryptographic hashes
    // Handles: sha1, sha, sha256, hash, md5, checksum, fingerprint
    if matches_any_field_name(
        field_name,
        &[
            "sha1",
            "sha",
            "sha256",
            "sha512",
            "hash",
            "md5",
            "checksum",
            "fingerprint",
            "digest",
        ],
    ) || (contains_field_pattern(field_name, "hash")
        && !contains_field_pattern(field_name, "tag"))
    {
        return Some((FieldType::HexString, 0.90));
    }

    // GraphQL cursors - opaque pagination cursors
    // Handles: cursor, nextCursor, prevCursor, startCursor, endCursor
    if matches_field_name(field_name, "cursor") || contains_field_pattern(field_name, "cursor") {
        return Some((FieldType::RandomString, 0.90));
    }

    // Origin/source fields - typically enums or short strings
    // Handles: origin, source
    if matches_any_field_name(field_name, &["origin", "source"]) {
        return Some((FieldType::Sentence, 0.80));
    }

    // Reference fields - typically IDs or identifiers
    // Handles: reference, ref
    if contains_field_pattern(field_name, "reference") || ends_with_field_pattern(field_name, "ref")
    {
        return Some((FieldType::RandomString, 0.80));
    }

    None
}

/// Layer 1: Detect from semantic context (field names)
#[allow(clippy::cast_precision_loss)]
pub fn detect_from_semantic_context(
    field_name: &str,
    values: &[&JsonValue],
) -> Option<(FieldType, f64)> {
    // Numeric ID fields - check field name ends with _id or is "id"
    // MUST CHECK THIS FIRST before any value analysis to catch numeric JSON values
    // This works with both numeric JSON values and string values
    if ends_with_field_pattern(field_name, "id") || matches_field_name(field_name, "id") {
        // Check if values are large integers (either as JSON numbers or as numeric strings)
        // A number has >= 10 digits if its absolute value >= 1_000_000_000
        const MIN_10_DIGIT: i64 = 1_000_000_000;
        const MIN_10_DIGIT_U: u64 = 1_000_000_000;
        const MIN_10_DIGIT_F: f64 = 1_000_000_000.0;

        let all_large_integers = values.iter().all(|v| {
            if let Some(num) = v.as_i64() {
                num.abs() >= MIN_10_DIGIT
            } else if let Some(num) = v.as_u64() {
                num >= MIN_10_DIGIT_U
            } else if let Some(num) = v.as_f64() {
                // JSON float that might represent a large integer
                let abs = num.abs();
                abs.fract() == 0.0 && abs >= MIN_10_DIGIT_F
            } else if let Some(s) = v.as_str() {
                // String value - check if it's a long numeric string
                s.len() >= 10 && s.chars().all(|c| c.is_ascii_digit())
            } else {
                false
            }
        });

        if all_large_integers {
            return Some((FieldType::NumericStringId, 0.95));
        }
    }

    // ETag fields - smart pattern matching (but avoid conflict with Semver)
    // "revision" alone is ambiguous - could be Semver or ETag
    // Prefer compound patterns or strong signals
    let suggests_etag = contains_field_pattern(field_name, "etag")
        || (contains_field_pattern(field_name, "version")
            && contains_field_pattern(field_name, "hash"))
        || (contains_field_pattern(field_name, "version")
            && contains_field_pattern(field_name, "tag"))
        || (contains_field_pattern(field_name, "cache")
            && contains_field_pattern(field_name, "tag"))
        || (contains_field_pattern(field_name, "entity")
            && contains_field_pattern(field_name, "tag"))
        || (contains_field_pattern(field_name, "checksum")
            && contains_field_pattern(field_name, "tag"))
        || (contains_field_pattern(field_name, "content")
            && contains_field_pattern(field_name, "hash"))
        || (contains_field_pattern(field_name, "object")
            && contains_field_pattern(field_name, "version"))
        || contains_field_pattern(field_name, "fingerprint");

    if suggests_etag {
        return Some((FieldType::ETag, 0.95));
    }

    // Login/username fields - field-name-based detection without needing sample values
    // MUST CHECK THIS EARLY to avoid being caught by later string-based detections
    // These are short alphanumeric identifiers, NOT full names with spaces
    // Handles: login, username, user_name, userName, UserName, user-name, LOGIN, USERNAME, etc.
    if matches_any_field_name(field_name, &["login", "username", "user_name"]) {
        // These should generate proper usernames (e.g., "johndoe", "janesmith")
        return Some((FieldType::Username, 0.95));
    }

    // Longitude fields - MUST check before Latitude since longitude range includes latitude range
    // Smart detection: only match when lon/lng is a distinct component, not part of other words
    // Handles: lng, lon, longitude, LNG, LON, Longitude, x, X, etc.
    // Also handles: lng_coord, lonCoord, coordinate_lng, etc.
    if matches_any_field_name(field_name, &["lng", "lon", "longitude", "x"])
        || ends_with_field_pattern(field_name, "lng")
        || ends_with_field_pattern(field_name, "lon")
        || ends_with_field_pattern(field_name, "longitude")
    {
        // High confidence just from field name for longitude-specific names
        return Some((FieldType::Longitude, 0.95));
    }

    // Latitude fields - Smart detection: only match when lat is a distinct component
    // Handles: lat, latitude, y, LAT, LATITUDE, Y, latCoord, coordinate_lat, etc.
    if matches_any_field_name(field_name, &["lat", "latitude", "y"])
        || ends_with_field_pattern(field_name, "lat")
        || ends_with_field_pattern(field_name, "latitude")
    {
        // High confidence just from field name for latitude-specific names
        return Some((FieldType::Latitude, 0.95));
    }

    // FileSize fields - intelligent context-based detection
    // Detect fields representing digital storage sizes (bytes, file sizes, etc.)

    // Storage/digital context - things that have byte sizes
    let suggests_storage_context = contains_any_field_pattern(
        field_name,
        &[
            "file",
            "blob",
            "object",
            "data",
            "attachment",
            "download",
            "upload",
            "content",
            "body",
            "byte",
        ],
    );

    // Measurement context - size/length indicators
    let suggests_size_measurement = contains_any_field_pattern(field_name, &["size", "length"]);

    // Physical/non-digital contexts - NOT file sizes
    let suggests_physical_measurement = contains_any_field_pattern(
        field_name,
        &["height", "width", "depth", "distance", "duration"],
    );

    // FileSize = measurement + storage context, OR just "size"/"length" (common in APIs)
    // Exclude physical measurements
    let suggests_filesize = suggests_size_measurement
        && (suggests_storage_context
            || !(suggests_physical_measurement
                || contains_any_field_pattern(field_name, &["array", "list"])));

    if suggests_filesize {
        // Check if values are large integers (typical file sizes are > 1000 bytes)
        let all_large_numbers = values.iter().all(|v| {
            if let Some(num) = v.as_i64() {
                num >= 1000
            } else if let Some(num) = v.as_u64() {
                num >= 1000
            } else if let Some(num) = v.as_f64() {
                num >= 1000.0 && num.fract() == 0.0
            } else if let Some(s) = v.as_str() {
                s.parse::<i64>().is_ok_and(|n| n >= 1000)
            } else {
                false
            }
        });

        if all_large_numbers {
            return Some((FieldType::FileSize, 0.95));
        }
    }

    // Float/decimal detection removed - let value-based analysis handle it
    // The analyzers.rs will correctly detect floats vs integers based on actual data

    // SequentialNumber fields - smart pattern matching
    // Use strong field name signals even if values aren't perfectly sequential
    let suggests_sequential = contains_any_field_pattern(
        field_name,
        &[
            "seq",
            "sequence",
            "order",
            "position",
            "rank",
            "step",
            "counter",
            "iteration",
            "index",
        ],
    ) || matches_field_name(field_name, "number");

    if suggests_sequential {
        // Try to parse as integers
        let nums: Vec<i64> = values
            .iter()
            .filter_map(|v| {
                if let Some(n) = v.as_i64() {
                    Some(n)
                } else if let Some(s) = v.as_str() {
                    s.parse::<i64>().ok()
                } else {
                    None
                }
            })
            .collect();

        // If all values are integers and field name is strong signal
        if nums.len() == values.len() && !nums.is_empty() {
            // Check if values are reasonable for a sequence (small, low numbers - not IDs or large counts)
            let max_value = nums.iter().max().copied().unwrap_or(0);
            let min_value = nums.iter().min().copied().unwrap_or(0);
            let range = max_value - min_value;

            // Sequential fields typically have small ranges and start from low numbers
            // If max > 10000 OR range > 1000, it's probably RandomNumber not sequential
            let looks_sequential = min_value >= 0 && max_value < 10000 && range < 1000;

            if looks_sequential {
                // Try to detect if actually sequential (for high confidence)
                if nums.len() >= 2 {
                    let mut sorted_nums = nums;
                    sorted_nums.sort_unstable();
                    let is_sequential = sorted_nums
                        .windows(2)
                        .all(|w| matches!((w.first(), w.get(1)), (Some(a), Some(b)) if b - a == 1));

                    if is_sequential && let Some(&start) = sorted_nums.first() {
                        return Some((FieldType::SequentialNumber { start, step: 1 }, 0.95));
                    }
                }

                // Strong field name + small numbers = likely sequential (lower confidence)
                let start = min_value;
                return Some((FieldType::SequentialNumber { start, step: 1 }, 0.80));
            }
        }
    }

    // Constant fields - all values are identical (version numbers, API versions, etc.)
    // Check this BEFORE extracting strings so we can handle any JSON type
    let suggests_constant = contains_any_field_pattern(
        field_name,
        &["version", "constant", "default", "fixed", "standard"],
    );

    if suggests_constant && values.len() >= 2 {
        // Check if all values are identical (using JSON comparison)
        if let Some(first) = values.first() {
            let all_same = values.iter().all(|v| v == first);

            if all_same {
                // All values are identical - return as Constant
                return Some((FieldType::Constant((*first).clone()), 0.95));
            }
        }
    }

    // Extract string values for validation (filter out nulls and non-strings)
    let strs: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();

    if !strs.is_empty() {
        // ETag value-based detection for ambiguous field names like "revision"
        // Detect quoted MD5/SHA hashes which are common ETag formats
        let suggests_revision =
            contains_any_field_pattern(field_name, &["revision", "version", "checksum"]);

        if suggests_revision {
            // Check if values look like quoted hash strings (ETags)
            let looks_like_etag = strs.iter().any(|s| {
                // Quoted 32-char hex string (MD5) or 40-char (SHA1) or 64-char (SHA256)
                if s.starts_with('"') && s.ends_with('"') {
                    let inner = s.get(1..s.len() - 1).unwrap_or_default();
                    (inner.len() == 32 || inner.len() == 40 || inner.len() == 64)
                        && inner.chars().all(|c| c.is_ascii_hexdigit())
                } else {
                    false
                }
            });

            if looks_like_etag {
                return Some((FieldType::ETag, 0.95));
            }
        }

        // LocaleCode detection - language/locale codes (e.g., "en-US", "fr-FR")
        // IMPORTANT: Exclude fields containing "country" - those are CountryCode, not LocaleCode
        let suggests_locale = (contains_any_field_pattern(
            field_name,
            &[
                "locale",
                "language",
                "lang",
                "l10n",
                "i18n",
                "culture",
                "region",
                "localization",
            ],
        ) || (contains_field_pattern(field_name, "code")
            && contains_field_pattern(field_name, "lang")))
            && !contains_field_pattern(field_name, "country");

        if suggests_locale {
            // Locale codes: 2-letter language code, optionally followed by -XX country code
            let looks_like_locale = strs.iter().all(|s| {
                let parts: Vec<&str> = s.split('-').collect();
                match (parts.first(), parts.get(1)) {
                    (Some(lang), None) => lang.len() == 2, // "en"
                    (Some(lang), Some(region)) => lang.len() == 2 && region.len() == 2, // "en-US"
                    _ => false,
                }
            });

            if looks_like_locale {
                return Some((FieldType::LocaleCode, 0.95));
            }
        }

        // MimeType detection - content types (e.g., "application/json")
        let suggests_mimetype =
            contains_any_field_pattern(
                field_name,
                &[
                    "mime",
                    "content_type",
                    "media_type",
                    "content_format",
                    "file_type",
                    "asset_type",
                    "document_type",
                ],
            ) || matches_any_field_name(field_name, &["type", "format", "encoding"]);

        if suggests_mimetype {
            // MIME types: type/subtype format
            let looks_like_mime = strs.iter().any(|s| {
                s.contains('/')
                    && (s.starts_with("application/")
                        || s.starts_with("text/")
                        || s.starts_with("image/")
                        || s.starts_with("audio/")
                        || s.starts_with("video/")
                        || s.starts_with("multipart/"))
            });

            if looks_like_mime {
                return Some((FieldType::MimeType, 0.95));
            }
        }

        // Timezone detection - IANA timezone names (e.g., "America/New_York")
        let suggests_timezone = contains_any_field_pattern(
            field_name,
            &[
                "timezone",
                "time_zone",
                "tz",
                "tzinfo",
                "zoneinfo",
                "zone_info",
                "iana_timezone",
            ],
        ) || matches_field_name(field_name, "zone");

        if suggests_timezone {
            // Timezone format: Area/Location (e.g., "America/New_York", "Europe/London")
            let looks_like_timezone = strs.iter().any(|s| {
                s.contains('/') && s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            });

            if looks_like_timezone {
                return Some((FieldType::Timezone, 0.95));
            }
        }

        // Categorical fields - detect based on field name patterns suggesting enum-like values
        // Common patterns: status, state, type, category, level, priority, role, mode, kind
        if contains_any_field_pattern(field_name, &["status", "state", "type", "category", "level", "priority", "role", "mode", "kind", "stage", "phase"])
      // Values should be short strings (not UUIDs, URLs, etc.)
      && strs.iter().all(|s| {
        s.len() < 30
          && !UUID_REGEX.is_match(s)
          && !URL_REGEX.is_match(s)
          && !EMAIL_REGEX.is_match(s)
          && !s.contains('/')
          && !s.contains('\\')
      }) {
            // Get unique values only
            let mut unique_values: Vec<String> = strs.iter().map(|s| (*s).to_string()).collect();
            unique_values.sort();
            unique_values.dedup();

            // Check if values are numeric strings (should be detected as NumericStringId, not categorical)
            let all_numeric = unique_values.iter().all(|s| s.parse::<i64>().is_ok());
            if all_numeric {
                // Numeric strings should not be categorical - let NumericStringId checker handle them
                return None;
            }

            return Some((
                FieldType::Categorical {
                    values: unique_values,
                },
                0.90,
            ));
        }

        // Pagination URLs - rely primarily on URL content analysis, not field names
        // Attempt to analyze pagination pattern from URLs
        if let Some(pattern) = analyze_pagination_pattern(&strs) {
            // Check if field name also suggests pagination (extra confidence boost)
            let field_suggests_pagination =
                contains_any_field_pattern(field_name, &["next", "prev", "page"]);

            let confidence = if field_suggests_pagination {
                0.95
            } else {
                0.85
            };
            return Some((FieldType::PaginationUrl(Box::new(pattern)), confidence));
        }

        // Fallback: simple pattern check if full analysis fails
        // Look for common pagination query params in URLs
        if strs.iter().any(|s| {
            URL_REGEX.is_match(s)
                && (s.contains("page=")
                    || s.contains("limit=")
                    || s.contains("offset=")
                    || s.contains("cursor="))
        }) {
            // Field name suggests pagination or navigation
            if contains_any_field_pattern(field_name, &["next", "prev", "page", "link"]) {
                // Create a simple pattern from the first URL
                if let Some(first_url) = strs.first()
                    && let Ok(parsed) = Url::parse(first_url)
                {
                    // Include port if present
                    let host_with_port = if let Some(port) = parsed.port() {
                        format!("{}:{}", parsed.host_str().unwrap_or("example.com"), port)
                    } else {
                        parsed.host_str().unwrap_or("example.com").to_string()
                    };
                    let base_url =
                        format!("{}://{}{}", parsed.scheme(), host_with_port, parsed.path());
                    let static_params: Vec<(String, String)> = parsed
                        .query_pairs()
                        .filter(|(k, _)| {
                            !PAGE_KEYS.contains(&k.as_ref()) && !LIMIT_KEYS.contains(&k.as_ref())
                        })
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect();

                    let fallback_pattern = PaginationUrlPattern {
                        base_url,
                        static_params,
                        pagination_scheme: PaginationScheme::PageBased {
                            page_key: "page".to_string(),
                            limit_key: Some("limit".to_string()),
                            sample_page: 1,
                            sample_limit: Some(10),
                        },
                    };
                    return Some((FieldType::PaginationUrl(Box::new(fallback_pattern)), 0.80));
                }
            }
        }

        // Image URL fields - detect BEFORE generic URL
        // Check if field name suggests image AND values are URLs
        if (matches_any_field_name(
            field_name,
            &[
                "avatar",
                "gravatar",
                "icon",
                "thumbnail",
                "logo",
                "image",
                "picture",
                "photo",
                "portrait",
            ],
        ) || contains_any_field_pattern(
            field_name,
            &[
                "avatar",
                "icon",
                "thumbnail",
                "gravatar",
                "picture",
                "photo",
                "image",
            ],
        )) && strs.iter().any(|s| URL_REGEX.is_match(s))
        {
            let confidence = calculate_url_confidence(&strs);
            if confidence >= CONFIDENCE_URL {
                return Some((FieldType::ImageUrl, confidence));
            }
        }

        // URL fields (next, previous, link, href, url)
        // Check this AFTER pagination and image URL detection
        if matches_any_field_name(
            field_name,
            &["next", "previous", "link", "href", "url", "uri"],
        ) && strs.iter().any(|s| URL_REGEX.is_match(s))
        {
            let confidence = calculate_url_confidence(&strs);
            if confidence >= CONFIDENCE_URL {
                return Some((FieldType::Url, confidence));
            }
        }

        // ApiEndpoint fields - intelligent detection for API paths/routes
        // Must check values are actual paths (not URLs, tokens, versions, etc.)
        let looks_like_api_path = strs.iter().any(|s| {
            // API paths start with / and contain API patterns
            // Exclude full URLs (http/https), file extensions, and single words
            let is_path_like = s.starts_with('/') && !s.contains("://");
            let has_api_pattern = s.contains("/api")
                || s.contains("/v1")
                || s.contains("/v2")
                || s.contains("/v3")
                || s.contains("/users")
                || s.contains("/items")
                || s.contains("/resources");

            is_path_like && has_api_pattern
        });

        // Field name context for API paths - but ONLY if values match
        let suggests_api_path_field = contains_any_field_pattern(field_name, &["path", "route"])
            && contains_any_field_pattern(field_name, &["api", "service"]);

        if looks_like_api_path
            || (suggests_api_path_field && strs.iter().any(|s| s.starts_with('/')))
        {
            return Some((FieldType::ApiEndpoint, 0.95));
        }

        // Download URL fields - detect based on URL content patterns, not just field names
        // Check if URLs contain download/file indicators
        if strs.iter().any(|s| URL_REGEX.is_match(s)) {
            let has_download_pattern = strs.iter().any(|s| {
                s.contains("download")
                    || s.contains("/d/")
                    || s.contains("/dl/")
                    || s.contains("content")
                    || s.contains("attachment")
                    || s.contains("/file")
                    || super::is_custom_download_url(s)
                    || s.contains(".pdf")
                    || s.contains(".doc")
                    || s.contains(".zip")
            });

            // If URLs have download patterns AND field name suggests downloads/files/media/assets
            if has_download_pattern
                && contains_any_field_pattern(
                    field_name,
                    &[
                        "download",
                        "file",
                        "attachment",
                        "document",
                        "media",
                        "asset",
                        "resource",
                        "content",
                        "binary",
                        "stream",
                    ],
                )
            {
                let sample_url = strs
                    .iter()
                    .find(|s| URL_REGEX.is_match(s))
                    .map(|s| (*s).to_string());
                return Some((FieldType::DownloadUrl { sample_url }, 0.95));
            }
        }

        // ID fields - check for different ID types
        if ends_with_field_pattern(field_name, "id") || matches_field_name(field_name, "id") {
            // Check for long numeric string IDs first
            if strs
                .iter()
                .all(|s| s.len() > 10 && s.chars().all(|c| c.is_ascii_digit()))
            {
                return Some((FieldType::NumericStringId, 0.95));
            }
            // Then check for UUIDs
            if strs.iter().all(|s| UUID_REGEX.is_match(s)) {
                return Some((FieldType::Uuid, 0.98));
            }
        }

        // Token vs HexString - intelligent value-based distinction
        // Tokens are typically 32+ chars, hex colors are 6-8 chars (with #)
        // Exclude "token_data" - that's likely Base64-encoded data, not a token
        let suggests_token = (contains_field_pattern(field_name, "token")
            && !contains_field_pattern(field_name, "data"))
            || contains_any_field_pattern(field_name, &["jwt", "bearer"])
            || (contains_field_pattern(field_name, "api")
                && contains_any_field_pattern(field_name, &["key", "secret"]))
            || (contains_field_pattern(field_name, "access")
                && contains_field_pattern(field_name, "key"))
            || (contains_field_pattern(field_name, "session")
                && contains_field_pattern(field_name, "key"));

        // Check if values look like tokens (long hex strings, not colors)
        // IMPORTANT: Require BOTH field name context AND value pattern to avoid false positives
        // Generic fields like "body", "description", "summary" should not be detected as Token
        if suggests_token || matches_field_name(field_name, "key") {
            let looks_like_token = strs.iter().any(|s| {
                // Tokens are typically 32+ chars without # prefix
                s.len() >= 32
                    && !s.starts_with('#')
                    && s.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            });

            // Require both field name context AND value pattern
            if (suggests_token || matches_field_name(field_name, "key")) && looks_like_token {
                return Some((FieldType::Token, 0.95));
            }
        }

        // Hex color fields - intelligent detection
        // Check if values look like hex colors (# followed by 3,6,8 hex chars)
        let suggests_color_context =
            contains_any_field_pattern(field_name, &["color", "colour", "hex", "theme", "accent"])
                || matches_any_field_name(field_name, &["bg", "fg"]);

        let looks_like_hex_color = strs.iter().all(|s| {
            // Hex colors: # followed by exactly 3, 6, or 8 hex digits
            if let Some(hex) = s.strip_prefix('#') {
                (hex.len() == 3 || hex.len() == 6 || hex.len() == 8)
                    && hex.chars().all(|c| c.is_ascii_hexdigit())
            } else {
                false
            }
        });

        if suggests_color_context && looks_like_hex_color {
            return Some((FieldType::HexString, 0.95));
        }

        // Even without field name context, if ALL values are hex colors, it's likely HexString
        if looks_like_hex_color && !strs.is_empty() {
            return Some((FieldType::HexString, 0.90));
        }

        // Email fields
        if contains_any_field_pattern(field_name, &["email", "mail"])
            && strs.iter().any(|s| EMAIL_REGEX.is_match(s))
        {
            let confidence = calculate_email_confidence(&strs);
            return Some((FieldType::Email, confidence));
        }

        // Name fields
        if matches_any_field_name(
            field_name,
            &["name", "full_name", "fullname", "first_name", "last_name"],
        ) && strs.iter().any(|s| s.contains(' '))
        {
            return Some((FieldType::Name, 0.85));
        }

        // FilePath vs FileName - intelligent separator detection
        let suggests_path =
            contains_any_field_pattern(field_name, &["path", "directory", "folder", "location"]);

        let suggests_filename = contains_field_pattern(field_name, "file")
            && contains_field_pattern(field_name, "name");

        // Check for path separators
        if suggests_path || suggests_filename {
            let has_path_separator = strs.iter().any(|s| s.contains('/') || s.contains('\\'));

            if has_path_separator {
                return Some((FieldType::FilePath, 0.95));
            } else if suggests_filename {
                // No separators = just a filename
                return Some((FieldType::FileName, 0.95));
            }
        }

        // Filename fields (fallback)
        if contains_field_pattern(field_name, "file")
            && contains_field_pattern(field_name, "name")
            && strs.iter().any(|s| s.contains('.'))
        {
            return Some((FieldType::FileName, 0.90));
        }

        // Timestamp/date fields
        if matches_any_field_name(
            field_name,
            &[
                "created",
                "updated",
                "modified",
                "timestamp",
                "created_at",
                "updated_at",
                "date",
                "time",
            ],
        ) {
            if strs.iter().any(|s| TIMESTAMP_REGEX.is_match(s)) {
                return Some((FieldType::Timestamp, 0.95));
            }
            if strs.iter().any(|s| ISO_DATE_REGEX.is_match(s)) {
                return Some((FieldType::IsoDate, 0.95));
            }
        }

        // Version fields
        if matches_any_field_name(field_name, &["version", "ver"])
            && strs.iter().any(|s| SEMVER_REGEX.is_match(s))
        {
            return Some((FieldType::Semver, 0.90));
        }

        // ETag fields (some APIs use numeric version numbers)
        // This handles numeric ETags (simple version numbers)
        if matches_field_name(field_name, "etag")
            && strs.iter().all(|s| s.chars().all(|c| c.is_ascii_digit()))
        {
            return Some((FieldType::ETag, 0.95));
        }

        // Token/API key fields
        if matches_any_field_name(
            field_name,
            &["token", "api_key", "access_token", "refresh_token", "jwt"],
        ) && strs.iter().all(|s| s.len() > 20)
        {
            return Some((FieldType::Token, 0.90));
        }

        // IP address fields
        if matches_any_field_name(field_name, &["ip", "ip_address", "ipv4", "host"])
            && strs.iter().any(|s| IP_REGEX.is_match(s))
        {
            return Some((FieldType::IpAddress, 0.95));
        }

        // Phone fields - smart pattern matching
        let suggests_phone = contains_any_field_pattern(
            field_name,
            &["phone", "tel", "fax", "mobile", "cell", "hotline"],
        ) || (contains_field_pattern(field_name, "contact")
            && contains_field_pattern(field_name, "number"))
            || (contains_field_pattern(field_name, "support")
                && contains_field_pattern(field_name, "number"));

        if suggests_phone
            && strs
                .iter()
                .any(|s| s.len() >= 10 && PHONE_REGEX.is_match(s))
        {
            return Some((FieldType::PhoneNumber, 0.85));
        }

        // Sentence vs Paragraph - intelligent length-based detection
        // ONLY trigger on strong field name signals, not generic text fields
        let suggests_short_text = contains_any_field_pattern(
            field_name,
            &[
                "title", "headline", "subject", "caption", "label", "heading", "tagline", "slogan",
                "motto", "line",
            ],
        ) || (contains_field_pattern(field_name, "summary")
            && !contains_field_pattern(field_name, "long"));

        let suggests_long_text = (contains_field_pattern(field_name, "description")
            && contains_field_pattern(field_name, "long"))
            || (contains_field_pattern(field_name, "body")
                && !contains_field_pattern(field_name, "size"))
            || contains_any_field_pattern(
                field_name,
                &["bio", "about", "details", "story", "narrative", "overview"],
            );

        // Analyze text length to distinguish Sentence from Paragraph
        if suggests_short_text || suggests_long_text {
            // IMPORTANT: Only detect as Sentence/Paragraph if values actually contain words/spaces
            // Random alphanumeric strings, data URIs, and other non-text should NOT be detected as sentences
            let looks_like_natural_text = strs.iter().any(|s| {
                // Natural text MUST contain spaces (word separators)
                // Exclude data URIs, URLs, and other structured formats
                s.contains(' ') && !s.starts_with("data:") && !s.starts_with("http")
            });

            if looks_like_natural_text {
                // Calculate average text length
                let avg_length = if strs.is_empty() {
                    0.0
                } else {
                    strs.iter().map(|s| s.len()).sum::<usize>() as f64 / strs.len() as f64
                };

                // Short text (< 100 chars avg) = Sentence, Long text (>= 150 chars) = Paragraph
                // For fields explicitly suggesting long text (body, description, etc.), use lower threshold (>=80)
                if avg_length >= 150.0 || (suggests_long_text && avg_length >= 80.0) {
                    return Some((FieldType::Paragraph, 0.90));
                } else if suggests_short_text && avg_length > 0.0 {
                    return Some((FieldType::Sentence, 0.90));
                }
            }
            // else: Skip Sentence/Paragraph detection for non-text content (random strings, data URIs, etc.)
            // Let other detection rules handle these
        }

        // CurrencyCode detection - MUST come before CountryCode since both are 3-letter codes
        let suggests_currency = contains_any_field_pattern(
            field_name,
            &[
                "currency",
                "forex",
                "denomination",
                "monetary_unit",
                "exchange",
            ],
        );

        if suggests_currency {
            // Check if values are exactly 3 uppercase letters (typical currency codes)
            let looks_like_currency_code = strs
                .iter()
                .all(|s| s.len() == 3 && s.chars().all(|c| c.is_ascii_uppercase()));

            if looks_like_currency_code {
                return Some((FieldType::CurrencyCode, 0.95));
            }
        }

        // CountryCode vs Categorical - intelligent 2-3 letter code detection
        let suggests_country =
            contains_any_field_pattern(field_name, &["country", "nation", "citizenship", "origin"])
                || (contains_field_pattern(field_name, "iso")
                    && contains_field_pattern(field_name, "country"));

        if suggests_country {
            // Check if values are 2-3 uppercase letters (typical country codes)
            let looks_like_country_code = strs
                .iter()
                .all(|s| s.len() >= 2 && s.len() <= 3 && s.chars().all(|c| c.is_ascii_uppercase()));

            if looks_like_country_code {
                return Some((FieldType::CountryCode, 0.95));
            }
        }

        // Postal code fields - smart pattern matching
        let suggests_postal = contains_any_field_pattern(field_name, &["zip", "postal"])
            || matches_field_name(field_name, "postcode")
            || (contains_field_pattern(field_name, "code")
                && contains_any_field_pattern(field_name, &["mail", "area", "region", "district"]));

        if suggests_postal {
            // Check if values look like postal codes - be more flexible with validation
            // Can be: all numeric (94105), alphanumeric (SW1A 1AA, K1A 0B1), or with dash (94105-1234)
            let looks_like_postal = strs.iter().all(|s| {
                let alphanumeric_count = s.chars().filter(|c| c.is_alphanumeric()).count();
                // Postal codes: 3-10 alphanumeric chars
                (3..=10).contains(&alphanumeric_count)
          // Only alphanumeric, spaces, and hyphens
          && s.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-')
            });

            // Also check for numeric values that could be postal codes
            let numeric_postal = values.iter().all(|v| {
                if let Some(num) = v.as_i64() {
                    // US zip codes are 5 digits (10000-99999)
                    (10_000..=999_999).contains(&num)
                } else {
                    false
                }
            });

            if looks_like_postal || (numeric_postal && !values.is_empty()) {
                return Some((FieldType::PostalCode, 0.90));
            }
        }
    }

    None
}

/// Calculate semantic adjustment based on field name and detected type
/// Returns an adjustment factor (positive = boost, negative = penalty)
/// Range: -0.3 (strong penalty) to 0.3 (strong boost)
pub(super) fn calculate_semantic_boost(field_name: &str, field_type: &FieldType) -> f64 {
    let name_lower = field_name.to_lowercase();

    match field_type {
        FieldType::Uuid => {
            // Positive: field name suggests UUID
            if name_lower.ends_with("_id")
                || name_lower.ends_with("uuid")
                || name_lower.contains("uuid")
            {
                return 0.2;
            }
            // Negative: field name suggests email or URL (common misdetection)
            if name_lower.contains("email") || name_lower.contains("url") {
                return -0.2;
            }
            0.0
        }
        FieldType::Timestamp => {
            if name_lower.contains("date")
                || name_lower.ends_with("_at")
                || name_lower.contains("time")
            {
                return 0.15;
            }
            // Penalty if field name suggests something else
            if name_lower.contains("name") || name_lower.contains("title") {
                return -0.15;
            }
            0.0
        }
        FieldType::Email => {
            if name_lower.contains("email") || name_lower.contains("mail") {
                return 0.25;
            }
            // Penalty if field name suggests URL or username
            if name_lower.contains("url") || name_lower.contains("username") {
                return -0.2;
            }
            0.0
        }
        FieldType::Url => {
            if name_lower.contains("url")
                || name_lower.contains("link")
                || name_lower.contains("href")
            {
                return 0.15;
            }
            // Penalty if field name suggests email or image
            if name_lower.contains("email")
                || name_lower.contains("avatar")
                || name_lower.contains("icon")
            {
                return -0.2;
            }
            0.0
        }
        FieldType::ImageUrl => {
            if name_lower.contains("avatar")
                || name_lower.contains("icon")
                || name_lower.contains("thumbnail")
                || name_lower.contains("image")
                || name_lower.contains("picture")
                || name_lower.contains("photo")
            {
                return 0.20;
            }
            // Penalty if field name suggests generic URL
            if name_lower.contains("url")
                && !name_lower.contains("avatar")
                && !name_lower.contains("icon")
            {
                return -0.15;
            }
            0.0
        }
        FieldType::IpAddress if name_lower.contains("ip") || name_lower.contains("host") => 0.2,
        FieldType::PhoneNumber => {
            if name_lower.contains("phone") || name_lower.contains("tel") {
                return 0.2;
            }
            // Penalty if looks like ID or code
            if name_lower.contains("id") || name_lower.contains("code") {
                return -0.15;
            }
            0.0
        }
        FieldType::Name => {
            if name_lower.contains("name") {
                return 0.15;
            }
            // Penalty if it's actually an ID or code field
            if name_lower.ends_with("_id") || name_lower.contains("code") {
                return -0.2;
            }
            0.0
        }
        FieldType::FileName if name_lower.contains("file") && name_lower.contains("name") => 0.2,
        FieldType::Token
            if name_lower.contains("token")
                || name_lower.contains("key")
                || name_lower.contains("jwt") =>
        {
            0.2
        }
        FieldType::Latitude if name_lower == "lat" || name_lower == "latitude" => 0.3,
        FieldType::Longitude
            if name_lower == "lon" || name_lower == "lng" || name_lower == "longitude" =>
        {
            0.3
        }
        FieldType::CountryCode if name_lower.contains("country") && name_lower.contains("code") => {
            0.25
        }
        FieldType::CurrencyCode if name_lower.contains("currency") => 0.25,
        FieldType::FilePath if name_lower.contains("path") => 0.2,
        _ => 0.0, // No adjustment
    }
}
