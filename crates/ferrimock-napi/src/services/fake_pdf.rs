use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsFakePdfInput {
    pub pages: Option<u32>,
    pub text: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsFakePdfResult {
    pub base64: String,
}

#[napi(namespace = "services")]
pub fn fake_pdf(input: JsFakePdfInput) -> Result<JsFakePdfResult> {
    let result =
        ferrimock::services::fake_pdf::generate(ferrimock::services::fake_pdf::FakePdfInput {
            pages: input.pages.unwrap_or(1),
            text: input.text,
        })
        .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsFakePdfResult {
        base64: result.base64,
    })
}
