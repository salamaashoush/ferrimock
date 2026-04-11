use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsServeInput {
    pub port: Option<u32>,
    pub host: Option<String>,
    pub mocks_dir: Option<String>,
    pub mock_file: Option<String>,
    pub watch: Option<bool>,
    pub cors: Option<bool>,
    pub verbose: Option<bool>,
}

#[napi(object, namespace = "services")]
pub struct JsServeResult {
    pub port: u32,
    pub url: String,
}

/// Start a standalone mock server using the service layer.
///
/// Returns the URL and port. The server runs until the process exits.
/// For more control (stop/restart), use the MockpitServer class instead.
#[napi(namespace = "services")]
pub async fn serve(input: JsServeInput) -> Result<JsServeResult> {
    let handle = mockpit::services::serve::start(mockpit::services::serve::ServeInput {
        port: input.port.unwrap_or(3006) as u16,
        host: input.host.unwrap_or_else(|| "127.0.0.1".into()),
        mocks_dir: input.mocks_dir,
        mock_file: input.mock_file,
        watch: input.watch.unwrap_or(false),
        cors: input.cors.unwrap_or(false),
        enable_management_endpoints: false,
        log_matches: false,
        verbose: input.verbose.unwrap_or(false),
    })
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    let result = JsServeResult {
        port: u32::from(handle.port),
        url: handle.url.clone(),
    };

    // Leak the handle so the server keeps running until process exits.
    // For controlled lifecycle, use the MockpitServer class instead.
    std::mem::forget(handle);

    Ok(result)
}
