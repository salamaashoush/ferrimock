//! Test mock matching
//!
//! Enhanced mock testing with header/body support and response rendering.

use super::ui;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method};
use base64::Engine;
use colored::Colorize;
use ferrimock::engine::{MockMatcher, MockRegistry, ResponseGeneratorExt};
use serde_json::json;

// Icons for debug output
const CHECK_ICON: &str = "[+]";
const CROSS_ICON: &str = "[-]";

/// Determine if a content-type represents binary data
fn is_binary_content_type(content_type: &str) -> bool {
    let content_type_lower = content_type.to_lowercase();

    // Binary types
    if content_type_lower.starts_with("image/")
        || content_type_lower.starts_with("video/")
        || content_type_lower.starts_with("audio/")
        || content_type_lower.starts_with("application/pdf")
        || content_type_lower.starts_with("application/octet-stream")
        || content_type_lower.starts_with("application/zip")
        || content_type_lower.starts_with("application/gzip")
        || content_type_lower.starts_with("font/")
    {
        return true;
    }

    // Text types
    if content_type_lower.starts_with("text/")
        || content_type_lower.starts_with("application/json")
        || content_type_lower.starts_with("application/xml")
        || content_type_lower.starts_with("application/javascript")
        || content_type_lower.contains("charset")
    {
        return false;
    }

    // Default to false for unknown types
    false
}

/// Parameters for mock match testing
pub struct TestMockParams {
    pub method_str: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<String>,
    pub body: Option<String>,
    pub render: bool,
    pub debug: bool,
    pub mock_file: Option<String>,
    pub json: bool,
}

/// Test mock matching with enhanced options
#[allow(clippy::large_futures)]
pub async fn test_mock_match(params: TestMockParams) -> anyhow::Result<()> {
    let TestMockParams {
        ref method_str,
        ref path,
        ref query,
        ref headers,
        ref body,
        render,
        debug,
        ref mock_file,
        json,
    } = params;

    // Smart URL parsing: extract query from path if not provided explicitly
    let (clean_path, final_query) = if query.is_none() && path.contains('?') {
        if let Some((before, after)) = path.split_once('?') {
            (before.to_string(), Some(after.to_string()))
        } else {
            (path.clone(), query.clone())
        }
    } else {
        (path.clone(), query.clone())
    };

    // JSON output path - early return with clean JSON
    if json {
        return test_mock_match_json(
            method_str,
            &clean_path,
            &final_query,
            headers,
            body,
            render,
            mock_file,
        )
        .await;
    }

    // Human-readable UI output path
    crate::say!("{}", ui::action("Testing mock match"));
    crate::say!("{}", ui::kv("Method", method_str));
    crate::say!("{}", ui::kv("Path", &clean_path));
    if let Some(q) = &final_query {
        crate::say!("{}", ui::kv("Query", q));
    }

    // Parse headers
    let mut header_map = HeaderMap::new();
    if !headers.is_empty() {
        crate::say!();
        crate::say!("{}", ui::header("Request Headers"));
        for header_str in headers {
            if let Some((name, value)) = header_str.split_once(':') {
                let name = name.trim();
                let value = value.trim();
                crate::say!("{}", ui::kv(name, value));

                if let (Ok(header_name), Ok(header_value)) =
                    (HeaderName::try_from(name), HeaderValue::from_str(value))
                {
                    header_map.insert(header_name, header_value);
                }
            } else {
                println!(
                    "{}",
                    ui::warning(&format!(
                        "Invalid header format: {header_str} (expected \"Name: Value\")"
                    ))
                );
            }
        }
    }

    // Parse body
    let body_bytes: Option<Vec<u8>> = match &body {
        Some(b) if b.starts_with('@') => {
            // Load from file
            #[allow(clippy::string_slice)]
            let file_path = &b[1..];
            match std::fs::read(file_path) {
                Ok(content) => {
                    crate::say!();
                    println!(
                        "{}",
                        ui::kv("Body", &format!("(from file: {})", ui::path(file_path)))
                    );
                    Some(content)
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Failed to read body file '{file_path}': {e}"
                    ));
                }
            }
        }
        Some(b) => {
            crate::say!();
            if b.len() > 100 {
                #[allow(clippy::string_slice)]
                let truncated = &b[..100];
                crate::say!("{}", ui::kv("Body", &format!("{truncated}...")));
            } else {
                crate::say!("{}", ui::kv("Body", b));
            }
            Some(b.as_bytes().to_vec())
        }
        None => None,
    };

    crate::say!();

    let registry = MockRegistry::new();
    let count = if let Some(file) = mock_file {
        let file_path = std::path::Path::new(file);
        if !file_path.exists() {
            anyhow::bail!("Mock file not found: {file}");
        }
        let spinner = ui::spinner(&format!("Loading mocks from {}...", ui::path(file)));
        let count = registry
            .load_collection_file(file_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load mocks from file: {e}"))?;
        spinner.finish_and_clear();
        count
    } else {
        let collections_dir = crate::config::mocks_dir();
        let spinner = ui::spinner(&format!(
            "Loading mocks from {}...",
            ui::path(&collections_dir)
        ));
        let count = registry
            .load_from_directory(&collections_dir)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load mocks: {e}"))?;
        spinner.finish_and_clear();
        count
    };

    println!(
        "{}",
        ui::success(&format!("Loaded {} mock definition(s)", ui::number(count)))
    );
    crate::say!();

    let method = method_str
        .to_uppercase()
        .parse::<Method>()
        .map_err(|_| anyhow::anyhow!("Invalid HTTP method: {method_str}"))?;

    let matcher = MockMatcher::new(registry.clone());

    // Debug mode: show matching details for all mocks
    if debug {
        show_debug_matching(
            &registry,
            &method,
            &clean_path,
            final_query.as_deref(),
            &header_map,
            body_bytes.as_deref(),
        );
        crate::say!();
    }

    // Find the match
    if let Some(mock_match) = matcher.find_match(
        &method,
        &clean_path,
        final_query.as_deref(),
        &header_map,
        body_bytes.as_deref(),
    ) {
        crate::say!("{}", ui::success("Match found!"));
        crate::say!();
        crate::say!("{}", ui::kv("Mock ID", &mock_match.mock.id));
        println!(
            "{}",
            ui::kv("Priority", &ui::number(mock_match.mock.priority))
        );
        println!(
            "{}",
            ui::kv(
                "Response Status",
                &ui::number(mock_match.mock.response.status.as_u16())
            )
        );
        println!(
            "{}",
            ui::kv(
                "Response Mode",
                &format!("{:?}", mock_match.mock.response.mode)
            )
        );

        // Show source file if available
        if let Some(ref source_file) = mock_match.mock.source_file {
            crate::say!("{}", ui::kv("Source File", source_file));
        }

        // Print captures if any
        if !mock_match.captures.is_empty() {
            crate::say!();
            crate::say!("{}", ui::header("URL Captures"));
            for (key, value) in &mock_match.captures {
                crate::say!("{}", ui::kv(key, value));
            }
        }

        // Render the response if requested
        if render {
            crate::say!();
            crate::say!("{}", ui::header("Rendered Response"));

            let response_generator = &mock_match.mock.response;

            // Show response headers
            if !response_generator.headers.is_empty() {
                crate::say!();
                crate::say!("{}", ui::emphasis("Headers:"));
                for (key, value) in &response_generator.headers {
                    println!("  {key}: {value}");
                }
            }

            // Render body
            crate::say!();
            crate::say!("{}", ui::emphasis("Body:"));

            match response_generator
                .generate_dynamic(
                    method.as_str(),
                    &clean_path,
                    final_query.as_deref(),
                    &header_map,
                    body_bytes.as_deref(),
                    mock_match.captures.clone(),
                    mock_match.mock.vars.as_ref(),
                )
                .await
            {
                Ok(dynamic_response) => {
                    // Show status override if any
                    if let Some(status) = dynamic_response.status {
                        println!(
                            "{}",
                            ui::kv("Status Override", &ui::number(status.as_u16()))
                        );
                    }

                    // Show header overrides if any
                    if let Some(ref headers) = dynamic_response.headers
                        && !headers.is_empty()
                    {
                        crate::say!("{}", ui::emphasis("Header Overrides:"));
                        for (key, value) in headers {
                            println!("  {key}: {value}");
                        }
                    }

                    // Show body
                    let body_str = String::from_utf8_lossy(&dynamic_response.body);

                    // Try to pretty-print JSON
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_str) {
                        if let Ok(pretty) = serde_json::to_string_pretty(&json) {
                            println!("{pretty}");
                        } else {
                            println!("{body_str}");
                        }
                    } else {
                        // Not JSON, print as-is (truncate if too long)
                        if body_str.len() > 2000 {
                            #[allow(clippy::string_slice)]
                            let truncated = &body_str[..2000];
                            println!("{truncated}...");
                            println!(
                                "{}",
                                ui::dim(&format!("(truncated, {} total bytes)", body_str.len()))
                            );
                        } else {
                            println!("{body_str}");
                        }
                    }
                }
                Err(e) => {
                    crate::say!("{}", ui::error(&format!("Failed to render response: {e}")));
                }
            }
        }
    } else {
        crate::say!("{}", ui::warning("No matching mock found for this request"));

        if !debug {
            crate::say!();
            println!(
                "{}",
                ui::dim("Tip: Use --debug to see why each mock didn't match")
            );
        }
    }

    Ok(())
}

/// JSON output path - clean and separate from UI code
#[allow(clippy::ref_option, clippy::large_futures)]
async fn test_mock_match_json(
    method_str: &str,
    path: &str,
    query: &Option<String>,
    headers: &[String],
    body: &Option<String>,
    render: bool,
    mock_file: &Option<String>,
) -> anyhow::Result<()> {
    let method = method_str
        .to_uppercase()
        .parse::<Method>()
        .map_err(|_| anyhow::anyhow!("Invalid HTTP method: {method_str}"))?;

    // Parse headers
    let mut header_map = HeaderMap::new();
    for header_str in headers {
        if let Some((name, value)) = header_str.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            if let (Ok(header_name), Ok(header_value)) =
                (HeaderName::try_from(name), HeaderValue::from_str(value))
            {
                header_map.insert(header_name, header_value);
            }
        }
    }

    // Parse body
    let body_bytes: Option<Vec<u8>> = match body {
        Some(b) if b.starts_with('@') => {
            #[allow(clippy::string_slice)]
            let file_path = &b[1..];
            match std::fs::read(file_path) {
                Ok(content) => Some(content),
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "Failed to read body file '{file_path}': {e}"
                    ));
                }
            }
        }
        Some(b) => Some(b.as_bytes().to_vec()),
        None => None,
    };

    // Load mocks
    let registry = MockRegistry::new();
    if let Some(file) = mock_file {
        let file_path = std::path::Path::new(file);
        if !file_path.exists() {
            anyhow::bail!("Mock file not found: {file}");
        }
        registry
            .load_collection_file(file_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load mocks from file: {e}"))?;
    } else {
        let collections_dir = crate::config::mocks_dir();
        registry
            .load_from_directory(&collections_dir)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load mocks: {e}"))?;
    }

    let matcher = MockMatcher::new(registry.clone());

    // Find match
    let mock_result = matcher.find_match(
        &method,
        path,
        query.as_deref(),
        &header_map,
        body_bytes.as_deref(),
    );

    // Build JSON output
    let captures: serde_json::Value = if let Some(ref m) = mock_result {
        serde_json::to_value(&m.captures).unwrap_or(json!({}))
    } else {
        json!({})
    };

    let mut output = json!({
      "matched": mock_result.is_some(),
      "mock_id": mock_result.as_ref().map(|m| &m.mock.id),
      "priority": mock_result.as_ref().map(|m| m.mock.priority),
      "response_status": mock_result.as_ref().map(|m| m.mock.response.status.as_u16()),
      "captures": captures,
    });

    // Render response if requested
    if render && let Some(mock_match) = &mock_result {
        let response_generator = &mock_match.mock.response;

        match response_generator
            .generate_dynamic(
                method.as_str(),
                path,
                query.as_deref(),
                &header_map,
                body_bytes.as_deref(),
                mock_match.captures.clone(),
                mock_match.mock.vars.as_ref(),
            )
            .await
        {
            Ok(dynamic_response) => {
                // Detect content type from headers
                let headers = dynamic_response
                    .headers
                    .clone()
                    .unwrap_or_else(|| response_generator.headers.clone());

                let content_type = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map_or("text/plain", |(_, v)| v.as_str());

                // Determine if content is binary based on content-type
                let is_binary = is_binary_content_type(content_type);

                let (body_value, is_base64) = if is_binary {
                    // Binary content: base64 encode
                    (
                        json!(
                            base64::engine::general_purpose::STANDARD
                                .encode(&dynamic_response.body)
                        ),
                        true,
                    )
                } else {
                    // Text content: try to parse as JSON, otherwise use string
                    let body_str = String::from_utf8_lossy(&dynamic_response.body);
                    let body_val = serde_json::from_str::<serde_json::Value>(&body_str)
                        .unwrap_or_else(|_| serde_json::Value::String(body_str.to_string()));
                    (body_val, false)
                };

                #[allow(clippy::indexing_slicing)]
                {
                    output["rendered"] = json!({
                      "status": dynamic_response.status.map(|s| s.as_u16()).or(Some(mock_match.mock.response.status.as_u16())),
                      "headers": headers,
                      "body": body_value,
                      "body_base64": is_base64,
                    });
                }
            }
            Err(e) => {
                #[allow(clippy::indexing_slicing)]
                {
                    output["render_error"] = json!(e.to_string());
                }
            }
        }
    }

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Show detailed matching debug info for all mocks
fn show_debug_matching(
    registry: &MockRegistry,
    method: &Method,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    body: Option<&[u8]>,
) {
    crate::say!("{}", ui::header("Debug: Mock Matching Analysis"));
    crate::say!();

    let all_mocks = registry.get_enabled_mocks();

    if all_mocks.is_empty() {
        crate::say!("{}", ui::warning("No mocks loaded"));
        return;
    }

    for mock in all_mocks {
        println!(
            "{} {} (priority: {})",
            ui::emphasis("Mock:"),
            ui::code(&mock.id),
            ui::number(mock.priority)
        );

        let mut all_matched = true;
        let mut match_details = Vec::new();

        // Check method
        let method_match = if mock.request.methods.is_empty() {
            true
        } else {
            mock.request.methods.contains(method)
        };

        if method_match {
            match_details.push(format!(
                "  {} Method: {} {}",
                CHECK_ICON.green(),
                method,
                if mock.request.methods.is_empty() {
                    "(any method)".to_string()
                } else {
                    format!(
                        "matches {:?}",
                        mock.request
                            .methods
                            .iter()
                            .map(Method::as_str)
                            .collect::<Vec<_>>()
                    )
                }
            ));
        } else {
            all_matched = false;
            match_details.push(format!(
                "  {} Method: {} does not match {:?}",
                CROSS_ICON.red(),
                method,
                mock.request
                    .methods
                    .iter()
                    .map(Method::as_str)
                    .collect::<Vec<_>>()
            ));
        }

        // Check URL patterns
        let url_match = if mock.request.url_patterns.is_empty() {
            true
        } else {
            let full_url = if let Some(q) = query {
                format!("{path}?{q}")
            } else {
                path.to_string()
            };

            mock.request
                .url_patterns
                .iter()
                .any(|pattern| pattern.matches(&full_url) || pattern.matches(path))
        };

        if url_match {
            match_details.push(format!(
                "  {} URL: {} matches pattern(s)",
                CHECK_ICON.green(),
                path
            ));
        } else {
            all_matched = false;
            let pattern_count = mock.request.url_patterns.len();
            match_details.push(format!(
                "  {} URL: {} does not match {} pattern(s)",
                CROSS_ICON.red(),
                path,
                pattern_count
            ));
        }

        // Check header matchers
        if !mock.request.header_matchers.is_empty() {
            let header_match = mock
                .request
                .header_matchers
                .iter()
                .all(|matcher| matcher.matches(headers));

            if header_match {
                match_details.push(format!(
                    "  {} Headers: all {} matcher(s) passed",
                    CHECK_ICON.green(),
                    mock.request.header_matchers.len()
                ));
            } else {
                all_matched = false;
                for matcher in &mock.request.header_matchers {
                    if !matcher.matches(headers) {
                        match_details.push(format!(
                            "  {} Header matcher failed: {:?}",
                            CROSS_ICON.red(),
                            matcher
                        ));
                    }
                }
            }
        }

        // Check query matchers
        if !mock.request.query_matchers.is_empty() {
            let query_match = mock
                .request
                .query_matchers
                .iter()
                .all(|matcher| matcher.matches(query));

            if query_match {
                match_details.push(format!(
                    "  {} Query: all {} matcher(s) passed",
                    CHECK_ICON.green(),
                    mock.request.query_matchers.len()
                ));
            } else {
                all_matched = false;
                match_details.push(format!("  {} Query matcher failed", CROSS_ICON.red()));
            }
        }

        // Check body matcher
        if let Some(ref body_matcher) = mock.request.body_matcher {
            let body_match = body.is_some_and(|b| body_matcher.matches(b, None));

            if body_match {
                match_details.push(format!("  {} Body: matcher passed", CHECK_ICON.green()));
            } else {
                all_matched = false;
                if body.is_none() {
                    match_details.push(format!(
                        "  {} Body: no body provided but matcher required",
                        CROSS_ICON.red()
                    ));
                } else {
                    match_details.push(format!("  {} Body: matcher failed", CROSS_ICON.red()));
                }
            }
        }

        // Check GraphQL matcher
        if let Some(ref gql_matcher) = mock.request.graphql_matcher {
            let gql_match = body.is_some_and(|b| {
                serde_json::from_slice::<serde_json::Value>(b).is_ok_and(|json| {
                    // Simplified GraphQL matching check
                    if gql_matcher.match_any {
                        json.get("query").is_some() || json.get("operationName").is_some()
                    } else if let Some(ref op_name) = gql_matcher.operation_name {
                        json.get("operationName")
                            .and_then(|v| v.as_str())
                            .is_some_and(|name| name == op_name)
                    } else {
                        true
                    }
                })
            });

            if gql_match {
                match_details.push(format!("  {} GraphQL: matcher passed", CHECK_ICON.green()));
            } else {
                all_matched = false;
                if let Some(ref op_name) = gql_matcher.operation_name {
                    match_details.push(format!(
                        "  {} GraphQL: operation '{}' not matched",
                        CROSS_ICON.red(),
                        op_name
                    ));
                } else {
                    match_details.push(format!("  {} GraphQL: matcher failed", CROSS_ICON.red()));
                }
            }
        }

        // Print result
        if all_matched {
            println!(
                "  {} {}",
                ui::success("MATCH"),
                ui::dim("(would be selected based on priority)")
            );
        } else {
            println!("  {}", ui::error("NO MATCH"));
        }

        for detail in match_details {
            println!("{detail}");
        }

        crate::say!();
    }
}
