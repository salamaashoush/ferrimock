//! Template rendering service.

use crate::types::RequestContext;

/// Input for template rendering.
#[derive(Debug, Clone)]
pub struct TemplateInput {
    /// Template string (Tera syntax)
    pub template: String,
    /// Optional JSON context for template variables
    pub context: Option<serde_json::Value>,
    /// Number of times to render (each render generates fresh fake data)
    pub count: usize,
}

/// Render a template with optional context.
#[allow(clippy::needless_pass_by_value)] // owned input is the service API boundary
pub fn render(input: TemplateInput) -> Result<Vec<String>, crate::MockpitError> {
    let ctx = build_context(input.context.as_ref());

    let mut results = Vec::with_capacity(input.count.max(1));
    for _ in 0..input.count.max(1) {
        let rendered = crate::template::render_template(&input.template, &ctx)
            .map_err(|e| crate::mp_err!("Template rendering failed: {e}"))?;
        results.push(rendered);
    }

    Ok(results)
}

/// Build a RequestContext from a JSON value.
fn build_context(context: Option<&serde_json::Value>) -> RequestContext {
    let mut ctx = RequestContext::new();

    let Some(context) = context else {
        return ctx;
    };

    if let Some(captures) = context.get("captures").and_then(|v| v.as_object()) {
        for (k, v) in captures {
            if let Some(s) = v.as_str() {
                ctx.captures.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(headers) = context.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            if let Some(s) = v.as_str() {
                ctx.headers.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(query) = context.get("query").and_then(|v| v.as_object()) {
        for (k, v) in query {
            if let Some(s) = v.as_str() {
                ctx.query.insert(k.clone(), s.to_string());
            }
        }
    }
    if let Some(body) = context.get("body") {
        ctx.body = Some(body.to_string());
        ctx.body_json = Some(body.clone());
    }

    ctx
}
