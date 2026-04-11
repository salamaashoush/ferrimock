use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsFormatInput {
    pub path: String,
    pub check: Option<bool>,
}

#[napi(object, namespace = "services")]
pub struct JsFormatFileResult {
    pub path: String,
    pub changed: bool,
    pub error: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsFormatOutput {
    pub files: Vec<JsFormatFileResult>,
    pub formatted_count: u32,
    pub error_count: u32,
    pub unchanged_count: u32,
}

#[napi(namespace = "services")]
pub fn format(input: JsFormatInput) -> Result<JsFormatOutput> {
    let result = mockpit::services::format::format_path(mockpit::services::format::FormatInput {
        path: input.path,
        check: input.check.unwrap_or(false),
    })
    .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsFormatOutput {
        formatted_count: result.formatted_count as u32,
        error_count: result.error_count as u32,
        unchanged_count: result.unchanged_count as u32,
        files: result
            .files
            .into_iter()
            .map(|f| JsFormatFileResult {
                path: f.path,
                changed: f.changed,
                error: f.error,
            })
            .collect(),
    })
}

#[napi(namespace = "services")]
pub fn format_content(content: String, file_format: String) -> Result<String> {
    mockpit::services::format::format_content(mockpit::services::format::FormatContentInput {
        content,
        file_format,
    })
    .map_err(|e| Error::from_reason(e.to_string()))
}
