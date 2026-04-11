use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsExportInput {
    pub mocks_dir: Option<String>,
    pub filter: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsExportResult {
    pub content: String,
    pub mocks_exported: u32,
}

#[napi(namespace = "services")]
pub async fn export(input: JsExportInput) -> Result<JsExportResult> {
    let result = mockpit::services::export::export(mockpit::services::export::ExportInput {
        mocks_dir: input.mocks_dir,
        filter: input.filter,
    })
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsExportResult {
        content: result.content,
        mocks_exported: result.mocks_exported as u32,
    })
}
