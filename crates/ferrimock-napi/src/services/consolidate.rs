use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsConsolidateInput {
    pub input: String,
    pub format: Option<String>,
    pub min_pattern: Option<u32>,
    pub enable_templates: Option<bool>,
}

#[napi(object, namespace = "services")]
pub struct JsConsolidateResult {
    pub content: String,
    pub mocks_before: u32,
    pub mocks_after: u32,
    pub input_size: f64,
    pub output_size: f64,
}

#[napi(namespace = "services")]
pub async fn consolidate(input: JsConsolidateInput) -> Result<JsConsolidateResult> {
    let result = ferrimock::services::consolidate::consolidate(
        ferrimock::services::consolidate::ConsolidateInput {
            input: input.input,
            format: input.format.unwrap_or_else(|| "json".into()),
            min_pattern: input.min_pattern.unwrap_or(3) as usize,
            enable_templates: input.enable_templates.unwrap_or(true),
        },
    )
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsConsolidateResult {
        content: result.content,
        mocks_before: result.mocks_before as u32,
        mocks_after: result.mocks_after as u32,
        input_size: result.input_size as f64,
        output_size: result.output_size as f64,
    })
}
