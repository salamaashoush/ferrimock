use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(namespace = "services")]
pub async fn show(mock_id: String, mocks_dir: Option<String>) -> Result<Option<serde_json::Value>> {
    let mock = mockpit::services::show::show(&mock_id, mocks_dir.as_deref())
        .await
        .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(mock.map(|m| {
        serde_json::json!({
            "id": m.id.to_string(),
            "priority": m.priority,
            "enabled": m.enabled,
            "methods": m.request.methods.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
            "url_patterns": m.request.url_patterns.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>(),
            "status": m.response.status.as_u16(),
            "has_header_matchers": !m.request.header_matchers.is_empty(),
            "has_delay": m.response.delay.is_some(),
            "scope": m.scope.as_ref().map(|s| s.to_string()),
        })
    }))
}
