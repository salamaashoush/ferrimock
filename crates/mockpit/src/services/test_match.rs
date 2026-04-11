//! Mock match testing service — test if a request matches any mock.

use crate::engine::{MockMatcher, MockRegistry};
use rustc_hash::FxHashMap;

/// Input for testing mock matching.
#[derive(Debug, Clone, Default)]
pub struct TestMatchInput {
    /// HTTP method
    pub method: String,
    /// Request path
    pub path: String,
    /// Query string (without leading ?)
    pub query: Option<String>,
    /// Request headers as key-value pairs
    pub headers: Vec<(String, String)>,
    /// Request body
    pub body: Option<String>,
    /// Whether to render the response (template evaluation)
    pub render: bool,
    /// Mock collections directory
    pub mocks_dir: Option<String>,
    /// Specific mock file to load
    pub mock_file: Option<String>,
}

/// Result of a mock match test.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TestMatchResult {
    /// Whether a mock was matched
    pub matched: bool,
    /// Matched mock ID
    pub mock_id: Option<String>,
    /// Mock priority
    pub priority: Option<u32>,
    /// URL captures from pattern matching
    pub captures: FxHashMap<String, String>,
    /// Rendered response (if render was requested and match found)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<RenderedResponse>,
}

/// A rendered mock response.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RenderedResponse {
    pub status: u16,
    pub headers: FxHashMap<String, String>,
    pub body: String,
    /// Base64-encoded body (for binary content)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_base64: Option<String>,
}

/// Test if a request matches any loaded mock.
pub async fn test_match(input: TestMatchInput) -> Result<TestMatchResult, anyhow::Error> {
    let method: http::Method = input.method.parse()?;

    // Build headers
    let mut header_map = http::HeaderMap::new();
    for (name, value) in &input.headers {
        let name = http::header::HeaderName::try_from(name.as_str())?;
        let value = http::header::HeaderValue::try_from(value.as_str())?;
        header_map.insert(name, value);
    }

    // Load mocks
    let registry = MockRegistry::new();

    if let Some(ref file) = input.mock_file {
        let path = std::path::Path::new(file);
        registry
            .load_collection_file(path)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
    } else {
        let default_dir = std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());
        let dir = input.mocks_dir.as_deref().unwrap_or(&default_dir);
        registry
            .load_from_directory(dir)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
    }

    let matcher = MockMatcher::new(registry);

    let body_bytes = input.body.as_deref().map(str::as_bytes);

    let mock_match = matcher.find_match(
        &method,
        &input.path,
        input.query.as_deref(),
        &header_map,
        body_bytes,
    );

    let Some(mock_match) = mock_match else {
        return Ok(TestMatchResult {
            matched: false,
            mock_id: None,
            priority: None,
            captures: FxHashMap::default(),
            response: None,
        });
    };

    let mock_def = &mock_match.mock;
    let captures = mock_match.captures.clone();

    // Render response if requested
    let response = if input.render {
        use crate::engine::types::ResponseGeneratorExt;

        let dynamic = mock_def
            .response
            .generate_dynamic(
                method.as_str(),
                &input.path,
                input.query.as_deref(),
                &header_map,
                body_bytes,
                mock_match.captures,
                mock_def.vars.as_ref(),
            )
            .await?;

        let status = dynamic.status.unwrap_or(mock_def.response.status);
        let mut headers = mock_def.response.headers.clone();
        if let Some(dyn_headers) = dynamic.headers {
            headers.extend(dyn_headers);
        }

        let body_str = String::from_utf8(dynamic.body.to_vec())
            .unwrap_or_else(|_| "<binary>".to_string());

        let body_base64 = if body_str == "<binary>" {
            Some(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &dynamic.body,
            ))
        } else {
            None
        };

        Some(RenderedResponse {
            status: status.as_u16(),
            headers,
            body: body_str,
            body_base64,
        })
    } else {
        None
    };

    Ok(TestMatchResult {
        matched: true,
        mock_id: Some(mock_def.id.to_string()),
        priority: Some(mock_def.priority),
        captures,
        response,
    })
}
