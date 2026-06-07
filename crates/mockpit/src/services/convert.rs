//! HAR to mock conversion service.

use crate::config::{HarLoadOptions, HarLoader, MockConfig};

/// Input for HAR conversion.
#[derive(Debug, Clone)]
pub struct ConvertInput {
    pub input: String,
    pub format: String,
    pub exclude_preflight: bool,
    pub exclude_redirects: bool,
    pub strip_browser_headers: bool,
    pub normalize_urls: bool,
    pub allowed_domains: Vec<String>,
    pub exclude_static_assets: bool,
    pub strip_sensitive_headers: bool,
    pub strip_infrastructure_headers: bool,
    pub extract_bodies: bool,
    pub body_threshold_kb: usize,
}

impl Default for ConvertInput {
    fn default() -> Self {
        Self {
            input: String::new(),
            format: "yaml".into(),
            exclude_preflight: true,
            exclude_redirects: true,
            strip_browser_headers: true,
            normalize_urls: true,
            allowed_domains: Vec::new(),
            exclude_static_assets: true,
            strip_sensitive_headers: true,
            strip_infrastructure_headers: true,
            extract_bodies: false,
            body_threshold_kb: 100,
        }
    }
}

/// Result of HAR conversion.
#[derive(Debug, Clone)]
pub struct ConvertResult {
    pub mocks: Vec<MockConfig>,
    pub entries_processed: usize,
    pub content: String,
}

/// Convert a HAR file to mock definitions.
pub async fn convert(input: ConvertInput) -> Result<ConvertResult, crate::MockpitError> {
    let mut options = HarLoadOptions {
        exclude_preflight: input.exclude_preflight,
        exclude_redirects: input.exclude_redirects,
        strip_browser_headers: input.strip_browser_headers,
        normalize_urls: input.normalize_urls,
        exclude_static_assets: input.exclude_static_assets,
        strip_sensitive_headers: input.strip_sensitive_headers,
        strip_infrastructure_headers: input.strip_infrastructure_headers,
        ..Default::default()
    };

    if !input.allowed_domains.is_empty() {
        let domains = input.allowed_domains.clone();
        let filter: std::sync::Arc<dyn crate::config::DomainFilter> =
            std::sync::Arc::new(move |host: &str| {
                let lower = host.to_lowercase();
                domains.iter().any(|d| {
                    let d = d.to_lowercase();
                    lower == d || lower.ends_with(&format!(".{d}"))
                })
            });
        options.domain_filter = Some(filter);
    }

    let loader = HarLoader::with_options(options);
    let mocks = loader.load_from_file(&input.input).await?;

    let entries_processed = mocks.len();

    // Build collection config manually
    let collection = serde_json::json!({
        "name": format!("Converted from {}", input.input),
        "enabled": true,
        "mocks": mocks,
    });

    let content = match input.format.as_str() {
        "json" => serde_json::to_string_pretty(&collection)?,
        _ => serde_yaml::to_string(&collection)?,
    };

    Ok(ConvertResult {
        mocks,
        entries_processed,
        content,
    })
}
