//! Core type definitions for type detection system

use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value as JsonValue;
use url::Url;

use super::constants::{CURSOR_KEYS, LIMIT_KEYS, PAGE_KEYS};

/// How a date without a time is written.
///
/// Carried on the type because a template that answers a `17/03/2024` field with
/// `2024-03-17` has changed the value's shape, and anything parsing it breaks.
/// Detecting the class is only half the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateFormat {
    /// `2024-03-17`
    #[default]
    Iso,
    /// `17/03/2024`
    Slash,
    /// `17.03.2024`
    Dotted,
    /// `20240317`
    Compact,
}

impl DateFormat {
    /// The name a template passes to the generator.
    pub fn name(self) -> &'static str {
        match self {
            Self::Iso => "iso",
            Self::Slash => "slash",
            Self::Dotted => "dotted",
            Self::Compact => "compact",
        }
    }

    /// Which format a value is written in, if it is a date at all.
    pub fn of(value: &str) -> Option<Self> {
        let digits = |part: &str, width: usize| {
            part.len() == width && part.chars().all(|c| c.is_ascii_digit())
        };

        if value.len() == 8 && value.chars().all(|c| c.is_ascii_digit()) {
            let year: u32 = value.get(..4)?.parse().ok()?;
            let month: u32 = value.get(4..6)?.parse().ok()?;
            let day: u32 = value.get(6..)?.parse().ok()?;
            return ((1900..=2999).contains(&year)
                && (1..=12).contains(&month)
                && (1..=31).contains(&day))
            .then_some(Self::Compact);
        }

        for (separator, format) in [('-', Self::Iso), ('/', Self::Slash), ('.', Self::Dotted)] {
            let parts: Vec<&str> = value.split(separator).collect();
            let [first, middle, last] = parts.as_slice() else {
                continue;
            };
            // ISO writes the year first; the other two write it last. That is
            // also what separates `17.03.2024` from the version `1.2.3`.
            let (year, month, day) = if format == Self::Iso {
                (*first, *middle, *last)
            } else {
                (*last, *middle, *first)
            };
            if !digits(year, 4) || !digits(month, 2) || !digits(day, 2) {
                continue;
            }
            let (month, day): (u32, u32) = (month.parse().ok()?, day.parse().ok()?);
            if (1..=12).contains(&month) && (1..=31).contains(&day) {
                return Some(format);
            }
        }
        None
    }
}

/// How a flag is written.
///
/// Same reason as [`DateFormat`]: JSON has a boolean and half the APIs in the
/// world do not use it. A field of `"1"`s answered `"true"` is the right class
/// and a value the client cannot parse, which is the defect the class was
/// supposed to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BooleanSpelling {
    /// A JSON `true`, or the word in lower case.
    #[default]
    TrueFalse,
    /// `True` / `False`
    TitleTrueFalse,
    /// `TRUE` / `FALSE`
    UpperTrueFalse,
    /// `yes` / `no`
    YesNo,
    /// `Y` / `N`
    YN,
    /// `on` / `off`
    OnOff,
    /// `1` / `0`
    Digit,
}

impl BooleanSpelling {
    /// The name a template passes to the generator.
    pub fn name(self) -> &'static str {
        match self {
            Self::TrueFalse => "true_false",
            Self::TitleTrueFalse => "title",
            Self::UpperTrueFalse => "upper",
            Self::YesNo => "yes_no",
            Self::YN => "y_n",
            Self::OnOff => "on_off",
            Self::Digit => "digit",
        }
    }

    /// Which spelling a value is written in, if it spells a flag at all.
    pub fn of(value: &str) -> Option<Self> {
        match value.trim() {
            "true" | "false" | "t" | "f" => Some(Self::TrueFalse),
            "True" | "False" => Some(Self::TitleTrueFalse),
            "TRUE" | "FALSE" => Some(Self::UpperTrueFalse),
            "1" | "0" => Some(Self::Digit),
            other => match other.to_lowercase().as_str() {
                "yes" | "no" => Some(Self::YesNo),
                "y" | "n" => Some(Self::YN),
                "on" | "off" => Some(Self::OnOff),
                _ => None,
            },
        }
    }

    /// The pair this spelling writes, falsy first.
    pub fn pair(self) -> (&'static str, &'static str) {
        match self {
            Self::TrueFalse => ("false", "true"),
            Self::TitleTrueFalse => ("False", "True"),
            Self::UpperTrueFalse => ("FALSE", "TRUE"),
            Self::YesNo => ("no", "yes"),
            Self::YN => ("N", "Y"),
            Self::OnOff => ("off", "on"),
            Self::Digit => ("0", "1"),
        }
    }
}

/// How a moment in time is written.
///
/// Same reason as [`DateFormat`]: a field holding `Sun, 17 Mar 2024 09:41:22 GMT`
/// is a timestamp field, and answering it with ISO 8601 breaks every client that
/// parses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimestampFormat {
    /// `2024-03-17T09:41:22Z`
    #[default]
    Rfc3339Utc,
    /// `2024-03-17T09:41:22+02:00`
    Rfc3339Offset,
    /// `2024-03-17T09:41:22.481Z`
    Rfc3339Millis,
    /// `2024-03-17T09:41:22.481920374Z`
    Rfc3339Nanos,
    /// `2024-03-17 09:41:22` -- a database column that reached the wire
    SqlDateTime,
    /// `Sun, 17 Mar 2024 09:41:22 +0000`
    Rfc2822,
    /// `Sun, 17 Mar 2024 09:41:22 GMT`
    HttpDate,
    /// `1710668482.000100`
    EpochFractional,
}

impl TimestampFormat {
    pub fn name(self) -> &'static str {
        match self {
            Self::Rfc3339Utc => "rfc3339",
            Self::Rfc3339Offset => "rfc3339_offset",
            Self::Rfc3339Millis => "rfc3339_millis",
            Self::Rfc3339Nanos => "rfc3339_nanos",
            Self::SqlDateTime => "sql",
            Self::Rfc2822 => "rfc2822",
            Self::HttpDate => "http",
            Self::EpochFractional => "epoch_fractional",
        }
    }

    /// Which format a value is written in, if it is a timestamp at all.
    pub fn of(value: &str) -> Option<Self> {
        // Every spelling below carries a clock time and a four-digit year. Text
        // that carries neither is not a timestamp however it is punctuated.
        let has_time = value.contains(':');
        let has_year = value
            .as_bytes()
            .windows(4)
            .any(|window| window.iter().all(u8::is_ascii_digit));

        // `1710668482.000100`
        if let Some((seconds, fraction)) = value.split_once('.')
            && seconds.len() >= 9
            && seconds.chars().all(|c| c.is_ascii_digit())
            && !fraction.is_empty()
            && fraction.chars().all(|c| c.is_ascii_digit())
        {
            return Some(Self::EpochFractional);
        }

        if !(has_time && has_year) {
            return None;
        }

        if value.ends_with(" GMT") && value.contains(", ") {
            return Some(Self::HttpDate);
        }
        if value.contains(", ") && (value.contains(" +") || value.contains(" -")) {
            return Some(Self::Rfc2822);
        }

        let separated = value.contains('T') || value.contains(' ');
        if !separated || !value.contains(':') {
            return None;
        }
        if value.contains(' ') && !value.contains('T') {
            return Some(Self::SqlDateTime);
        }

        let fraction = value
            .split_once('.')
            .map(|(_, rest)| rest.chars().take_while(char::is_ascii_digit).count());
        Some(match fraction {
            Some(digits) if digits >= 7 => Self::Rfc3339Nanos,
            Some(_) => Self::Rfc3339Millis,
            None if value.ends_with('Z') => Self::Rfc3339Utc,
            None => Self::Rfc3339Offset,
        })
    }
}

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
    /// A moment in time, in the format it was written in
    Timestamp { format: TimestampFormat },
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
    /// Boolean value that varies, in the spelling the field used
    Boolean { spelling: BooleanSpelling },
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
    /// A date without a time, in the format it was written in
    IsoDate { format: DateFormat },
    /// Unix timestamp (numeric, seconds)
    UnixTimestamp,
    /// Unix timestamp in milliseconds
    MillisecondTimestamp,
    /// Unix timestamp in microseconds
    MicrosecondTimestamp,
    /// Semantic version string
    Semver,
    /// Hexadecimal string, at the width and case it was seen at
    HexString { length: Option<usize>, upper: bool },
    /// Base64-encoded data
    Base64,
    /// Latitude coordinate (-90 to 90)
    Latitude,
    /// Longitude coordinate (-180 to 180)
    Longitude,
    /// Categorical/Enum string (low cardinality)
    Categorical { values: Vec<String> },
    /// A value shaped like `inner` that the recording wrote as text.
    ///
    /// APIs do this constantly: `"size": "1024"`, `"sequence_id": "0"`,
    /// `"enabled": "true"`. Answering such a field with a JSON number is a
    /// different defect from getting its class wrong -- the class is right and
    /// the type the client parses has changed.
    Stringified(Box<FieldType>),
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

impl FieldType {
    /// Whether a template writes this type without surrounding quotes.
    ///
    /// The list is the emitter's, and the two have to agree: this is what says
    /// a class needs [`FieldType::Stringified`] wrapping to keep the JSON kind
    /// the recording used.
    pub fn writes_bare(&self) -> bool {
        matches!(
            self,
            Self::SequentialNumber { .. }
                | Self::RandomNumber { .. }
                | Self::RandomFloat { .. }
                | Self::UnixTimestamp
                | Self::MillisecondTimestamp
                | Self::MicrosecondTimestamp
                | Self::FileSize
                | Self::Latitude
                | Self::Longitude
                | Self::Boolean { .. }
        )
    }
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
    /// The type of each position of a representative recorded array, used when
    /// the elements are not all the same shape.
    ///
    /// A listing mixes kinds -- a file carries `extension`, a folder carries
    /// `fileCount` -- and neither one element type nor an empty array can stand
    /// for that. Keeping the shapes in the order they were recorded lets the
    /// template answer with the same sequence of kinds.
    pub element_shapes: Vec<FieldType>,
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
    /// The base URL without query params (e.g., "http://localhost:3000/api/v1/documents-search/")
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
