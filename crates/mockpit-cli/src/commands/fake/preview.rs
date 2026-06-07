//! Template preview

use crate::commands::ui;
use mockpit::types::RequestContext;

/// Preview template rendering with fake data
pub async fn preview_template(
    template: Option<&str>,
    file: Option<&str>,
    context: Option<&str>,
    count: usize,
    format: &str,
) -> anyhow::Result<()> {
    // Get template content
    let template_content = if let Some(t) = template {
        t.to_string()
    } else if let Some(f) = file {
        tokio::fs::read_to_string(f).await?
    } else {
        anyhow::bail!("Provide a template string or --file path");
    };

    // Parse context into RequestContext
    let ctx = parse_context(context)?;

    // Render template(s)
    let mut results = Vec::new();
    for _ in 0..count {
        let result = mockpit::template::render_template(&template_content, &ctx)
            .map_err(|e| anyhow::anyhow!("Template error: {e}"))?;
        results.push(result);
    }

    // Output
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for (i, result) in results.iter().enumerate() {
            if count > 1 {
                crate::say!("{}", ui::dim(&format!("--- Render {} ---", i + 1)));
            }
            // Try to pretty-print JSON
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(result) {
                println!("{}", serde_json::to_string_pretty(&parsed)?);
            } else {
                println!("{result}");
            }
            if count > 1 {
                crate::say!();
            }
        }
    }

    Ok(())
}

/// Parse user-provided context JSON into RequestContext
pub fn parse_context(context: Option<&str>) -> anyhow::Result<RequestContext> {
    let ctx = if let Some(c) = context {
        // Parse user-provided context and merge into RequestContext
        let parsed: serde_json::Value = serde_json::from_str(c)?;
        let mut req_ctx = RequestContext::new();
        // If user provides captures, headers, etc. we can merge them
        if let Some(captures) = parsed.get("captures").and_then(|v| v.as_object()) {
            for (k, v) in captures {
                if let Some(s) = v.as_str() {
                    req_ctx.captures.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(headers) = parsed.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in headers {
                if let Some(s) = v.as_str() {
                    req_ctx.headers.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(query) = parsed.get("query").and_then(|v| v.as_object()) {
            for (k, v) in query {
                if let Some(s) = v.as_str() {
                    req_ctx.query.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(body) = parsed.get("body") {
            req_ctx.body = Some(body.to_string());
            req_ctx.body_json = Some(body.clone());
        }
        req_ctx
    } else {
        RequestContext::new()
    };
    Ok(ctx)
}
