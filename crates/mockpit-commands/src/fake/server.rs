//! Fake data HTTP server

use crate::ui;
use base64::Engine;
use mockpit_types::RequestContext;

use super::data::generate_single_value;

/// Start a fake data HTTP server
#[allow(clippy::disallowed_types)] // axum Query extractor requires std HashMap
#[allow(clippy::items_after_statements)] // Inner functions/structs are the standard axum handler pattern
pub async fn serve_fake_data(
    port: u16,
    host: &str,
    cors: bool,
    open_browser: bool,
    verbose: bool,
) -> anyhow::Result<()> {
    use axum::{
        Router,
        extract::{Path, Query},
        http::{HeaderMap, HeaderValue, StatusCode, header},
        response::{Html, IntoResponse, Response},
        routing::{get, post},
    };
    use std::collections::HashMap;

    println!("{}", ui::header("Fake Data Server"));
    println!();
    println!("{}", ui::kv("Address", &format!("http://{host}:{port}")));
    println!();

    println!("{}", ui::emphasis("Endpoints:"));
    println!(
        "{}",
        ui::list_item("GET  /                     - API documentation")
    );
    println!(
        "{}",
        ui::list_item("GET  /fake/:type           - Generate fake data")
    );
    println!(
        "{}",
        ui::list_item("GET  /fake/image/:type     - Generate image")
    );
    println!(
        "{}",
        ui::list_item("GET  /fake/pdf             - Generate PDF")
    );
    println!(
        "{}",
        ui::list_item("POST /render               - Render template")
    );
    println!();

    // Index page handler
    async fn index_handler() -> Html<String> {
        Html(INDEX_HTML.to_string())
    }

    // Fake data handler
    async fn fake_data_handler(
        Path(generator): Path<String>,
        Query(params): Query<HashMap<String, String>>,
    ) -> Response {
        let count: usize = params
            .get("count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let min: Option<f64> = params.get("min").and_then(|v| v.parse().ok());
        let max: Option<f64> = params.get("max").and_then(|v| v.parse().ok());
        let words: Option<usize> = params.get("words").and_then(|v| v.parse().ok());
        let length: Option<usize> = params.get("length").and_then(|v| v.parse().ok());

        let mut results = Vec::new();
        for _ in 0..count {
            match generate_single_value(&generator, min, max, words, length) {
                Ok(v) => results.push(v),
                Err(e) => {
                    return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
                }
            }
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );

        let json_values: Vec<serde_json::Value> = results
            .iter()
            .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::String(s.clone())))
            .collect();
        let body = serde_json::to_string(&json_values).unwrap_or_default();

        (headers, body).into_response()
    }

    // Image handler
    async fn fake_image_handler(
        Path(image_type): Path<String>,
        Query(params): Query<HashMap<String, String>>,
    ) -> Response {
        use mockpit_fake_data::*;

        let width: u32 = params
            .get("width")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let height: u32 = params
            .get("height")
            .and_then(|v| v.parse().ok())
            .unwrap_or(200);
        let bg_color = params
            .get("bg_color")
            .or(params.get("bg"))
            .map(String::as_str);
        let text_color = params
            .get("text_color")
            .or(params.get("color"))
            .map(String::as_str);
        let text = params.get("text").map(String::as_str);
        let initials = params.get("initials").map(String::as_str);

        let base64_data = match image_type.as_str() {
            "placeholder" => {
                let display_text = text.map_or_else(|| format!("{width}x{height}"), String::from);
                fake_placeholder(
                    Some(width),
                    Some(height),
                    Some(&display_text),
                    bg_color,
                    text_color,
                )
            }
            "avatar" => fake_avatar(initials, Some(width), bg_color, text_color),
            "gradient" => {
                fake_image_gradient(Some(width), Some(height), bg_color, text_color, None)
            }
            "checkerboard" => {
                fake_image_checkerboard(Some(width), Some(height), bg_color, text_color, Some(20))
            }
            "noise" => fake_image_noise(Some(width), Some(height), Some(false)),
            _ => fake_placeholder(
                Some(width),
                Some(height),
                Some(&format!("{width}x{height}")),
                bg_color,
                text_color,
            ),
        };

        let bytes = match base64::engine::general_purpose::STANDARD.decode(&base64_data) {
            Ok(b) => b,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };

        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
        (headers, bytes).into_response()
    }

    // PDF handler
    async fn fake_pdf_handler(Query(params): Query<HashMap<String, String>>) -> Response {
        let pages: u32 = params
            .get("pages")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let text = params.get("text").map(String::as_str);

        let base64_data = mockpit_fake_data::fake_pdf(text, Some(pages));
        let bytes = match base64::engine::general_purpose::STANDARD.decode(&base64_data) {
            Ok(b) => b,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/pdf"),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("inline; filename=\"document.pdf\""),
        );
        (headers, bytes).into_response()
    }

    // Template render handler
    #[derive(serde::Deserialize)]
    struct RenderRequest {
        template: String,
        #[serde(default)]
        context: serde_json::Value,
    }

    async fn render_handler(axum::Json(req): axum::Json<RenderRequest>) -> Response {
        // Build RequestContext from the provided context
        let mut req_ctx = RequestContext::new();
        if !req.context.is_null() {
            if let Some(captures) = req.context.get("captures").and_then(|v| v.as_object()) {
                for (k, v) in captures {
                    if let Some(s) = v.as_str() {
                        req_ctx.captures.insert(k.clone(), s.to_string());
                    }
                }
            }
            if let Some(headers) = req.context.get("headers").and_then(|v| v.as_object()) {
                for (k, v) in headers {
                    if let Some(s) = v.as_str() {
                        req_ctx.headers.insert(k.clone(), s.to_string());
                    }
                }
            }
            if let Some(query) = req.context.get("query").and_then(|v| v.as_object()) {
                for (k, v) in query {
                    if let Some(s) = v.as_str() {
                        req_ctx.query.insert(k.clone(), s.to_string());
                    }
                }
            }
            if let Some(body) = req.context.get("body") {
                req_ctx.body = Some(body.to_string());
                req_ctx.body_json = Some(body.clone());
            }
        }

        match mockpit_template::render_template(&req.template, &req_ctx) {
            Ok(result) => {
                let mut headers = HeaderMap::new();
                // Check if result is JSON
                if result.trim().starts_with('{') || result.trim().starts_with('[') {
                    headers.insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    );
                } else {
                    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
                }
                (headers, result).into_response()
            }
            Err(e) => (StatusCode::BAD_REQUEST, format!("Template error: {e}")).into_response(),
        }
    }

    // Build router
    let mut app = Router::new()
        .route("/", get(index_handler))
        .route("/fake/{generator}", get(fake_data_handler))
        .route("/fake/image/{image_type}", get(fake_image_handler))
        .route("/fake/pdf", get(fake_pdf_handler))
        .route("/render", post(render_handler));

    // Add CORS if enabled
    if cors {
        use tower_http::cors::{Any, CorsLayer};
        app = app.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );
        println!("{}", ui::info("CORS enabled"));
    }

    // Add logging if verbose
    if verbose {
        use tower_http::trace::TraceLayer;
        app = app.layer(TraceLayer::new_for_http());
        println!("{}", ui::info("Verbose logging enabled"));
    }

    println!();
    println!("{}", ui::success("Server starting..."));

    // Open browser if requested
    if open_browser {
        let url = format!("http://{host}:{port}");
        let _ = open::that(&url);
    }

    // Start server
    let addr: std::net::SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid address: {e}"))?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .await
        .map_err(|e| anyhow::anyhow!("Server error: {e}"))?;

    Ok(())
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <title>Fake Data Server</title>
  <style>
    body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; max-width: 800px; margin: 0 auto; padding: 2rem; }
    h1 { color: #333; }
    h2 { color: #666; margin-top: 2rem; }
    code { background: #f4f4f4; padding: 0.2rem 0.4rem; border-radius: 3px; }
    pre { background: #f4f4f4; padding: 1rem; border-radius: 5px; overflow-x: auto; }
    .endpoint { margin: 1rem 0; padding: 1rem; border: 1px solid #ddd; border-radius: 5px; }
    .method { font-weight: bold; color: #0066cc; }
    a { color: #0066cc; }
  </style>
</head>
<body>
  <h1>Fake Data Server</h1>
  <p>Generate fake data on-demand via HTTP.</p>

  <h2>Endpoints</h2>

  <div class="endpoint">
    <p><span class="method">GET</span> <code>/fake/:type</code></p>
    <p>Generate fake data. Examples:</p>
    <ul>
      <li><a href="/fake/name">/fake/name</a> - Random name</li>
      <li><a href="/fake/email">/fake/email</a> - Random email</li>
      <li><a href="/fake/uuid">/fake/uuid</a> - Random UUID</li>
      <li><a href="/fake/user">/fake/user</a> - Random user object</li>
      <li><a href="/fake/number?min=1&max=100">/fake/number?min=1&max=100</a> - Random number</li>
    </ul>
  </div>

  <div class="endpoint">
    <p><span class="method">GET</span> <code>/fake/image/:type</code></p>
    <p>Generate images. Examples:</p>
    <ul>
      <li><a href="/fake/image/placeholder?width=400&height=300">/fake/image/placeholder</a> - Placeholder image</li>
      <li><a href="/fake/image/avatar?initials=JS">/fake/image/avatar?initials=JS</a> - Avatar with initials</li>
      <li><a href="/fake/image/gradient">/fake/image/gradient</a> - Gradient image</li>
    </ul>
  </div>

  <div class="endpoint">
    <p><span class="method">GET</span> <code>/fake/pdf</code></p>
    <p>Generate PDF document.</p>
    <ul>
      <li><a href="/fake/pdf">/fake/pdf</a> - Single page PDF</li>
      <li><a href="/fake/pdf?pages=5">/fake/pdf?pages=5</a> - Multi-page PDF</li>
    </ul>
  </div>

  <div class="endpoint">
    <p><span class="method">POST</span> <code>/render</code></p>
    <p>Render a template with fake data.</p>
    <pre>curl -X POST http://localhost:PORT/render \
  -H "Content-Type: application/json" \
  -d '{"template": "{\"name\": \"{{ fake_name() }}\"}"}'</pre>
  </div>

  <h2>Available Generators</h2>
  <p>identity: name, first_name, last_name, username, password, title, suffix</p>
  <p>contact: email, free_email, phone, cell_phone</p>
  <p>company: company, job_title, industry</p>
  <p>internet: url, domain, ipv4, ipv6, mac_address, user_agent</p>
  <p>finance: credit_card, currency_code, price, amount</p>
  <p>datetime: date, time, iso_date, unix_timestamp</p>
  <p>location: city, state, country, latitude, longitude</p>
  <p>text: word, words, sentence, paragraph, slug</p>
  <p>identifiers: uuid, token, sha256, md5, jwt</p>
  <p>composite: user, address</p>
</body>
</html>"#;
