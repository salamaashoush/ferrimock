//! Re-export all types from bdg-mock-types and provide template rendering extensions

// Re-export everything from bdg-mock-types
pub use mockpit_types::*;

use http::HeaderMap;
use rustc_hash::FxHashMap;

/// Extension trait for ResponseGenerator to add template rendering support
///
/// This trait provides the original `generate()` and `generate_dynamic()` methods
/// that were removed from bdg-mock-types to avoid circular dependencies.
pub trait ResponseGeneratorExt {
  /// Generate the response as bytes (supports templates)
  fn generate(&self) -> impl std::future::Future<Output = Result<bytes::Bytes, anyhow::Error>> + Send;

  /// Generate the response with request context (supports templates)
  fn generate_with_context(
    &self,
    context: &RequestContext,
  ) -> impl std::future::Future<Output = Result<bytes::Bytes, anyhow::Error>> + Send;

  /// Generate response dynamically (for templates with request context)
  /// Returns DynamicResponse which may override status and headers
  #[allow(clippy::too_many_arguments)]
  fn generate_dynamic(
    &self,
    method: &str,
    uri: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
    captures: FxHashMap<String, String>,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
  ) -> impl std::future::Future<Output = Result<DynamicResponse, anyhow::Error>> + Send;

  /// Synchronous response generation for Inline, FileCached, and Template bodies.
  ///
  /// Returns `Err` with `"NEEDS_ASYNC"` for `File` bodies (which need tokio::fs::read)
  /// or if the mock has a delay configured (which needs tokio::time::sleep).
  ///
  /// This avoids the ~98us NAPI async bridge overhead for the common case where
  /// no actual I/O is needed.
  #[allow(clippy::too_many_arguments)]
  fn generate_dynamic_sync(
    &self,
    method: &str,
    uri: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
    captures: FxHashMap<String, String>,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
  ) -> Result<DynamicResponse, anyhow::Error>;

  /// Check if this response can be generated synchronously (no file I/O, no delay).
  fn can_generate_sync(&self) -> bool;
}

/// Fast base64 detection using byte-level checks.
/// Short-circuits immediately for JSON/HTML/text (first byte is `{`, `[`, `"`, `<`).
fn is_likely_base64(s: &str) -> bool {
  let bytes = s.trim().as_bytes();
  match bytes.first() {
    Some(b'{') | Some(b'[') | Some(b'"') | Some(b'<') | None => false,
    _ => bytes
      .iter()
      .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='),
  }
}

/// Try to decode a rendered string as base64 binary data.
/// Returns `Some(Bytes)` if the output looks like pure base64, `None` otherwise.
fn try_decode_base64(rendered: &str) -> Option<bytes::Bytes> {
  if is_likely_base64(rendered) {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, rendered.trim())
      .ok()
      .map(bytes::Bytes::from)
  } else {
    None
  }
}

/// Render a template and convert to DynamicResponse, handling base64 and structured responses.
fn render_to_dynamic(
  template: &str,
  hash: u64,
  context: &RequestContext,
  mock_id: Option<&str>,
  structured_response: bool,
) -> Result<DynamicResponse, anyhow::Error> {
  let rendered = mockpit_template::render_template_with_hash(template, hash, context, mock_id)
    .map_err(|e| anyhow::anyhow!("Template rendering failed: {}", e))?;

  if let Some(decoded) = try_decode_base64(&rendered) {
    return Ok(DynamicResponse::body_only(decoded));
  }

  if structured_response {
    Ok(DynamicResponse::from_rendered_string(rendered))
  } else {
    Ok(DynamicResponse::body_only(bytes::Bytes::from(rendered)))
  }
}

impl ResponseGeneratorExt for ResponseGenerator {
  async fn generate(&self) -> Result<bytes::Bytes, anyhow::Error> {
    self.generate_with_context(&RequestContext::new()).await
  }

  async fn generate_with_context(&self, context: &RequestContext) -> Result<bytes::Bytes, anyhow::Error> {
    if let Some(delay) = self.delay {
      tokio::time::sleep(delay).await;
    }

    match &self.body {
      BodySource::Inline(cached_bytes) => Ok((**cached_bytes).clone()),
      BodySource::File(path) => {
        let content = tokio::fs::read(path).await?;
        Ok(bytes::Bytes::from(content))
      },
      BodySource::FileCached(cached_bytes) => Ok((**cached_bytes).clone()),
      BodySource::Template { source, hash } => {
        let dynamic = render_to_dynamic(source, *hash, context, None, self.structured_response)?;
        Ok(dynamic.body)
      },
    }
  }

  #[allow(clippy::too_many_arguments)]
  async fn generate_dynamic(
    &self,
    method: &str,
    uri: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
    captures: FxHashMap<String, String>,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
  ) -> Result<DynamicResponse, anyhow::Error> {
    if let Some(delay) = self.delay {
      tokio::time::sleep(delay).await;
    }

    match &self.body {
      BodySource::Template { source, hash } => {
        let mut context = RequestContext::from_request(method, uri, query, headers, body);
        context.captures = captures;
        context.vars = vars.cloned();
        render_to_dynamic(source, *hash, &context, None, self.structured_response)
      },
      _ => {
        let body_bytes = self.generate().await?;
        Ok(DynamicResponse::body_only(body_bytes))
      },
    }
  }

  #[allow(clippy::too_many_arguments)]
  fn generate_dynamic_sync(
    &self,
    method: &str,
    uri: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
    captures: FxHashMap<String, String>,
    vars: Option<&serde_json::Map<String, serde_json::Value>>,
  ) -> Result<DynamicResponse, anyhow::Error> {
    if self.delay.is_some() {
      return Err(anyhow::anyhow!("NEEDS_ASYNC"));
    }

    match &self.body {
      BodySource::File(_) => Err(anyhow::anyhow!("NEEDS_ASYNC")),
      BodySource::Template { source, hash } => {
        let mut context = RequestContext::from_request(method, uri, query, headers, body);
        context.captures = captures;
        context.vars = vars.cloned();
        render_to_dynamic(source, *hash, &context, None, self.structured_response)
      },
      BodySource::Inline(cached_bytes) => Ok(DynamicResponse::body_only((**cached_bytes).clone())),
      BodySource::FileCached(cached_bytes) => Ok(DynamicResponse::body_only((**cached_bytes).clone())),
    }
  }

  fn can_generate_sync(&self) -> bool {
    self.delay.is_none() && !matches!(&self.body, BodySource::File(_))
  }
}
