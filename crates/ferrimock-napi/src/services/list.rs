use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsListInput {
    pub mocks_dir: Option<String>,
    pub filter: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsMockSummary {
    pub id: String,
    pub priority: u32,
    pub enabled: bool,
    pub methods: Vec<String>,
    pub url_patterns: Vec<String>,
    pub status: u32,
    pub has_header_matchers: bool,
    pub has_delay: bool,
    pub scope: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsListOutput {
    pub mocks: Vec<JsMockSummary>,
    pub total: u32,
}

#[napi(namespace = "services")]
pub async fn list(input: JsListInput) -> Result<JsListOutput> {
    let result = ferrimock::services::list::list(ferrimock::services::list::ListInput {
        mocks_dir: input.mocks_dir,
        filter: input.filter,
    })
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsListOutput {
        total: result.total as u32,
        mocks: result
            .mocks
            .into_iter()
            .map(|m| JsMockSummary {
                id: m.id,
                priority: m.priority,
                enabled: m.enabled,
                methods: m.methods,
                url_patterns: m.url_patterns,
                status: u32::from(m.status),
                has_header_matchers: m.has_header_matchers,
                has_delay: m.has_delay,
                scope: m.scope,
            })
            .collect(),
    })
}
