use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsTemplateInput {
    pub template: String,
    pub context: Option<serde_json::Value>,
    pub count: Option<u32>,
}

#[napi(namespace = "services")]
pub fn render_template(input: JsTemplateInput) -> Result<Vec<String>> {
    mockpit::services::template::render(mockpit::services::template::TemplateInput {
        template: input.template,
        context: input.context,
        count: input.count.unwrap_or(1) as usize,
    })
    .map_err(|e| Error::from_reason(e.to_string()))
}
