//! Type checker functions for pattern-based type detection

use super::DetectionContext;
use super::constants::*;
use super::features::TypeFeatures;
use super::types::{DateFormat, FieldType, TimestampFormat};

/// Type checker entry for data-driven pattern detection
pub(super) struct TypeChecker {
    #[allow(dead_code)]
    pub name: &'static str,
    pub checker_fn: fn(&[&str], &TypeFeatures, &DetectionContext<'_>) -> Option<f64>,
    pub threshold: f64,
    pub field_type: FieldType,
}

/// Get all type checkers in priority order
pub(super) fn get_checkers() -> Vec<TypeChecker> {
    vec![
        TypeChecker {
            name: "DataUri",
            checker_fn: check_data_uri,
            threshold: CONFIDENCE_DATA_URI,
            field_type: FieldType::DataUri { mime_type: None },
        },
        TypeChecker {
            name: "DownloadUrl",
            checker_fn: check_download_url,
            threshold: CONFIDENCE_DOWNLOAD_URL,
            field_type: FieldType::DownloadUrl { sample_url: None },
        },
        TypeChecker {
            name: "Url",
            checker_fn: check_url,
            threshold: CONFIDENCE_URL,
            field_type: FieldType::Url,
        },
        TypeChecker {
            name: "Email",
            checker_fn: check_email,
            threshold: CONFIDENCE_EMAIL,
            field_type: FieldType::Email,
        },
        TypeChecker {
            name: "Timestamp",
            checker_fn: check_timestamp,
            threshold: CONFIDENCE_TIMESTAMP,
            field_type: FieldType::Timestamp {
                format: TimestampFormat::Rfc3339Utc,
            },
        },
        TypeChecker {
            name: "IsoDate",
            checker_fn: check_iso_date,
            threshold: CONFIDENCE_TIMESTAMP,
            field_type: FieldType::IsoDate {
                format: DateFormat::Iso,
            },
        },
        TypeChecker {
            name: "BooleanWords",
            checker_fn: check_boolean_words,
            threshold: CONFIDENCE_BOOLEAN_WORDS,
            field_type: FieldType::Boolean,
        },
        TypeChecker {
            name: "NumericStringId",
            checker_fn: check_numeric_string_id,
            threshold: CONFIDENCE_NUMERIC_STRING_ID,
            field_type: FieldType::NumericStringId,
        },
        TypeChecker {
            name: "UUID",
            checker_fn: check_uuid,
            threshold: CONFIDENCE_UUID,
            field_type: FieldType::Uuid,
        },
        TypeChecker {
            name: "Semver",
            checker_fn: check_semver,
            threshold: CONFIDENCE_SEMVER,
            field_type: FieldType::Semver,
        },
        TypeChecker {
            name: "FileName",
            checker_fn: check_filename,
            threshold: CONFIDENCE_FILENAME,
            field_type: FieldType::FileName,
        },
        TypeChecker {
            name: "Base64",
            checker_fn: check_base64,
            threshold: CONFIDENCE_BASE64,
            field_type: FieldType::Base64,
        },
        TypeChecker {
            name: "HexString",
            checker_fn: check_hex_string,
            threshold: CONFIDENCE_HEX_STRING,
            field_type: FieldType::HexString {
                length: None,
                upper: false,
            },
        },
        TypeChecker {
            name: "ETag",
            checker_fn: check_etag,
            threshold: CONFIDENCE_ETAG,
            field_type: FieldType::ETag,
        },
        TypeChecker {
            name: "Token",
            checker_fn: check_token,
            threshold: CONFIDENCE_TOKEN,
            field_type: FieldType::Token,
        },
        TypeChecker {
            name: "MimeType",
            checker_fn: check_mime_type,
            threshold: CONFIDENCE_MIME_TYPE,
            field_type: FieldType::MimeType,
        },
        TypeChecker {
            name: "IpAddress",
            checker_fn: check_ip_address,
            threshold: CONFIDENCE_IP_ADDRESS,
            field_type: FieldType::IpAddress,
        },
        TypeChecker {
            name: "PhoneNumber",
            checker_fn: check_phone_number,
            threshold: CONFIDENCE_PHONE_NUMBER,
            field_type: FieldType::PhoneNumber,
        },
        TypeChecker {
            name: "Name",
            checker_fn: check_name,
            threshold: CONFIDENCE_NAME,
            field_type: FieldType::Name,
        },
        TypeChecker {
            name: "Paragraph",
            checker_fn: check_paragraph,
            threshold: 0.70,
            field_type: FieldType::Paragraph,
        },
        TypeChecker {
            name: "Sentence",
            checker_fn: check_sentence,
            threshold: 0.70,
            field_type: FieldType::Sentence,
        },
        TypeChecker {
            name: "FilePath",
            checker_fn: check_file_path,
            threshold: 0.70,
            field_type: FieldType::FilePath,
        },
        TypeChecker {
            name: "ApiEndpoint",
            checker_fn: check_api_endpoint,
            threshold: 0.70,
            field_type: FieldType::ApiEndpoint,
        },
        TypeChecker {
            name: "CountryCode",
            checker_fn: check_country_code,
            threshold: 0.80,
            field_type: FieldType::CountryCode,
        },
        TypeChecker {
            name: "CurrencyCode",
            checker_fn: check_currency_code,
            threshold: 0.80,
            field_type: FieldType::CurrencyCode,
        },
        TypeChecker {
            name: "Timezone",
            checker_fn: check_timezone,
            threshold: 0.80,
            field_type: FieldType::Timezone,
        },
        TypeChecker {
            name: "LocaleCode",
            checker_fn: check_locale_code,
            threshold: 0.80,
            field_type: FieldType::LocaleCode,
        },
        TypeChecker {
            name: "PostalCode",
            checker_fn: check_postal_code,
            threshold: 0.75,
            field_type: FieldType::PostalCode,
        },
        // Last, so every class that can name itself has already spoken. What
        // reaches here is a value with no marker of its own, and a handle is the
        // one such shape that still reads as something.
        TypeChecker {
            name: "Handle",
            checker_fn: check_handle,
            threshold: 0.70,
            field_type: FieldType::Username,
        },
    ]
}

// ============================================================================
// Individual Type Checkers with Multi-Sample Validation
// ============================================================================

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_download_url(
    values: &[&str],
    _features: &TypeFeatures,
    ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let matches = values
        .iter()
        .filter(|s| {
            URL_REGEX.is_match(s) // Must be a valid URL
        && (s.contains("download")
            || s.contains("/d/")
            || s.contains("content")
            || s.contains("attachment")
            || ctx.profile.is_download_url(s))
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_DOWNLOAD_URL {
        Some(match_ratio.mul_add(0.8, 0.2))
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_data_uri(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let matches = values
        .iter()
        .filter(|s| {
            // Check if it starts with data: and has base64 encoding
            DATA_URI_REGEX.is_match(s) && s.len() > 50
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_DATA_URI && features.avg_length > 50.0 {
        Some(match_ratio.mul_add(0.95, 0.05))
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_url(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if !features.has_protocol {
        return None;
    }

    let matches = values.iter().filter(|s| URL_REGEX.is_match(s)).count();

    let match_ratio = matches as f64 / values.len() as f64;

    // Additional validation: check for domain pattern
    let has_domain = values
        .iter()
        .filter(|s| s.contains('.') && s.contains("://"))
        .count() as f64
        / values.len() as f64;

    let confidence = has_domain.mul_add(0.3, match_ratio * 0.7);

    if confidence > MIN_MATCH_RATIO_URL {
        Some(confidence)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn calculate_url_confidence(values: &[&str]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let matches = values.iter().filter(|s| URL_REGEX.is_match(s)).count();

    (matches as f64 / values.len() as f64).clamp(0.0, 1.0)
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_email(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if !features.has_email_at {
        return None;
    }

    // Anti-pattern: Emails should not contain URL protocols
    if values.iter().any(|s| s.contains("://")) {
        return Some(0.1);
    }

    let matches = values.iter().filter(|s| EMAIL_REGEX.is_match(s)).count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_EMAIL {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn calculate_email_confidence(values: &[&str]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let matches = values.iter().filter(|s| EMAIL_REGEX.is_match(s)).count();

    (matches as f64 / values.len() as f64).clamp(0.0, 1.0)
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_timestamp(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Every spelling of a moment, not just the ISO one: a database column that
    // reached the wire (`2024-03-17 09:41:22`), an HTTP date, an RFC 2822 date,
    // a fractional epoch. All of them were falling through to `RandomString`.
    let matches = values
        .iter()
        .filter(|s| TIMESTAMP_REGEX.is_match(s) || TimestampFormat::of(s).is_some())
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_TIMESTAMP {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_iso_date(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Every spelling of a date without a time, not just the ISO one. `17/03/2024`
    // and `20240317` are dates, and answering them `RandomString` filled the
    // field with ten random characters.
    let matches = values
        .iter()
        .filter(|s| DateFormat::of(s).is_some())
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_TIMESTAMP {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_numeric_string_id(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Numeric string IDs are:
    // - All digits
    // - Any length >= 1 (includes page numbers, small IDs, etc.)
    // - No decimals
    //
    // This correctly classifies all numeric string IDs.
    // Numeric ETags are handled separately via semantic field name detection.
    // Real HTTP ETags are quoted strings, handled by the ETag checker.
    //
    // A compact date is eight digits and is not an identifier. This runs ahead
    // of the date check, so without the anti-pattern `20240317` becomes a
    // numeric id and the template answers a date field with `fake_numeric_id()`.
    if values.iter().all(|s| DateFormat::of(s).is_some()) {
        return None;
    }
    let matches = values
        .iter()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_NUMERIC_STRING_ID {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_uuid(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if !features.has_uuid_format {
        return None;
    }

    // Anti-pattern: UUIDs should ONLY contain hex chars and hyphens
    // If we see other characters, it's not a UUID
    if values
        .iter()
        .any(|s| s.chars().any(|c| !c.is_ascii_hexdigit() && c != '-'))
    {
        return Some(0.1);
    }

    let matches = values.iter().filter(|s| UUID_REGEX.is_match(s)).count();

    let match_ratio = matches as f64 / values.len() as f64;

    // High threshold for UUIDs due to potential false positives
    if match_ratio > MIN_MATCH_RATIO_UUID {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_semver(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // `17.03.2024` is three dot-separated numbers and so is `1.2.3`. A date has
    // a four-digit year in it and a version does not, which is the only thing
    // separating them -- and without this the detector answered a date field
    // with `fake_semver()`.
    if values.iter().any(|s| DateFormat::of(s).is_some()) {
        return None;
    }

    let matches = values.iter().filter(|s| SEMVER_REGEX.is_match(s)).count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_SEMVER {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_filename(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if !features.has_file_extension {
        return None;
    }

    let common_exts = [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "jpg", "jpeg", "png", "gif",
        "svg", "webp", "mp4", "mp3", "wav", "avi", "mov", "zip", "rar", "tar", "gz", "7z", "html",
        "css", "js", "json", "xml", "yaml", "yml", "md", "rst", "csv", "tsv",
    ];

    let matches = values
        .iter()
        .filter(|s| {
            if let Some(ext) = s.rsplit('.').next() {
                common_exts.contains(&ext.to_lowercase().as_str())
            } else {
                false
            }
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_FILENAME {
        Some(match_ratio * 0.9) // Slightly lower confidence
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_base64(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Base64 characteristics:
    // - Only contains A-Z, a-z, 0-9, +, /, =
    // - Length is multiple of 4 (with padding)
    // - Typically longer strings
    // - May end with = or ==

    // Eight characters, not twenty. A relay-style global id (`VXNlcjox`) and a
    // short cursor are base64 and were being read as unstructured strings for no
    // reason but a length floor picked for encoded payloads.
    if features.avg_length < 8.0 {
        return None;
    }

    let base64_chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";

    let matches = values
        .iter()
        .filter(|s| {
            // Hex is a subset of the base64 alphabet, and a 32-character MD5 is a
            // multiple of four. Without this the lower floor would take every
            // digest away from `HexString` and answer it with base64 noise.
            let is_hex = s.chars().all(|c| c.is_ascii_hexdigit());

            s.len() >= 8
                && !is_hex
                && s.chars().all(|c| base64_chars.contains(c))
                && (s.len() % 4 == 0 || s.ends_with('=') || s.ends_with("=="))
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_BASE64 {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_hex_string(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Hex strings are typically:
    // - Hex colors: 3, 6, or 8 characters (RGB, RRGGBB, RRGGBBAA) - may have # prefix
    // - Hash strings: 32, 40, 64, or 128 characters (MD5, SHA-1, SHA-256, SHA-512)
    // - Only hexadecimal characters
    // - May have high format consistency

    if features.digit_ratio + features.alpha_ratio < 0.95 {
        return None;
    }

    // Support both hash lengths and color lengths
    let hex_lengths = [3, 6, 8, 32, 40, 64, 128];

    let matches = values
        .iter()
        .filter(|s| {
            // Strip optional # prefix for matching
            let stripped = s.trim_start_matches('#');

            // Exclude strings that are purely decimal digits (these should be NumericStringId)
            // Only consider hex if it contains at least one hex-specific character (a-f, A-F)
            let has_hex_chars = stripped.chars().any(|c| matches!(c, 'a'..='f' | 'A'..='F'));

            HEX_REGEX.is_match(stripped)
                && has_hex_chars
                && (hex_lengths.contains(&stripped.len()) || stripped.len() >= 16)
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    // Check if these are likely color values (short hex strings)
    let is_likely_color = values.iter().any(|s| {
        let stripped = s.trim_start_matches('#');
        stripped.len() == 3 || stripped.len() == 6 || stripped.len() == 8
    });

    if match_ratio > MIN_MATCH_RATIO_HEX_STRING {
        // For colors, we can relax the format consistency requirement since colors vary
        // For hashes, we still require high format consistency
        if is_likely_color {
            // Colors can have lower format consistency (they vary in value)
            if features.format_consistency > 0.6 {
                Some(match_ratio)
            } else {
                None
            }
        } else if features.format_consistency > 0.8 {
            // Hashes need high format consistency
            Some(match_ratio)
        } else {
            None
        }
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_etag(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    let matches = values.iter().filter(|s| ETAG_REGEX.is_match(s)).count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_ETAG {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_token(
    values: &[&str],
    features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Tokens are typically:
    // - Long strings (> 20 chars)
    // - Alphanumeric with some special chars (-_.)
    // - High entropy

    if features.avg_length < 20.0 || features.char_entropy < 4.0 {
        return None;
    }

    let matches = values
        .iter()
        .filter(|s| {
            s.len() > 20
                && s.chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_TOKEN {
        Some(match_ratio * 0.85) // Lower confidence due to ambiguity
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_mime_type(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    let common_types = [
        "application/",
        "text/",
        "image/",
        "video/",
        "audio/",
        "multipart/",
        "message/",
    ];

    let matches = values
        .iter()
        .filter(|s| s.contains('/') && common_types.iter().any(|prefix| s.starts_with(prefix)))
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_MIME_TYPE {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_ip_address(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    let matches = values
        .iter()
        .filter(|s| {
            if IP_REGEX.is_match(s) {
                // Validate octets are in valid range
                s.split('.').all(|octet| octet.parse::<u8>().is_ok())
            } else {
                false
            }
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_IP_ADDRESS {
        Some(match_ratio)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_phone_number(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Anti-patterns: Phone numbers should be primarily digits
    // If we have multiple consecutive letters, it's not a phone number
    if values.iter().any(|s| {
        let letter_count = s.chars().filter(|c| c.is_alphabetic()).count();
        letter_count > 2 // More than 2 letters likely not a phone number
    }) {
        return Some(0.1);
    }

    let matches = values
        .iter()
        .filter(|s| {
            // Count actual digits
            let digit_count = s.chars().filter(char::is_ascii_digit).count();
            // Must have 7-15 digits and match the pattern
            (7..=15).contains(&digit_count) && PHONE_REGEX.is_match(s)
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_PHONE {
        Some(match_ratio * 0.85) // Lower confidence due to ambiguity
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_name(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Anti-patterns: Names should NOT contain:
    // - URLs or email addresses
    // - Sentence-ending punctuation (. ! ?)
    // - More than 5 words (likely a sentence, not a name)
    // - Very long text (> 50 chars is likely not a person name)
    if values.iter().any(|s| {
        s.contains("://")
            || s.contains("www.")
            || EMAIL_REGEX.is_match(s)
            || s.ends_with('.')
            || s.ends_with('!')
            || s.ends_with('?')
            || s.split_whitespace().count() > 5
            || s.len() > 50
    }) {
        return Some(0.1); // Very low confidence - disqualify
    }

    // Person names typically:
    // - Contain spaces (2-4 words usually)
    // - Start with capital letter
    // - Are alphabetic with occasional special chars (', -, .)
    // - 2-50 characters

    let matches = values
        .iter()
        .filter(|s| {
            let characters = s.chars().count();
            let word_count = s.split_whitespace().count();
            // Japanese and Chinese names are written without a space, and Hebrew
            // and Arabic have no letter case at all. Requiring either of those
            // reads every name outside the Latin conventions as a random string.
            let unspaced = is_unspaced_script(s);
            let spelled_as_words = s.contains(' ') && (2..=5).contains(&word_count);
            let leading = s.chars().next();
            let cased_script = leading.is_some_and(|c| c.is_lowercase() || c.is_uppercase());

            (spelled_as_words || (unspaced && (2..=8).contains(&characters)))
                && (!cased_script || leading.is_some_and(char::is_uppercase))
                && s.chars().filter(|c| c.is_alphabetic()).count() as f64 / characters as f64 > 0.7
                && (2..=50).contains(&characters)
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_NAME {
        Some(match_ratio * 0.8) // Lower confidence
    } else {
        None
    }
}

/// Whether a field holds handles -- the lower-case word-and-separator names a
/// service lets people choose.
///
/// Decided on the values alone, for the fields whose names give nothing away:
/// `owner`, `val`, `slug`. The shape is words joined by `.`, `_` or `-`, each
/// spelled out in full and optionally numbered -- `victor.whisky`, `annexe1`,
/// `kunde_bericht`. An opaque identifier fails it on the digits scattered
/// through the middle of it, which is exactly what separates the two.
#[allow(clippy::cast_precision_loss)]
pub(super) fn check_handle(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    /// Long enough to be a name someone typed, short enough not to be a key.
    const HANDLE_LENGTH: std::ops::RangeInclusive<usize> = 3..=32;
    /// A one-letter chunk is a prefix (`f_1234`), not a word.
    const SHORTEST_WORD: usize = 2;

    let matches = values
        .iter()
        .filter(|value| {
            if !HANDLE_LENGTH.contains(&value.len())
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
            {
                return false;
            }

            // A bare dictionary word is an enum member -- `premium`, `folder`,
            // `active` -- and answering one with an invented handle is worse
            // than repeating what was recorded. A handle carries a separator or
            // a number, which is what being chosen from a namespace looks like.
            if !value
                .bytes()
                .any(|b| b.is_ascii_digit() || b"._-".contains(&b))
            {
                return false;
            }

            let mut words = 0;
            for chunk in value.split(['.', '_', '-']) {
                let word = chunk.trim_end_matches(|c: char| c.is_ascii_digit());
                if word.len() < SHORTEST_WORD || !word.bytes().all(|b| b.is_ascii_lowercase()) {
                    return false;
                }
                words += 1;
            }
            words > 0
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;
    (match_ratio > 0.80).then_some(match_ratio * 0.85)
}

/// Every character a sentence can end with, across the scripts an API answers in.
const SENTENCE_TERMINATORS: [char; 8] = [
    '.', '!', '?', '\u{3002}', '\u{ff01}', '\u{ff1f}', '\u{0964}', '\u{06d4}',
];

/// Whether text is written in a script that does not separate words with spaces.
///
/// Japanese, Chinese and Thai. Counting words in any of them counts one, and
/// then every sentence in the language fails a word-count test.
fn is_unspaced_script(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character as u32,
            0x3040..=0x30FF   // kana
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified
            | 0xF900..=0xFAFF // CJK compatibility
            | 0x0E00..=0x0E7F // Thai
        )
    })
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_sentence(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Sentences are:
    // - Single sentence (may or may not end with punctuation)
    // - 5-20 words (not too short, not paragraph-length)
    // - Contains spaces
    // - 20-200 characters
    // - NOT just a name (handled by Name checker with higher priority)
    //
    // Relaxed detection: Punctuation and capitalization are preferred but not required
    // since fake data doesn't always follow proper grammar

    let matches = values
        .iter()
        .filter(|s| {
            // Measured in characters throughout. Byte length is the same number
            // only for ASCII, and every prose field in a non-Latin script was
            // being judged against a limit three times shorter than it looked.
            let characters = s.chars().count();
            let word_count = s.split_whitespace().count();
            let unspaced = is_unspaced_script(s);

            let ends_with_punctuation = s.ends_with(SENTENCE_TERMINATORS);
            // Japanese, Chinese and Thai do not put spaces between words, so
            // counting words there counts one and rejects every sentence in the
            // language. Their length is what carries the structure instead.
            let has_sentence_structure = if unspaced {
                (8..=120).contains(&characters)
            } else {
                // Two words are enough when the value is punctuated as a
                // sentence. `Short note.` is prose; `Ada Lovelace` is not, and
                // the full stop is what separates them.
                (5..=20).contains(&word_count)
                    || (s.ends_with(SENTENCE_TERMINATORS) && (2..=20).contains(&word_count))
            };
            // A short value is prose when something says so -- a script with no
            // spaces, or a full stop. Without either, twenty characters is the
            // floor that keeps a two-word label out.
            let reasonable_length = if unspaced || s.ends_with(SENTENCE_TERMINATORS) {
                (8..=200).contains(&characters)
            } else {
                (20..=200).contains(&characters)
            };

            // Count sentences - should be just one
            let sentence_count = s.matches(SENTENCE_TERMINATORS).count();
            let is_single_sentence = sentence_count <= 1;

            // Must have basic sentence structure (removed capitalization requirement for fake data)
            if !has_sentence_structure || !reasonable_length || !is_single_sentence {
                return false;
            }

            // Check for anti-patterns that indicate it's NOT a sentence
            let has_colons_or_semicolons = s.contains(':') || s.contains(';');
            let looks_like_code_or_spec =
                has_colons_or_semicolons && s.split([':', ';']).count() > 2;

            if looks_like_code_or_spec {
                return false;
            }

            // Prefer punctuated sentences (high confidence)
            if ends_with_punctuation {
                return true;
            }

            // Thai writes no full stop at all, and Japanese and Chinese often
            // drop it in a short field. Demanding one rejects the language.
            if unspaced {
                return true;
            }

            // Accept without punctuation if strong sentence indicators present
            // - Has articles/prepositions (common in English sentences)
            let lowercase = s.to_lowercase();
            let has_articles = lowercase.contains(" the ")
                || lowercase.contains(" a ")
                || lowercase.contains(" an ");
            let has_prepositions = lowercase.contains(" to ")
                || lowercase.contains(" of ")
                || lowercase.contains(" in ")
                || lowercase.contains(" for ")
                || lowercase.contains(" with ");

            // For fake/test data, be more lenient - accept if it looks sentence-like
            // (has multiple words in reasonable length range without being too long)
            if has_articles || has_prepositions {
                return true;
            }

            // Final fallback: If it's in the sweet spot for sentences (6-15 words, 30-150 chars)
            // and has spaces (multiple words), accept it
            (6..=15).contains(&word_count) && s.len() >= 30 && s.len() <= 150
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > 0.7 {
        Some(match_ratio * 0.85) // Good confidence
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_paragraph(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Paragraphs are:
    // - Multiple sentences (2+ sentence-ending punctuation marks) - PRIMARY indicator
    // - OR long text (150+ chars) with many words (20+) - SECONDARY indicator
    // - OR very many words (25+) even without punctuation - TERTIARY indicator
    // - 100-1000 characters (minimum length for paragraph)
    // - Contains multiple spaces
    //
    // Relaxed capitalization requirement - fake data doesn't always capitalize properly

    let matches = values
        .iter()
        .filter(|s| {
            // Characters rather than bytes: the same prose is three times longer
            // in bytes in Japanese than in English, and a byte floor let one
            // through while rejecting the other.
            let characters = s.chars().count();
            let word_count = s.split_whitespace().count();
            let unspaced = is_unspaced_script(s);
            let sentence_count = s.matches(SENTENCE_TERMINATORS).count();

            if characters > 1000 {
                return false;
            }

            // Primary indicator, and it leads: several sentences is a paragraph
            // whatever the length. Requiring a hundred characters first threw
            // away every short two-sentence note before this could say so.
            if sentence_count >= 2 && (word_count >= 6 || (unspaced && characters >= 12)) {
                return true;
            }

            if characters < 100 {
                return false;
            }

            // A script without spaces counts one word however long it is.
            if unspaced {
                return characters >= 60;
            }

            if word_count < 15 {
                return false;
            }

            // Secondary indicator: Long text (150+ chars) with decent word count (20+)
            // This catches paragraphs that are run-on sentences or missing punctuation
            if characters >= 150 && word_count >= 20 {
                return true;
            }

            // Tertiary indicator: Very many words (25+) even if shorter
            // This catches dense multi-sentence paragraphs
            if word_count >= 25 {
                return true;
            }

            false
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > 0.7 {
        Some(match_ratio * 0.9) // Higher confidence
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_api_endpoint(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // API endpoints are typically:
    // - Start with /
    // - Contain path segments
    // - May have {id} or :id placeholders
    // Anti-pattern: Should not have common file extensions

    let matches = values
        .iter()
        .filter(|s| {
            let looks_like_api = s.starts_with('/') &&
              !s.contains("://") && // Not a full URL
              s.matches('/').count() >= 2;

            // Anti-pattern: Check for file extensions
            let has_file_extension = s.contains('.')
                && s.rsplit('.').next().is_some_and(|ext| {
                    [
                        "log", "txt", "yml", "yaml", "json", "xml", "csv", "conf", "cfg", "ini",
                    ]
                    .contains(&ext)
                });

            looks_like_api && !has_file_extension
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_API_ENDPOINT {
        Some(match_ratio)
    } else {
        None
    }
}

/// Words that spell a flag.
const BOOLEAN_WORDS: [&str; 10] = [
    "true", "false", "yes", "no", "y", "n", "t", "f", "on", "off",
];

/// Words that spell a flag and nothing else.
///
/// `no` is Norway and `n` is anybody's guess, so a lone one of those is not
/// evidence of a flag. `true`, `false`, `yes`, `on` and `off` are not country
/// codes, currency codes or locales, so one of them on its own is.
const UNAMBIGUOUS_BOOLEAN_WORDS: [&str; 5] = ["true", "false", "yes", "on", "off"];

/// A boolean written as text.
///
/// JSON has a boolean and half the APIs in the world do not use it: a flag comes
/// back as `"true"`, `"yes"`, `"Y"` or `"on"`. Every one of those was reaching
/// the template as an unstructured string, which then filled the field with ten
/// random characters where a client expected a flag.
#[allow(clippy::cast_precision_loss)]
pub(super) fn check_boolean_words(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    if values.is_empty() {
        return None;
    }

    let lowered: Vec<String> = values.iter().map(|s| s.to_lowercase()).collect();
    if !lowered
        .iter()
        .all(|value| BOOLEAN_WORDS.contains(&value.as_str()))
    {
        return None;
    }

    // Either something in there can only be a flag, or the field takes both
    // polarities and so is one whatever the individual words could also mean.
    let unambiguous = lowered
        .iter()
        .any(|value| UNAMBIGUOUS_BOOLEAN_WORDS.contains(&value.as_str()));
    let truthy = lowered
        .iter()
        .any(|value| matches!(value.as_str(), "true" | "yes" | "y" | "t" | "on"));
    let falsy = lowered
        .iter()
        .any(|value| matches!(value.as_str(), "false" | "no" | "n" | "f" | "off"));

    if unambiguous || (truthy && falsy) {
        Some(1.0)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_country_code(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // ISO 3166-1 alpha-2 country codes are two letters. The standard writes them
    // upper case and services return them either way -- `country: "us"` is not a
    // different kind of field from `country: "US"`.
    let matches = values
        .iter()
        .filter(|s| s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic()))
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_COUNTRY_CODE {
        Some(match_ratio * 0.9) // Slightly lower confidence due to ambiguity (US state codes, etc.)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_currency_code(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // ISO 4217 currency codes are three letters, written upper case by the
    // standard and lower case by plenty of services.
    let matches = values
        .iter()
        .filter(|s| s.len() == 3 && s.chars().all(|c| c.is_ascii_alphabetic()))
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_CURRENCY_CODE {
        Some(match_ratio * 0.85) // Lower confidence due to ambiguity
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_postal_code(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Postal/ZIP codes have various formats:
    // - US: 5 digits or 5+4 (12345 or 12345-6789)
    // - UK: Alphanumeric (SW1A 1AA, EC1A 1BB)
    // - CA: Alphanumeric (K1A 0B1, M5H 2N2)
    // - General: Mix of letters, digits, spaces, hyphens

    let matches = values
        .iter()
        .filter(|s| {
            let cleaned = s.trim();
            let len = cleaned.len();

            // Must be reasonable postal code length (3-10 chars typically)
            if !(3..=10).contains(&len) {
                return false;
            }

            // Count different character types
            let has_digits = cleaned.chars().any(|c| c.is_ascii_digit());
            let has_letters = cleaned.chars().any(|c| c.is_ascii_alphabetic());
            let has_space_or_dash = cleaned.contains(' ') || cleaned.contains('-');

            // US ZIP: 5 digits or 5+4
            let is_us_zip = cleaned.len() == 5 && cleaned.chars().all(|c| c.is_ascii_digit())
                || (cleaned.len() == 10
                    && cleaned.chars().take(5).all(|c| c.is_ascii_digit())
                    && cleaned.chars().nth(5) == Some('-')
                    && cleaned.chars().skip(6).all(|c| c.is_ascii_digit()));

            // UK/CA format: Alphanumeric with space (e.g., "SW1A 1AA", "K1A 0B1")
            let is_alphanumeric_postal = has_digits
                && has_letters
                && has_space_or_dash
                && cleaned
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-');

            // Generic: Just digits (common in many countries)
            let is_numeric_only = len >= 4 && cleaned.chars().all(|c| c.is_ascii_digit());

            is_us_zip || is_alphanumeric_postal || is_numeric_only
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > 0.75 {
        Some(match_ratio * 0.8)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_locale_code(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // Locale codes follow BCP 47 format:
    // - language-COUNTRY (en-US, fr-FR, ja-JP)
    // - language-script-COUNTRY (zh-Hans-CN)
    // - Just language (en, fr, ja)

    let matches = values
        .iter()
        .filter(|s| {
            // POSIX writes `en_US` and BCP 47 writes `en-US`; both name the same
            // locale, and both turn up in the same response. POSIX also appends
            // the character set and a modifier -- `de_DE.UTF-8`, `sr_RS@latin`
            // -- neither of which changes which locale is named.
            let named = s
                .split(['.', '@'])
                .next()
                .filter(|named| !named.is_empty())
                .unwrap_or(s);
            let parts: Vec<&str> = named.split(['-', '_']).collect();

            // Must have 1-3 parts
            if parts.is_empty() || parts.len() > 3 {
                return false;
            }

            // First part: 2-3 lowercase letters (language code). A bare three
            // letter word is not enough on its own -- `pdf`, `zip` and `doc` are
            // all shaped like a language code, and a file extension answered
            // with `fr-CA` is a broken response. With a region attached there is
            // no such ambiguity.
            let Some(lang) = parts.first() else {
                return false;
            };
            let plausible_length = if parts.len() == 1 {
                lang.len() == 2
            } else {
                (2..=3).contains(&lang.len())
            };
            if !plausible_length || !lang.chars().all(|c| c.is_ascii_lowercase()) {
                return false;
            }

            // If has second part, check format
            if let Some(second) = parts.get(1) {
                // Could be country (2 uppercase) or script (4 titlecase)
                let is_country =
                    second.len() == 2 && second.chars().all(|c| c.is_ascii_uppercase());
                let is_script = second.len() == 4
                    && second.chars().next().is_some_and(char::is_uppercase)
                    && second.chars().skip(1).all(char::is_lowercase);

                if !is_country && !is_script {
                    return false;
                }
            }

            // If has third part, must be country code
            if let Some(country) = parts.get(2)
                && (country.len() != 2 || !country.chars().all(|c| c.is_ascii_uppercase()))
            {
                return false;
            }

            true
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > 0.80 {
        Some(match_ratio * 0.9)
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_timezone(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // IANA timezone identifiers:
    // - Format: Area/Location or Area/Location/SubLocation
    // - Examples: America/New_York, Europe/London, Asia/Tokyo
    // - Must have at least one slash
    // - Parts are TitleCase (first letter uppercase)

    let matches = values
        .iter()
        .filter(|s| {
            // Must contain at least one slash
            if !s.contains('/') {
                return false;
            }

            let parts: Vec<&str> = s.split('/').collect();

            // Must have 2-3 parts
            if parts.len() < 2 || parts.len() > 3 {
                return false;
            }

            // Each part should start with uppercase and contain mostly letters/underscores
            parts.iter().all(|part| {
                !part.is_empty()
                    && part.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > 0.80 {
        Some(match_ratio * 0.95) // High confidence - very specific format
    } else {
        None
    }
}

#[allow(clippy::cast_precision_loss)]
pub(super) fn check_file_path(
    values: &[&str],
    _features: &TypeFeatures,
    _ctx: &DetectionContext<'_>,
) -> Option<f64> {
    // File paths have:
    // - Forward slashes (Unix) or backslashes (Windows)
    // - No protocol (not a URL)
    // - Start with / or drive letter for absolute paths
    // - May or may not have file extensions

    let matches = values
        .iter()
        .filter(|s| {
            let has_slashes = s.contains('/') || s.contains('\\');
            let not_url = !s.contains("://");
            let reasonable_length = s.len() > 3;

            // Check for typical path patterns
            let unix_absolute = s.starts_with('/');
            let unix_tilde = s.starts_with('~'); // ~/path or ~user/path (valid shell paths)
            let windows_path =
                s.len() >= 3 && s.chars().nth(1) == Some(':') && s.chars().nth(2) == Some('\\');

            // Accept paths that match any valid path pattern
            has_slashes
                && not_url
                && reasonable_length
                && (unix_absolute || unix_tilde || windows_path)
        })
        .count();

    let match_ratio = matches as f64 / values.len() as f64;

    if match_ratio > MIN_MATCH_RATIO_FILE_PATH {
        Some(match_ratio * 0.85)
    } else {
        None
    }
}
