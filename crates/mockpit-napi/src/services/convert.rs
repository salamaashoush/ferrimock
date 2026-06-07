use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsConvertInput {
    pub input: String,
    pub format: Option<String>,
    pub exclude_preflight: Option<bool>,
    pub exclude_redirects: Option<bool>,
    pub strip_browser_headers: Option<bool>,
    pub normalize_urls: Option<bool>,
    pub allowed_domains: Option<Vec<String>>,
    pub exclude_static_assets: Option<bool>,
    pub strip_sensitive_headers: Option<bool>,
    pub strip_infrastructure_headers: Option<bool>,
}

#[napi(object, namespace = "services")]
pub struct JsConvertResult {
    pub entries_processed: u32,
    pub mocks_count: u32,
    pub content: String,
}

#[napi(namespace = "services")]
pub async fn convert(input: JsConvertInput) -> Result<JsConvertResult> {
    let defaults = mockpit::services::convert::ConvertInput::default();
    let result = mockpit::services::convert::convert(mockpit::services::convert::ConvertInput {
        input: input.input,
        format: input.format.unwrap_or(defaults.format),
        exclude_preflight: input
            .exclude_preflight
            .unwrap_or(defaults.exclude_preflight),
        exclude_redirects: input
            .exclude_redirects
            .unwrap_or(defaults.exclude_redirects),
        strip_browser_headers: input
            .strip_browser_headers
            .unwrap_or(defaults.strip_browser_headers),
        normalize_urls: input.normalize_urls.unwrap_or(defaults.normalize_urls),
        allowed_domains: input.allowed_domains.unwrap_or_default(),
        exclude_static_assets: input
            .exclude_static_assets
            .unwrap_or(defaults.exclude_static_assets),
        strip_sensitive_headers: input
            .strip_sensitive_headers
            .unwrap_or(defaults.strip_sensitive_headers),
        strip_infrastructure_headers: input
            .strip_infrastructure_headers
            .unwrap_or(defaults.strip_infrastructure_headers),
        extract_bodies: false,
        body_threshold_kb: 100,
    })
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsConvertResult {
        entries_processed: result.entries_processed as u32,
        mocks_count: result.mocks.len() as u32,
        content: result.content,
    })
}
