//! Recording filter options and utilities

use regex::Regex;
use std::time::Duration;

/// Recording filter options
#[derive(Debug, Clone)]
pub struct RecordingFilterOptions {
    /// Only record URLs matching this regex pattern
    pub filter_url: Option<Regex>,
    /// Only record requests with error status codes (4xx, 5xx)
    pub capture_errors_only: bool,
    /// Only record successful responses (2xx) - default behavior for backward compatibility
    pub capture_success_only: bool,
    /// Automatically export recording when an error occurs
    pub auto_export_on_error: bool,
    /// Number of requests to include before/after an error
    pub error_context_requests: usize,
    /// Exclude URLs matching these regex patterns (e.g., for static assets, analytics)
    /// Examples: r"\.js$", r"\.css$", r"/static/", r"google-analytics\.com"
    pub exclude_patterns: Vec<Regex>,
    /// Only record requests that take longer than this duration
    pub min_duration: Option<Duration>,
    /// Strip delay information from recorded mocks
    pub strip_delay: bool,
}

impl Default for RecordingFilterOptions {
    fn default() -> Self {
        Self {
            filter_url: None,
            capture_errors_only: false,
            capture_success_only: true, // Default: only record successful responses
            auto_export_on_error: false,
            error_context_requests: 0,
            exclude_patterns: Self::web_static_patterns(),
            min_duration: None,
            strip_delay: false,
        }
    }
}

impl RecordingFilterOptions {
    /// Create exclude patterns for common web static assets (JS, CSS, fonts, source maps)
    ///
    /// This excludes typical web UI assets but NOT API file content like PDFs, images served
    /// through /api/ or /files/ endpoints.
    ///
    /// # Examples
    /// ```
    /// use ferrimock::recorder::filters::RecordingFilterOptions;
    ///
    /// let mut options = RecordingFilterOptions::default();
    /// options.exclude_patterns = RecordingFilterOptions::web_static_patterns();
    /// ```
    #[allow(clippy::expect_used)] // Static regex literals -- panic on invalid pattern is correct
    pub fn web_static_patterns() -> Vec<Regex> {
        vec![
            // JavaScript files
            Regex::new(r"\.js(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.mjs(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.jsx(\?.*)?$").expect("valid regex"),
            // TypeScript (if served)
            Regex::new(r"\.ts(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.tsx(\?.*)?$").expect("valid regex"),
            // CSS files
            Regex::new(r"\.css(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.scss(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.sass(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.less(\?.*)?$").expect("valid regex"),
            // Fonts
            Regex::new(r"\.woff2?(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.ttf(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.eot(\?.*)?$").expect("valid regex"),
            Regex::new(r"\.otf(\?.*)?$").expect("valid regex"),
            // Source maps
            Regex::new(r"\.map(\?.*)?$").expect("valid regex"),
        ]
    }

    /// Combine multiple pattern sets
    ///
    /// # Examples
    /// ```
    /// use ferrimock::recorder::filters::RecordingFilterOptions;
    /// use regex::Regex;
    ///
    /// let mut options = RecordingFilterOptions::default();
    ///
    /// // Combine web static patterns with custom patterns
    /// let custom_patterns = vec![
    ///   Regex::new(r"google-analytics\.com").unwrap(),
    ///   Regex::new(r"/internal/").unwrap(),
    /// ];
    ///
    /// options.exclude_patterns = RecordingFilterOptions::combine_patterns(vec![
    ///   RecordingFilterOptions::web_static_patterns(),
    ///   custom_patterns,
    /// ]);
    /// ```
    pub fn combine_patterns(pattern_sets: Vec<Vec<Regex>>) -> Vec<Regex> {
        pattern_sets.into_iter().flatten().collect()
    }
}
