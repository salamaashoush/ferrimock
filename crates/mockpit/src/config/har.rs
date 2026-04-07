//! HAR (HTTP Archive) file loading and conversion to mock configurations
//!
//! Produces clean, replay-ready mock collections from HAR files.
//! By default, normalizes absolute URLs to relative paths, filters non-Box
//! domains and static assets, strips sensitive and infrastructure headers,
//! and optionally extracts large response bodies to separate files.
//!
//! Use the consolidator for further smart pattern detection and optimization.

use anyhow::{Context, Result};
use har::{Har, Spec, v1_2};
use rustc_hash::FxHashMap;
use std::path::{Path, PathBuf};
use url::Url;

use super::{MatchConfig, MockConfig, ResponseConfig};

/// Default body size threshold for extraction (100 KB)
const DEFAULT_BODY_SIZE_THRESHOLD: usize = 100 * 1024;

/// Check if a hostname belongs to a Box domain
pub fn is_box_domain(host: &str, extra_domains: &[String]) -> bool {
    let lower = host.to_lowercase();

    // Standard Box domains
    if lower.ends_with(".box.com")
        || lower == "box.com"
        || lower.ends_with(".box.net")
        || lower == "box.net"
        || lower.ends_with(".boxcloud.com")
        || lower == "boxcloud.com"
        || lower.ends_with(".boxcdn.net")
        || lower == "boxcdn.net"
    {
        return true;
    }

    // User-provided extra domains
    for domain in extra_domains {
        let d = domain.to_lowercase();
        if lower == d || lower.ends_with(&format!(".{d}")) {
            return true;
        }
    }

    false
}

/// Check if a URL points to a static asset based on file extension
fn is_static_asset(raw_url: &str) -> bool {
    // Strip query string and fragment
    let path = raw_url.split('?').next().unwrap_or(raw_url);
    let path = path.split('#').next().unwrap_or(path);

    // Extract extension from the last path segment
    let last_segment = path.rsplit('/').next().unwrap_or("");
    let ext = match last_segment.rsplit('.').next() {
        Some(e) if e != last_segment => e.to_lowercase(),
        _ => return false,
    };

    matches!(
        ext.as_str(),
        // Scripts & styles
        "js" | "mjs" | "cjs" | "css" | "map" |
    // Images
    "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "avif" | "bmp" |
    // Fonts
    "woff" | "woff2" | "ttf" | "otf" | "eot" |
    // Media
    "mp3" | "mp4" | "webm" | "ogg" | "wav" | "avi" |
    // Documents
    "pdf" |
    // Archives
    "zip" | "gz" | "tar" | "br" |
    // Manifests & metadata
    "json" | "xml" | "manifest" | "webmanifest"
    )
}

/// Check if a query parameter name is sensitive and should be stripped
fn is_sensitive_query_param(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "access_token" | "token" | "api_key" | "apikey" | "secret" | "password" | "session_id"
    )
}

/// Check if a response body should be extracted to a file rather than inlined
fn should_use_file_body(body: &str, content_type: Option<&str>, threshold: usize) -> bool {
    if body.len() > threshold {
        return true;
    }

    if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.starts_with("image/")
            || ct_lower.starts_with("video/")
            || ct_lower.starts_with("audio/")
            || ct_lower.starts_with("font/")
            || ct_lower.contains("application/pdf")
            || ct_lower.contains("application/zip")
            || ct_lower.contains("application/octet-stream")
            || ct_lower.contains("text/html")
            || ct_lower.contains("text/css")
        {
            return true;
        }
    }

    false
}

/// Options for loading HAR files
#[derive(Debug, Clone)]
pub struct HarLoadOptions {
    /// Exclude OPTIONS preflight requests
    pub exclude_preflight: bool,
    /// Exclude redirect responses (3xx)
    pub exclude_redirects: bool,
    /// Strip browser-specific headers
    pub strip_browser_headers: bool,
    /// Convert absolute URLs to relative paths (default: true)
    pub normalize_urls: bool,
    /// Skip entries from non-Box domains (default: true)
    pub filter_non_box_domains: bool,
    /// Skip static asset entries like .js, .css, .png (default: true)
    pub exclude_static_assets: bool,
    /// Remove Authorization, Cookie, Set-Cookie headers (default: true)
    pub strip_sensitive_headers: bool,
    /// Remove date, server, x-envoy-*, alt-svc, etc. (default: true)
    pub strip_infrastructure_headers: bool,
    /// Remove access_token, api_key from query strings (default: true)
    pub strip_sensitive_query_params: bool,
    /// Directory for extracted body files (None = inline all bodies)
    pub body_output_dir: Option<PathBuf>,
    /// Size threshold for body extraction (default: 100KB)
    pub body_size_threshold: usize,
    /// Additional domains to treat as Box domains
    pub extra_box_domains: Vec<String>,
}

impl Default for HarLoadOptions {
    fn default() -> Self {
        Self {
            exclude_preflight: true,
            exclude_redirects: true,
            strip_browser_headers: true,
            normalize_urls: true,
            filter_non_box_domains: true,
            exclude_static_assets: true,
            strip_sensitive_headers: true,
            strip_infrastructure_headers: true,
            strip_sensitive_query_params: true,
            body_output_dir: None,
            body_size_threshold: DEFAULT_BODY_SIZE_THRESHOLD,
            extra_box_domains: Vec::new(),
        }
    }
}

/// HAR file loader
pub struct HarLoader {
    options: HarLoadOptions,
}

impl HarLoader {
    /// Create a new HAR loader with default options
    pub fn new() -> Self {
        Self {
            options: HarLoadOptions::default(),
        }
    }

    /// Create a new HAR loader with custom options
    pub fn with_options(options: HarLoadOptions) -> Self {
        Self { options }
    }

    /// Load HAR file and convert to mock definitions
    pub async fn load_from_file(&self, path: impl AsRef<Path>) -> Result<Vec<MockConfig>> {
        let content = tokio::fs::read_to_string(path.as_ref()).await?;
        let har: Har = serde_json::from_str(&content)?;

        self.convert_har_to_mocks(har).await
    }

    /// Convert HAR structure to mock definitions (simple 1:1 conversion)
    pub async fn convert_har_to_mocks(&self, har: Har) -> Result<Vec<MockConfig>> {
        let entries = match &har.log {
            Spec::V1_2(log) => &log.entries,
            Spec::V1_3(_) => return Err(anyhow::anyhow!("Unsupported HAR version")),
        };

        // Create bodies directory if body extraction is enabled
        if let Some(ref output_dir) = self.options.body_output_dir {
            let bodies_dir = output_dir.join("bodies");
            tokio::fs::create_dir_all(&bodies_dir)
                .await
                .context("Failed to create bodies directory")?;
        }

        let mut mocks = Vec::new();

        for (idx, entry) in entries.iter().enumerate() {
            // Apply filtering options
            if self.should_skip_entry(entry) {
                continue;
            }

            // Convert entry to mock - returns None if domain filtered
            if let Some(mock) = self.convert_entry_to_mock(entry, idx).await? {
                mocks.push(mock);
            }
        }

        Ok(mocks)
    }

    /// Check if an entry should be skipped based on filtering options
    fn should_skip_entry(&self, entry: &v1_2::Entries) -> bool {
        // Skip OPTIONS preflight requests
        if self.options.exclude_preflight && entry.request.method == "OPTIONS" {
            return true;
        }

        // Skip redirects
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if self.options.exclude_redirects && (300..400).contains(&(entry.response.status as u16)) {
            return true;
        }

        // Skip static assets
        if self.options.exclude_static_assets && is_static_asset(&entry.request.url) {
            return true;
        }

        false
    }

    /// Normalize a URL: convert absolute to relative, strip sensitive query params.
    /// Returns None if the domain is filtered out.
    fn normalize_url(&self, raw_url: &str) -> Option<String> {
        // Try parsing as an absolute URL
        if let Ok(parsed) = Url::parse(raw_url) {
            if let Some(host) = parsed.host_str() {
                // Filter non-Box domains
                if self.options.filter_non_box_domains
                    && !is_box_domain(host, &self.options.extra_box_domains)
                {
                    return None;
                }

                if self.options.normalize_urls {
                    // Build relative path with filtered query params
                    let path = parsed.path();
                    let query = self.filter_query_params(parsed.query_pairs());
                    if query.is_empty() {
                        return Some(path.to_string());
                    }
                    return Some(format!("{path}?{query}"));
                }
            }

            // Not normalizing, but still filter query params if enabled
            if self.options.strip_sensitive_query_params {
                let query = self.filter_query_params(parsed.query_pairs());
                let base = raw_url
                    .get(..raw_url.find('?').unwrap_or(raw_url.len()))
                    .unwrap_or(raw_url);
                if query.is_empty() {
                    return Some(base.to_string());
                }
                return Some(format!("{base}?{query}"));
            }

            return Some(raw_url.to_string());
        }

        // Already a relative URL - just handle query param stripping
        if self.options.strip_sensitive_query_params
            && let Some(query_start) = raw_url.find('?')
        {
            let path = raw_url.get(..query_start).unwrap_or(raw_url);
            let query_str = raw_url.get(query_start + 1..).unwrap_or("");
            let filtered: Vec<String> = query_str
                .split('&')
                .filter(|pair| {
                    let name = pair.split('=').next().unwrap_or("");
                    !is_sensitive_query_param(name)
                })
                .map(std::string::ToString::to_string)
                .collect();
            if filtered.is_empty() {
                return Some(path.to_string());
            }
            return Some(format!("{}?{}", path, filtered.join("&")));
        }

        Some(raw_url.to_string())
    }

    /// Filter query parameters, removing sensitive ones
    fn filter_query_params<'a, I>(&self, pairs: I) -> String
    where
        I: Iterator<Item = (std::borrow::Cow<'a, str>, std::borrow::Cow<'a, str>)>,
    {
        if !self.options.strip_sensitive_query_params {
            let all: Vec<String> = pairs.map(|(k, v)| format!("{k}={v}")).collect();
            return all.join("&");
        }

        let filtered: Vec<String> = pairs
            .filter(|(k, _)| !is_sensitive_query_param(k))
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        filtered.join("&")
    }

    /// Convert a HAR entry to a mock definition. Returns None if the entry's
    /// domain is filtered out.
    async fn convert_entry_to_mock(
        &self,
        entry: &v1_2::Entries,
        index: usize,
    ) -> Result<Option<MockConfig>> {
        let mock_id = format!("har-entry-{}", index + 1);

        // Normalize the URL (may return None if domain is filtered)
        let Some(normalized_url) = self.normalize_url(&entry.request.url) else {
            return Ok(None);
        };

        // Use exact matching with normalized URL
        let url_pattern = format!("exact:{normalized_url}");

        // Convert headers, stripping based on options
        let headers: FxHashMap<String, String> = entry
            .response
            .headers
            .iter()
            .filter(|h| !self.should_strip_header(&h.name))
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect();

        // Extract response body
        let body = entry.response.content.text.clone().unwrap_or_default();
        let content_type = entry.response.content.mime_type.as_deref();

        // Determine if body should be extracted to a file
        let (body_value, file_value) = if self.options.body_output_dir.is_some()
            && should_use_file_body(&body, content_type, self.options.body_size_threshold)
        {
            let file_path = format!("bodies/{mock_id}.body");
            if let Some(ref output_dir) = self.options.body_output_dir {
                let full_path = output_dir.join(&file_path);
                tokio::fs::write(&full_path, &body).await.with_context(|| {
                    format!("Failed to write body file: {}", full_path.display())
                })?;
            }
            (None, Some(file_path))
        } else {
            (Some(body), None)
        };

        // Calculate delay from timings (use wait time)
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let delay_ms = entry.timings.wait.max(0.0) as u64;

        Ok(Some(MockConfig {
            id: mock_id.into(),
            description: None,
            #[allow(clippy::cast_possible_truncation)]
            priority: 100u32.saturating_sub(index as u32),
            enabled: true,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                method: None,
                methods: vec![entry.request.method.clone()],
                url: None,
                urls: vec![url_pattern],
                headers: FxHashMap::default(),
                query: FxHashMap::default(),
                body: FxHashMap::default(),
                graphql: None,
            }),
            request: None,
            response_config: Some(ResponseConfig::Structured {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                status: Some(entry.response.status as u16),
                headers,
                body: body_value,
                template: None,
                file: file_value,
                template_file: None,
                json: Box::new(serde_json::Value::Null),
            }),
            patch: None,
            delay: if delay_ms > 0 {
                Some(format!("{delay_ms}ms"))
            } else {
                None
            },
        }))
    }

    /// Check if a header should be stripped
    fn should_strip_header(&self, name: &str) -> bool {
        let lower = name.to_lowercase();

        // Sensitive headers (auth, cookies)
        if self.options.strip_sensitive_headers
            && matches!(
                lower.as_str(),
                "authorization"
                    | "cookie"
                    | "set-cookie"
                    | "x-auth-token"
                    | "x-csrf-token"
                    | "proxy-authorization"
            )
        {
            return true;
        }

        // Browser-specific headers
        if self.options.strip_browser_headers
            && matches!(
                lower.as_str(),
                "user-agent"
                    | "accept-language"
                    | "accept-encoding"
                    | "cache-control"
                    | "connection"
                    | "upgrade-insecure-requests"
                    | "sec-fetch-site"
                    | "sec-fetch-mode"
                    | "sec-fetch-dest"
                    | "sec-ch-ua"
                    | "sec-ch-ua-mobile"
                    | "sec-ch-ua-platform"
                    | "referer"
                    | "origin"
            )
        {
            return true;
        }

        // Infrastructure headers (server, proxy, CDN)
        if self.options.strip_infrastructure_headers {
            if matches!(
                lower.as_str(),
                "date"
                    | "age"
                    | "server"
                    | "via"
                    | "server-timing"
                    | "alt-svc"
                    | "x-cache"
                    | "strict-transport-security"
                    | "expect-ct"
                    | "report-to"
                    | "nel"
            ) {
                return true;
            }

            // Prefix-based infrastructure headers
            if lower.starts_with("x-envoy-")
                || lower.starts_with("x-devgate-")
                || lower.starts_with("x-amz-")
                || lower.starts_with("x-cdn-")
                || lower.starts_with("x-forwarded-")
            {
                return true;
            }
        }

        false
    }
}

impl Default for HarLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_collect
)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- Helper --

    fn create_test_entry(method: &str, url: &str, status: i64) -> v1_2::Entries {
        create_test_entry_with_headers(method, url, status, vec![], None)
    }

    fn create_test_entry_with_headers(
        method: &str,
        url: &str,
        status: i64,
        response_headers: Vec<(&str, &str)>,
        body: Option<&str>,
    ) -> v1_2::Entries {
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
                headers: response_headers
                    .into_iter()
                    .map(|(n, v)| v1_2::Headers {
                        name: n.to_string(),
                        value: v.to_string(),
                        comment: None,
                    })
                    .collect(),
                content: v1_2::Content {
                    #[allow(clippy::cast_possible_wrap)]
                    size: body.map_or(0, |b| b.len() as i64),
                    compression: None,
                    mime_type: Some("application/json".to_string()),
                    text: Some(body.unwrap_or("{}").to_string()),
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

    fn make_har(entries: Vec<v1_2::Entries>) -> Har {
        Har {
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
        }
    }

    // -- is_box_domain --

    #[test]
    fn test_is_box_domain_standard() {
        assert!(is_box_domain("api.box.com", &[]));
        assert!(is_box_domain("app.box.com", &[]));
        assert!(is_box_domain("upload.box.com", &[]));
        assert!(is_box_domain("dl.boxcloud.com", &[]));
        assert!(is_box_domain("cdn01.boxcdn.net", &[]));
        assert!(is_box_domain("realtime.services.box.net", &[]));
        assert!(is_box_domain("box.com", &[]));
    }

    #[test]
    fn test_is_box_domain_enterprise() {
        assert!(is_box_domain("myorg.app.box.com", &[]));
        assert!(is_box_domain("myorg.ent.box.com", &[]));
        assert!(is_box_domain("fupload-us1.app.box.com", &[]));
    }

    #[test]
    fn test_is_box_domain_regional() {
        assert!(is_box_domain("us-east-1.boxcloud.com", &[]));
    }

    #[test]
    fn test_is_box_domain_negative() {
        assert!(!is_box_domain("google.com", &[]));
        assert!(!is_box_domain("api.github.com", &[]));
        assert!(!is_box_domain("cdn.jsdelivr.net", &[]));
        assert!(!is_box_domain("analytics.google.com", &[]));
    }

    #[test]
    fn test_is_box_domain_extra() {
        let extra = vec!["internal.mycompany.com".to_string()];
        assert!(is_box_domain("internal.mycompany.com", &extra));
        assert!(is_box_domain("api.internal.mycompany.com", &extra));
        assert!(!is_box_domain("google.com", &extra));
    }

    // -- is_static_asset --

    #[test]
    fn test_is_static_asset() {
        assert!(is_static_asset("https://cdn.example.com/app.js"));
        assert!(is_static_asset("https://cdn.example.com/style.css"));
        assert!(is_static_asset("https://cdn.example.com/logo.png"));
        assert!(is_static_asset("https://cdn.example.com/font.woff2"));
        assert!(is_static_asset("/assets/bundle.js?v=123"));
        assert!(is_static_asset("/images/icon.svg#fragment"));
    }

    #[test]
    fn test_is_not_static_asset() {
        assert!(!is_static_asset("https://api.box.com/2.0/users/me"));
        assert!(!is_static_asset("/2.0/files/123"));
        assert!(!is_static_asset("https://api.box.com/2.0/folders/0/items"));
    }

    // -- is_sensitive_query_param --

    #[test]
    fn test_is_sensitive_query_param() {
        assert!(is_sensitive_query_param("access_token"));
        assert!(is_sensitive_query_param("token"));
        assert!(is_sensitive_query_param("api_key"));
        assert!(is_sensitive_query_param("ACCESS_TOKEN"));
        assert!(!is_sensitive_query_param("fields"));
        assert!(!is_sensitive_query_param("limit"));
        assert!(!is_sensitive_query_param("offset"));
    }

    // -- should_use_file_body --

    #[test]
    fn test_should_use_file_body_large() {
        let large_body = "x".repeat(200 * 1024);
        assert!(should_use_file_body(
            &large_body,
            Some("application/json"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
    }

    #[test]
    fn test_should_use_file_body_small() {
        assert!(!should_use_file_body(
            "{}",
            Some("application/json"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
    }

    #[test]
    fn test_should_use_file_body_binary_content_type() {
        assert!(should_use_file_body(
            "small",
            Some("image/png"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
        assert!(should_use_file_body(
            "small",
            Some("application/pdf"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
        assert!(should_use_file_body(
            "small",
            Some("text/html"),
            DEFAULT_BODY_SIZE_THRESHOLD
        ));
    }

    // -- URL normalization --

    #[test]
    fn test_normalize_url_absolute_box() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("https://api.box.com/2.0/users/me"),
            Some("/2.0/users/me".to_string())
        );
    }

    #[test]
    fn test_normalize_url_preserves_query() {
        let loader = HarLoader::new();
        assert_eq!(
            loader
                .normalize_url("https://api.box.com/2.0/folders/0/items?fields=name,id&limit=100"),
            Some("/2.0/folders/0/items?fields=name,id&limit=100".to_string())
        );
    }

    #[test]
    fn test_normalize_url_strips_access_token() {
        let loader = HarLoader::new();
        assert_eq!(
            loader
                .normalize_url("https://api.box.com/2.0/users/me?access_token=SECRET&fields=name"),
            Some("/2.0/users/me?fields=name".to_string())
        );
    }

    #[test]
    fn test_normalize_url_filters_non_box_domain() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("https://www.google.com/analytics"),
            None
        );
    }

    #[test]
    fn test_normalize_url_already_relative() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("/2.0/users/me"),
            Some("/2.0/users/me".to_string())
        );
    }

    #[test]
    fn test_normalize_url_enterprise_domain() {
        let loader = HarLoader::new();
        assert_eq!(
            loader.normalize_url("https://myorg.app.box.com/api/oauth2/token"),
            Some("/api/oauth2/token".to_string())
        );
    }

    #[test]
    fn test_normalize_url_disabled() {
        let loader = HarLoader::with_options(HarLoadOptions {
            normalize_urls: false,
            filter_non_box_domains: false,
            ..Default::default()
        });
        // Should keep the absolute URL but still strip sensitive params
        let result = loader.normalize_url("https://api.box.com/2.0/users/me?access_token=SECRET");
        assert_eq!(result, Some("https://api.box.com/2.0/users/me".to_string()));
    }

    // -- should_skip_entry --

    #[test]
    fn test_skip_static_assets() {
        let loader = HarLoader::new();
        let entry = create_test_entry("GET", "https://cdn.box.com/app.js", 200);
        assert!(loader.should_skip_entry(&entry));
    }

    #[test]
    fn test_keep_api_calls() {
        let loader = HarLoader::new();
        let entry = create_test_entry("GET", "https://api.box.com/2.0/users/me", 200);
        assert!(!loader.should_skip_entry(&entry));
    }

    // -- Header stripping --

    #[test]
    fn test_strip_sensitive_headers() {
        let loader = HarLoader::new();
        assert!(loader.should_strip_header("Authorization"));
        assert!(loader.should_strip_header("Cookie"));
        assert!(loader.should_strip_header("Set-Cookie"));
        assert!(loader.should_strip_header("x-csrf-token"));
    }

    #[test]
    fn test_strip_infrastructure_headers() {
        let loader = HarLoader::new();
        assert!(loader.should_strip_header("date"));
        assert!(loader.should_strip_header("server"));
        assert!(loader.should_strip_header("x-envoy-upstream-service-time"));
        assert!(loader.should_strip_header("alt-svc"));
        assert!(loader.should_strip_header("x-amz-request-id"));
        assert!(loader.should_strip_header("x-forwarded-for"));
    }

    #[test]
    fn test_keep_content_headers() {
        let loader = HarLoader::new();
        assert!(!loader.should_strip_header("content-type"));
        assert!(!loader.should_strip_header("content-length"));
        assert!(!loader.should_strip_header("x-request-id"));
        assert!(!loader.should_strip_header("box-request-id"));
    }

    #[test]
    fn test_keep_sensitive_headers_when_disabled() {
        let loader = HarLoader::with_options(HarLoadOptions {
            strip_sensitive_headers: false,
            ..Default::default()
        });
        assert!(!loader.should_strip_header("Authorization"));
        assert!(!loader.should_strip_header("Cookie"));
    }

    // -- Domain filtering integration --

    #[tokio::test]
    async fn test_filter_non_box_domains() {
        let har = make_har(vec![
            create_test_entry("GET", "https://api.box.com/2.0/users/me", 200),
            create_test_entry("GET", "https://www.google.com/analytics", 200),
            create_test_entry("POST", "https://upload.box.com/api/2.0/files/content", 201),
            create_test_entry("GET", "https://cdn.jsdelivr.net/npm/react", 200),
        ]);

        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        // Only the 2 Box domain entries should remain
        assert_eq!(mocks.len(), 2);
        let urls: Vec<&str> = mocks
            .iter()
            .filter_map(|m| m.match_config.as_ref())
            .flat_map(|mc| mc.urls.iter())
            .map(std::string::String::as_str)
            .collect();
        assert!(
            urls.iter()
                .all(|u| u.contains("/2.0/") || u.contains("/api/"))
        );
    }

    #[tokio::test]
    async fn test_all_domains_disabled() {
        let har = make_har(vec![
            create_test_entry("GET", "https://api.box.com/2.0/users/me", 200),
            create_test_entry("GET", "https://www.google.com/analytics", 200),
        ]);

        let loader = HarLoader::with_options(HarLoadOptions {
            filter_non_box_domains: false,
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 2);
    }

    // -- URL normalization integration --

    #[tokio::test]
    async fn test_urls_normalized_to_relative() {
        let har = make_har(vec![create_test_entry(
            "GET",
            "https://api.box.com/2.0/users/me",
            200,
        )]);

        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let mc = mocks[0].match_config.as_ref().unwrap();
        assert_eq!(mc.urls[0], "exact:/2.0/users/me");
    }

    #[tokio::test]
    async fn test_absolute_urls_preserved_when_disabled() {
        let har = make_har(vec![create_test_entry(
            "GET",
            "https://api.box.com/2.0/users/me",
            200,
        )]);

        let loader = HarLoader::with_options(HarLoadOptions {
            normalize_urls: false,
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let mc = mocks[0].match_config.as_ref().unwrap();
        assert_eq!(mc.urls[0], "exact:https://api.box.com/2.0/users/me");
    }

    // -- Body extraction --

    #[tokio::test]
    async fn test_body_extraction_large() {
        let temp_dir = TempDir::new().unwrap();
        let large_body = "x".repeat(200 * 1024);

        let mut entry = create_test_entry("GET", "https://api.box.com/2.0/files/123", 200);
        entry.response.content.text = Some(large_body.clone());

        let har = make_har(vec![entry]);

        let loader = HarLoader::with_options(HarLoadOptions {
            body_output_dir: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let rc = mocks[0].response_config.as_ref().unwrap();
        // Body should be in file, not inline
        assert_eq!(rc.file_ref(), Some(&"bodies/har-entry-1.body".to_string()));
        assert!(rc.body().is_none());

        // Verify file was written
        let file_content =
            tokio::fs::read_to_string(temp_dir.path().join("bodies/har-entry-1.body"))
                .await
                .unwrap();
        assert_eq!(file_content, large_body);
    }

    #[tokio::test]
    async fn test_body_inline_small() {
        let temp_dir = TempDir::new().unwrap();

        let har = make_har(vec![create_test_entry(
            "GET",
            "https://api.box.com/2.0/users/me",
            200,
        )]);

        let loader = HarLoader::with_options(HarLoadOptions {
            body_output_dir: Some(temp_dir.path().to_path_buf()),
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let rc = mocks[0].response_config.as_ref().unwrap();
        // Small body should be inline
        assert!(rc.body().is_some());
        assert!(rc.file_ref().is_none());
    }

    // -- End-to-end --

    #[tokio::test]
    async fn test_end_to_end_clean_conversion() {
        let har = make_har(vec![
            // Box API call - should be kept with relative URL
            create_test_entry_with_headers(
                "GET",
                "https://api.box.com/2.0/users/me?access_token=SECRET_TOKEN&fields=name",
                200,
                vec![
                    ("content-type", "application/json"),
                    ("Authorization", "Bearer tok_123"),
                    ("x-envoy-upstream-service-time", "42"),
                    ("date", "Mon, 01 Jan 2024 00:00:00 GMT"),
                    ("box-request-id", "abc123"),
                ],
                Some(r#"{"id":"123","name":"Test"}"#),
            ),
            // Static asset - should be filtered
            create_test_entry("GET", "https://cdn.box.com/static/app.js", 200),
            // Non-Box domain - should be filtered
            create_test_entry("GET", "https://www.google-analytics.com/collect", 200),
            // OPTIONS preflight - should be filtered
            create_test_entry("OPTIONS", "https://api.box.com/2.0/files", 204),
        ]);

        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        // Only the first entry should survive
        assert_eq!(mocks.len(), 1);
        let mock = &mocks[0];

        // URL should be relative and access_token stripped
        let mc = mock.match_config.as_ref().unwrap();
        assert_eq!(mc.urls[0], "exact:/2.0/users/me?fields=name");

        // Sensitive and infrastructure headers should be stripped
        let rc = mock.response_config.as_ref().unwrap();
        let headers = rc.headers().unwrap();
        assert!(headers.contains_key("content-type"));
        assert!(headers.contains_key("box-request-id"));
        assert!(!headers.contains_key("Authorization"));
        assert!(!headers.contains_key("x-envoy-upstream-service-time"));
        assert!(!headers.contains_key("date"));
    }

    // -- File loading --

    #[tokio::test]
    async fn test_load_har_file() {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let har_path = temp_dir.path().join("test.har");

        let har_content = r#"{
      "log": {
        "version": "1.2",
        "creator": {
          "name": "test",
          "version": "1.0"
        },
        "entries": [
          {
            "startedDateTime": "2025-10-07T12:00:00.000Z",
            "time": 50,
            "request": {
              "method": "GET",
              "url": "https://api.box.com/2.0/users/me",
              "httpVersion": "HTTP/1.1",
              "headers": [],
              "queryString": [],
              "cookies": [],
              "headersSize": -1,
              "bodySize": 0
            },
            "response": {
              "status": 200,
              "statusText": "OK",
              "httpVersion": "HTTP/1.1",
              "headers": [
                {
                  "name": "content-type",
                  "value": "application/json"
                }
              ],
              "cookies": [],
              "content": {
                "size": 100,
                "mimeType": "application/json",
                "text": "{\"id\":\"123\",\"name\":\"Test User\"}"
              },
              "redirectURL": "",
              "headersSize": -1,
              "bodySize": 100
            },
            "cache": {},
            "timings": {
              "send": 0,
              "wait": 50,
              "receive": 0
            }
          }
        ]
      }
    }"#;

        tokio::fs::write(&har_path, har_content)
            .await
            .expect("Failed to write HAR file");

        let loader = HarLoader::new();
        let mocks = loader
            .load_from_file(&har_path)
            .await
            .expect("Failed to load HAR file");

        assert_eq!(mocks.len(), 1);
        let match_config = mocks[0]
            .match_config
            .as_ref()
            .expect("match_config should exist");
        let response_config = mocks[0]
            .response_config
            .as_ref()
            .expect("response_config should exist");
        assert_eq!(match_config.methods[0], "GET");
        assert_eq!(response_config.status().expect("status should exist"), 200);
        // Should now be a relative path
        assert_eq!(match_config.urls[0], "exact:/2.0/users/me");
    }

    #[tokio::test]
    async fn test_exclude_preflight() {
        let har = make_har(vec![
            create_test_entry("OPTIONS", "https://api.box.com/test", 204),
            create_test_entry("GET", "https://api.box.com/test", 200),
        ]);

        let loader = HarLoader::new();
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 1);
        let match_config = mocks[0]
            .match_config
            .as_ref()
            .expect("match_config should exist");
        assert_eq!(match_config.methods[0], "GET");
    }

    #[tokio::test]
    async fn test_extra_box_domains() {
        let har = make_har(vec![
            create_test_entry("GET", "https://api.box.com/2.0/users/me", 200),
            create_test_entry("GET", "https://internal.mycompany.com/api/data", 200),
        ]);

        let loader = HarLoader::with_options(HarLoadOptions {
            extra_box_domains: vec!["internal.mycompany.com".to_string()],
            ..Default::default()
        });
        let mocks = loader
            .convert_har_to_mocks(har)
            .await
            .expect("conversion failed");

        assert_eq!(mocks.len(), 2);
    }
}
