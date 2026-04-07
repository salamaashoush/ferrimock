//! Template rendering API

use mockpit_types::RequestContext;
use rustc_hash::FxHashMap;
use tera::Context;

use super::engine::{TEMPLATE_ENGINE, VALIDATION_ENGINE};
use super::error::TemplateError;

/// Build a Tera context from a RequestContext, skipping empty/None fields.
fn build_tera_context(context: &RequestContext) -> Context {
    let mut tera_context = Context::new();
    if !context.method.is_empty() {
        tera_context.insert("method", &context.method);
    }
    if !context.uri.is_empty() {
        tera_context.insert("uri", &context.uri);
    }
    if !context.path.is_empty() {
        tera_context.insert("path", &context.path);
    }
    if !context.captures.is_empty() {
        tera_context.insert("captures", &context.captures);
    }
    if !context.query.is_empty() {
        tera_context.insert("query", &context.query);
    }
    if !context.headers.is_empty() {
        tera_context.insert("headers", &context.headers);
    }
    if let Some(body) = &context.body {
        tera_context.insert("body", body);
    }
    if let Some(body_json) = &context.body_json {
        tera_context.insert("body_json", body_json);
    }
    if let Some(vars) = &context.vars {
        tera_context.insert("vars", vars);
    }
    tera_context
}

/// Render a template string with the given request context (uses thread-local engine)
pub fn render_template(template: &str, context: &RequestContext) -> Result<String, String> {
    render_template_with_id(template, context, None)
}

/// Render a template with an optional mock ID for better error messages
pub fn render_template_with_id(
    template: &str,
    context: &RequestContext,
    mock_id: Option<&str>,
) -> Result<String, String> {
    TEMPLATE_ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        let tera_context = build_tera_context(context);

        engine
            .render(template, &tera_context)
            .map_err(|e| match mock_id {
                Some(id) => format!("[Mock: {id}] {e}"),
                None => e,
            })
    })
}

/// Render a template with a pre-computed hash (skips hashing the template string)
pub fn render_template_with_hash(
    template: &str,
    hash: u64,
    context: &RequestContext,
    mock_id: Option<&str>,
) -> Result<String, String> {
    TEMPLATE_ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        let tera_context = build_tera_context(context);

        engine
            .render_with_hash(template, hash, &tera_context)
            .map_err(|e| match mock_id {
                Some(id) => format!("[Mock: {id}] {e}"),
                None => e,
            })
    })
}

/// Render a template string with patch context (request + upstream response data).
///
/// Used for dynamic patch values like `{{ captures.id }}` or `{{ response.body_json.name }}`.
/// In addition to all request context variables, provides:
/// - `response.status` - upstream response status code
/// - `response.headers` - upstream response headers
/// - `response.body_json` - upstream response body as JSON
pub fn render_patch_template(
    template: &str,
    context: &mockpit_types::PatchContext,
    mock_id: Option<&str>,
) -> Result<String, String> {
    TEMPLATE_ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        let mut tera_context = build_tera_context(&context.request);

        // Add upstream response context under "response" namespace
        let mut response_map = FxHashMap::default();
        response_map.insert("status", serde_json::json!(context.response_status));
        response_map.insert("headers", serde_json::json!(context.response_headers));
        if let Some(ref body_json) = context.response_body_json {
            response_map.insert("body_json", body_json.clone());
        }
        tera_context.insert("response", &response_map);

        engine
            .render(template, &tera_context)
            .map_err(|e| match mock_id {
                Some(id) => format!("[Mock: {id}] {e}"),
                None => e,
            })
    })
}

/// Validate a template without rendering it.
/// Uses a separate validation engine to avoid polluting the render cache.
/// This checks for syntax errors and returns detailed error information.
#[allow(clippy::result_large_err)]
pub fn validate_template(template: &str) -> Result<(), TemplateError> {
    VALIDATION_ENGINE.with(|engine| {
        let mut engine = engine.borrow_mut();
        engine.validate(template)
    })
}
