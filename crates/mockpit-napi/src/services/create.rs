use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsCreateInput {
    pub url: String,
    pub method: Option<String>,
    pub status: Option<u32>,
    pub body: Option<String>,
    pub template: Option<bool>,
    pub id: Option<String>,
    pub priority: Option<u32>,
    pub collection: Option<String>,
    pub format: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsCreateResult {
    pub mock_id: String,
    pub content: String,
}

#[napi(namespace = "services")]
pub fn create(input: JsCreateInput) -> Result<JsCreateResult> {
    let result = mockpit::services::create::create(mockpit::services::create::CreateInput {
        url: input.url,
        method: input.method.unwrap_or_else(|| "GET".into()),
        status: input.status.unwrap_or(200) as u16,
        body: input.body,
        template: input.template.unwrap_or(false),
        id: input.id,
        priority: input.priority.unwrap_or(100),
        collection: input.collection,
        format: input.format.unwrap_or_else(|| "yaml".into()),
    })
    .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsCreateResult {
        mock_id: result.mock_id,
        content: result.content,
    })
}
