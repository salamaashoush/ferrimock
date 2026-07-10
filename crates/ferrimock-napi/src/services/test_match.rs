use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;

#[napi(object, namespace = "services")]
pub struct JsTestMatchInput {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
    pub render: Option<bool>,
    pub mocks_dir: Option<String>,
    pub mock_file: Option<String>,
}

#[napi(namespace = "services")]
pub async fn test_match(input: JsTestMatchInput) -> Result<serde_json::Value> {
    let headers: Vec<(String, String)> = input.headers.unwrap_or_default().into_iter().collect();

    let result = ferrimock::services::test_match::test_match(
        ferrimock::services::test_match::TestMatchInput {
            method: input.method,
            path: input.path,
            query: input.query,
            headers,
            body: input.body,
            render: input.render.unwrap_or(false),
            mocks_dir: input.mocks_dir,
            mock_file: input.mock_file,
        },
    )
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    serde_json::to_value(&result).map_err(|e| Error::from_reason(e.to_string()))
}
