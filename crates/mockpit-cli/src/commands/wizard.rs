//! Interactive mock creation wizard
//!
//! Provides a step-by-step guided experience for creating mocks.

use super::ui;
use std::io::{self, Write};
use std::path::PathBuf;

use super::create::{generate_json_mock, generate_template_body, generate_yaml_mock};

/// Wizard state holding all configuration
#[derive(Debug, Clone)]
struct WizardState {
    // Request matching
    url_pattern: String,
    methods: Vec<String>,
    header_matchers: Vec<(String, String)>,
    query_matchers: Vec<(String, String)>,
    body_matcher: Option<(String, String)>,

    // Response configuration
    status: u16,
    content_type: String,

    // Response body
    body_source: BodySource,
    template_body: String,

    // Response behavior
    delay_ms: Option<u64>,

    // Metadata
    mock_id: String,
    priority: u32,
    collection: Option<String>,

    // Output
    output_path: PathBuf,
    format: String,
}

#[derive(Debug, Clone)]
enum BodySource {
    Template,
    Static,
    File(String),
    Empty,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            url_pattern: String::new(),
            methods: vec!["GET".to_string()],
            header_matchers: Vec::new(),
            query_matchers: Vec::new(),
            body_matcher: None,
            status: 200,
            content_type: "application/json".to_string(),
            body_source: BodySource::Template,
            template_body: String::new(),
            delay_ms: None,
            mock_id: String::new(),
            priority: 100,
            collection: None,
            output_path: PathBuf::new(),
            format: "yaml".to_string(),
        }
    }
}

/// Run the interactive mock creation wizard
#[allow(clippy::too_many_arguments)]
pub fn run_wizard(
    initial_url: Option<String>,
    output: Option<String>,
    initial_method: &str,
    initial_status: u16,
    initial_body: Option<String>,
    use_template: bool,
    initial_id: Option<String>,
    initial_priority: u32,
    initial_collection: Option<String>,
) -> anyhow::Result<()> {
    println!();
    println!("{}", ui::header("Mock Creation Wizard"));
    println!();
    println!(
        "{}",
        ui::dim("Create a new mock definition with step-by-step guidance.")
    );
    println!(
        "{}",
        ui::dim("Press Enter to accept defaults shown in [brackets].")
    );
    println!();

    // Initialize state with any provided defaults
    let mut state = WizardState {
        url_pattern: initial_url.unwrap_or_default(),
        methods: vec![initial_method.to_uppercase()],
        status: initial_status,
        body_source: if use_template {
            BodySource::Template
        } else if initial_body.is_some() {
            BodySource::Static
        } else {
            BodySource::Template
        },
        template_body: initial_body.unwrap_or_default(),
        mock_id: initial_id.unwrap_or_default(),
        priority: initial_priority,
        collection: initial_collection,
        ..Default::default()
    };

    // Step 1: Request Matching
    step_request_matching(&mut state)?;

    // Step 2: Response Configuration
    step_response_config(&mut state)?;

    // Step 3: Response Body
    step_response_body(&mut state)?;

    // Step 4: Response Behavior
    step_response_behavior(&mut state)?;

    // Step 5: Metadata
    step_metadata(&mut state, output)?;

    // Step 6: Review & Save
    step_review_and_save(&state)
}

fn step_request_matching(state: &mut WizardState) -> anyhow::Result<()> {
    ui::divider();
    println!("{}", ui::step(1, 6, "Request Matching"));
    ui::divider();
    println!();

    // URL Pattern
    let default_url = if state.url_pattern.is_empty() {
        "/api/resource/:id"
    } else {
        &state.url_pattern
    };
    print!(
        "{} [{}]: ",
        ui::emphasis("URL Pattern"),
        ui::dim(default_url)
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    state.url_pattern = if input.is_empty() {
        default_url.to_string()
    } else {
        input.to_string()
    };

    // Detect URL pattern type
    let pattern_type = detect_url_pattern_type(&state.url_pattern);
    println!(
        "  {}",
        ui::dim(&format!("Auto-detected: {pattern_type} pattern"))
    );
    println!();

    // HTTP Methods
    println!("{}", ui::emphasis("HTTP Method(s):"));
    println!(
        "  {}",
        ui::dim("Select methods (space-separated, e.g., 'GET POST')")
    );
    let methods_list = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD"];
    for (i, method) in methods_list.iter().enumerate() {
        let selected = state.methods.contains(&method.to_string());
        println!(
            "  {} {} {}",
            if selected { "[x]" } else { "[ ]" },
            i + 1,
            method
        );
    }
    let default_methods = state.methods.join(" ");
    print!(
        "{} [{}]: ",
        ui::emphasis("Methods"),
        ui::dim(&default_methods)
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_uppercase();

    if !input.is_empty() {
        state.methods = input.split_whitespace().map(ToString::to_string).collect();
    }
    println!();

    // Header Matchers
    print!("{} (y/N): ", ui::emphasis("Add header matchers?"));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim().eq_ignore_ascii_case("y") {
        loop {
            print!("  {} (or Enter to finish): ", ui::dim("Header name"));
            io::stdout().flush()?;
            let mut name = String::new();
            io::stdin().read_line(&mut name)?;
            let name = name.trim();
            if name.is_empty() {
                break;
            }

            print!("  {} (regex): ", ui::dim("Pattern"));
            io::stdout().flush()?;
            let mut pattern = String::new();
            io::stdin().read_line(&mut pattern)?;
            let pattern = pattern.trim();

            state
                .header_matchers
                .push((name.to_string(), pattern.to_string()));
            println!("  {}", ui::success(&format!("Added: {name} = {pattern}")));
        }
    }
    println!();

    // Query Parameter Matchers
    print!("{} (y/N): ", ui::emphasis("Add query parameter matchers?"));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim().eq_ignore_ascii_case("y") {
        loop {
            print!("  {} (or Enter to finish): ", ui::dim("Query param name"));
            io::stdout().flush()?;
            let mut name = String::new();
            io::stdin().read_line(&mut name)?;
            let name = name.trim();
            if name.is_empty() {
                break;
            }

            print!("  {} (regex): ", ui::dim("Pattern"));
            io::stdout().flush()?;
            let mut pattern = String::new();
            io::stdin().read_line(&mut pattern)?;
            let pattern = pattern.trim();

            state
                .query_matchers
                .push((name.to_string(), pattern.to_string()));
            println!("  {}", ui::success(&format!("Added: {name} = {pattern}")));
        }
    }
    println!();

    // Body Matcher (for POST/PUT/PATCH)
    if state
        .methods
        .iter()
        .any(|m| m == "POST" || m == "PUT" || m == "PATCH")
    {
        print!("{} (y/N): ", ui::emphasis("Add request body matcher?"));
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if input.trim().eq_ignore_ascii_case("y") {
            print!("  {} (e.g., $.email): ", ui::dim("JSONPath"));
            io::stdout().flush()?;
            let mut path = String::new();
            io::stdin().read_line(&mut path)?;
            let path = path.trim();

            print!("  {} (regex): ", ui::dim("Pattern"));
            io::stdout().flush()?;
            let mut pattern = String::new();
            io::stdin().read_line(&mut pattern)?;
            let pattern = pattern.trim();

            if !path.is_empty() {
                state.body_matcher = Some((path.to_string(), pattern.to_string()));
                println!("  {}", ui::success(&format!("Added: {path} = {pattern}")));
            }
        }
    }
    println!();

    Ok(())
}

fn step_response_config(state: &mut WizardState) -> anyhow::Result<()> {
    ui::divider();
    println!("{}", ui::step(2, 6, "Response Configuration"));
    ui::divider();
    println!();

    // Status Code
    let default_status = state.status.to_string();
    print!(
        "{} [{}]: ",
        ui::emphasis("Status Code"),
        ui::dim(&default_status)
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if !input.is_empty() {
        state.status = input.parse().unwrap_or(state.status);
    }
    println!();

    // Content-Type
    println!("{}", ui::emphasis("Content-Type:"));
    let content_types = [
        ("1", "application/json"),
        ("2", "text/plain"),
        ("3", "application/xml"),
        ("4", "text/html"),
        ("5", "application/octet-stream"),
    ];
    for (num, ct) in &content_types {
        let selected = state.content_type == *ct;
        println!("  {} {} {}", if selected { "[x]" } else { "[ ]" }, num, ct);
    }
    print!(
        "{} [{}]: ",
        ui::emphasis("Select or enter custom"),
        ui::dim("1")
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if !input.is_empty() {
        state.content_type = match input {
            "1" => "application/json".to_string(),
            "2" => "text/plain".to_string(),
            "3" => "application/xml".to_string(),
            "4" => "text/html".to_string(),
            "5" => "application/octet-stream".to_string(),
            custom => custom.to_string(),
        };
    }
    println!();

    Ok(())
}

fn step_response_body(state: &mut WizardState) -> anyhow::Result<()> {
    ui::divider();
    println!("{}", ui::step(3, 6, "Response Body"));
    ui::divider();
    println!();

    println!("{}", ui::emphasis("Body Source:"));
    println!("  [x] 1 Template with fake data (recommended)");
    println!("  [ ] 2 Static JSON/text");
    println!("  [ ] 3 File reference");
    println!("  [ ] 4 Empty");
    print!("{} [1]: ", ui::emphasis("Select"));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    state.body_source = match input {
        "2" => BodySource::Static,
        "3" => {
            print!("  {} ", ui::dim("File path:"));
            io::stdout().flush()?;
            let mut path = String::new();
            io::stdin().read_line(&mut path)?;
            BodySource::File(path.trim().to_string())
        }
        "4" => BodySource::Empty,
        _ => BodySource::Template,
    };
    println!();

    // Generate or get the body content
    match &state.body_source {
        BodySource::Template => {
            println!("{}", ui::emphasis("Template Type:"));
            println!("  [x] 1 Auto-generate based on endpoint");
            println!("  [ ] 2 User/profile response");
            println!("  [ ] 3 Paginated list response");
            println!("  [ ] 4 Single item response");
            println!("  [ ] 5 Create (POST) response");
            println!("  [ ] 6 Update (PUT/PATCH) response");
            println!("  [ ] 7 Delete response");
            println!("  [ ] 8 Error response");
            println!("  [ ] 9 Custom template");
            print!("{} [1]: ", ui::emphasis("Select"));
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            let template_choice = input.trim();

            state.template_body = match template_choice {
                "2" => generate_user_template(),
                "3" => generate_list_template(),
                "4" => generate_item_template(),
                "5" => generate_create_template(),
                "6" => generate_update_template(),
                "7" => generate_delete_template(),
                "8" => generate_error_template(state.status),
                "9" => {
                    println!();
                    println!(
                        "{}",
                        ui::dim("Enter template (press Ctrl+D or empty line to finish):")
                    );
                    let mut lines = Vec::new();
                    loop {
                        let mut line = String::new();
                        if io::stdin().read_line(&mut line).is_err() || line.is_empty() {
                            break;
                        }
                        if line.trim().is_empty() && !lines.is_empty() {
                            break;
                        }
                        lines.push(line);
                    }
                    lines.concat()
                }
                _ => {
                    // Auto-generate based on endpoint
                    let method = state.methods.first().map_or("GET", String::as_str);
                    generate_template_body(method, &state.url_pattern)
                }
            };

            // Show preview
            println!();
            ui::preview_box("Template Preview", &state.template_body);

            print!("{} (y/N): ", ui::emphasis("Edit template?"));
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            if input.trim().eq_ignore_ascii_case("y") {
                println!(
                    "{}",
                    ui::dim("Enter new template (press Ctrl+D or empty line to finish):")
                );
                let mut lines = Vec::new();
                loop {
                    let mut line = String::new();
                    if io::stdin().read_line(&mut line).is_err() || line.is_empty() {
                        break;
                    }
                    if line.trim().is_empty() && !lines.is_empty() {
                        break;
                    }
                    lines.push(line);
                }
                if !lines.is_empty() {
                    state.template_body = lines.concat();
                }
            }
        }
        BodySource::Static => {
            println!(
                "{}",
                ui::dim("Enter static body (press Enter twice to finish):")
            );
            let mut lines = Vec::new();
            loop {
                let mut line = String::new();
                io::stdin().read_line(&mut line)?;
                if line.trim().is_empty() && !lines.is_empty() {
                    break;
                }
                lines.push(line);
            }
            state.template_body = lines.concat().trim_end().to_string();
        }
        BodySource::File(path) => {
            state.template_body = format!("@{path}");
        }
        BodySource::Empty => {
            state.template_body = String::new();
        }
    }
    println!();

    Ok(())
}

fn step_response_behavior(state: &mut WizardState) -> anyhow::Result<()> {
    ui::divider();
    println!("{}", ui::step(4, 6, "Response Behavior"));
    ui::divider();
    println!();

    // Delay
    print!("{} (y/N): ", ui::emphasis("Add response delay?"));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim().eq_ignore_ascii_case("y") {
        print!("  {} [200]: ", ui::dim("Delay (ms)"));
        io::stdout().flush()?;

        let mut delay = String::new();
        io::stdin().read_line(&mut delay)?;
        let delay = delay.trim();

        let delay_val: u64 = delay.parse().unwrap_or(200);
        state.delay_ms = Some(delay_val);
        println!("  {}", ui::success(&format!("Set delay: {delay_val}ms")));
    }
    println!();

    Ok(())
}

fn step_metadata(state: &mut WizardState, output: Option<String>) -> anyhow::Result<()> {
    ui::divider();
    println!("{}", ui::step(5, 6, "Metadata"));
    ui::divider();
    println!();

    // Generate default mock ID if not set
    if state.mock_id.is_empty() {
        let method = state.methods.first().map_or("get", String::as_str);
        let url_part = state
            .url_pattern
            .trim_start_matches('^')
            .trim_end_matches('$')
            .trim_matches('/')
            .replace(['/', '.', ':'], "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>();
        state.mock_id = format!("{}-{}", method.to_lowercase(), url_part);
    }

    // Mock ID
    print!(
        "{} [{}]: ",
        ui::emphasis("Mock ID"),
        ui::dim(&state.mock_id)
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if !input.is_empty() {
        state.mock_id = input.to_string();
    }
    println!();

    // Priority
    let default_priority = state.priority.to_string();
    print!(
        "{} [{}]: ",
        ui::emphasis("Priority (higher = matched first)"),
        ui::dim(&default_priority)
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if !input.is_empty() {
        state.priority = input.parse().unwrap_or(state.priority);
    }
    println!();

    // Collection
    let default_collection = state.collection.clone().unwrap_or_default();
    print!(
        "{} [{}]: ",
        ui::emphasis("Collection (optional)"),
        ui::dim(if default_collection.is_empty() {
            "none"
        } else {
            &default_collection
        })
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if !input.is_empty() {
        state.collection = Some(input.to_string());
    }
    println!();

    // Output path and format
    let default_dir =
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());
    let default_path = output.unwrap_or_else(|| format!("{}/{}.yaml", default_dir, state.mock_id));
    print!(
        "{} [{}]: ",
        ui::emphasis("Output file"),
        ui::dim(&default_path)
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    let path_str = if input.is_empty() {
        &default_path
    } else {
        input
    };
    state.output_path = PathBuf::from(path_str);
    state.format = state
        .output_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("yaml")
        .to_string();
    println!();

    Ok(())
}

fn step_review_and_save(state: &WizardState) -> anyhow::Result<()> {
    ui::divider();
    println!("{}", ui::step(6, 6, "Review & Save"));
    ui::divider();
    println!();

    // Show summary
    println!("{}", ui::header("Mock Summary"));
    println!();
    println!("{}", ui::kv("Mock ID", &state.mock_id));
    println!("{}", ui::kv("Priority", &state.priority.to_string()));
    println!("{}", ui::kv("Methods", &state.methods.join(", ")));
    println!("{}", ui::kv("URL Pattern", &state.url_pattern));
    println!("{}", ui::kv("Status", &state.status.to_string()));
    println!("{}", ui::kv("Content-Type", &state.content_type));
    if let Some(delay) = state.delay_ms {
        println!("{}", ui::kv("Delay", &format!("{delay}ms")));
    }
    if !state.header_matchers.is_empty() {
        println!(
            "{}",
            ui::kv("Header Matchers", &state.header_matchers.len().to_string())
        );
    }
    if !state.query_matchers.is_empty() {
        println!(
            "{}",
            ui::kv("Query Matchers", &state.query_matchers.len().to_string())
        );
    }
    if state.body_matcher.is_some() {
        println!("{}", ui::kv("Body Matcher", "Yes"));
    }
    println!();
    println!(
        "{}",
        ui::kv("Output", &state.output_path.display().to_string())
    );
    println!("{}", ui::kv("Format", &state.format));
    println!();

    // Generate and show the mock configuration
    let body = if state.template_body.is_empty() {
        r#"{"message": "Mock response"}"#.to_string()
    } else {
        state.template_body.clone()
    };

    let mock_config = match state.format.as_str() {
        "json" => generate_json_mock_extended(state, &body)?,
        _ => generate_yaml_mock_extended(state, &body)?,
    };

    ui::preview_box("Generated Configuration", &mock_config);

    // Confirm save
    print!("{} (Y/n): ", ui::emphasis("Save mock?"));
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    if input.trim().eq_ignore_ascii_case("n") {
        println!("{}", ui::warning("Cancelled"));
        return Ok(());
    }

    // Create parent directories if they don't exist
    if let Some(parent) = state.output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create directory: {e}"))?;
    }

    // Write the mock file
    std::fs::write(&state.output_path, &mock_config)
        .map_err(|e| anyhow::anyhow!("Failed to write mock file: {e}"))?;

    println!();
    println!(
        "{}",
        ui::success(&format!(
            "Created mock: {}",
            ui::path(&state.output_path.display().to_string())
        ))
    );
    println!();
    println!(
        "{}",
        ui::dim(&format!(
            "Tip: Test with: mockpit mock test -m {} {}",
            state.methods.first().unwrap_or(&"GET".to_string()),
            state
                .url_pattern
                .replace(":id", "123")
                .replace(":userId", "456")
        ))
    );

    Ok(())
}

// Helper functions

fn detect_url_pattern_type(url: &str) -> &'static str {
    if url.contains(':') && !url.contains("://") {
        "Express-style (captures)"
    } else if url.contains('*') || url.contains('?') {
        "Glob pattern"
    } else if url.starts_with('^')
        || url.ends_with('$')
        || url.contains("\\d")
        || url.contains("\\w")
    {
        "Regex"
    } else {
        "Exact match"
    }
}

fn generate_user_template() -> String {
    r#"{
  "id": "{{ fake_uuid() }}",
  "type": "user",
  "name": "{{ fake_name() }}",
  "login": "{{ fake_email() }}",
  "created_at": "{{ fake_iso_date() }}",
  "modified_at": "{{ fake_iso_date() }}",
  "language": "en",
  "timezone": "America/Los_Angeles",
  "space_amount": 10737418240,
  "space_used": {{ fake_number(min=0, max=5000000000) }},
  "max_upload_size": 5368709120,
  "status": "active",
  "job_title": "{{ fake_job_title() }}",
  "phone": "{{ fake_phone() }}",
  "address": "{{ fake_address() }}"
}"#
    .to_string()
}

fn generate_list_template() -> String {
    r#"{
  "entries": [
    {% for i in range(start=0, end=10) %}
    {
      "id": "{{ fake_uuid() }}",
      "type": "item",
      "name": "{{ fake_sentence(words=4) }}",
      "created_at": "{{ fake_iso_date() }}",
      "modified_at": "{{ fake_iso_date() }}"
    }{% if not loop.last %},{% endif %}
    {% endfor %}
  ],
  "total_count": {{ request_query.limit | default(value=100) }},
  "limit": {{ request_query.limit | default(value=10) }},
  "offset": {{ request_query.offset | default(value=0) }}
}"#
    .to_string()
}

fn generate_item_template() -> String {
    r#"{
  "id": "{{ captures.id | default(value=fake_uuid()) }}",
  "type": "item",
  "name": "{{ fake_sentence(words=4) }}",
  "description": "{{ fake_paragraph(sentences=2) }}",
  "size": {{ fake_number(min=1024, max=10485760) }},
  "created_at": "{{ fake_iso_date() }}",
  "modified_at": "{{ fake_iso_date() }}",
  "created_by": {
    "id": "{{ fake_uuid() }}",
    "type": "user",
    "name": "{{ fake_name() }}",
    "login": "{{ fake_email() }}"
  },
  "owned_by": {
    "id": "{{ fake_uuid() }}",
    "type": "user",
    "name": "{{ fake_name() }}",
    "login": "{{ fake_email() }}"
  }
}"#
    .to_string()
}

fn generate_create_template() -> String {
    r#"{
  "id": "{{ fake_uuid() }}",
  "type": "{{ body_json.type | default(value='item') }}",
  "name": "{{ body_json.name | default(value='New Item') }}",
  "created_at": "{{ now() }}",
  "modified_at": "{{ now() }}",
  "created_by": {
    "id": "{{ fake_uuid() }}",
    "type": "user",
    "name": "{{ fake_name() }}",
    "login": "{{ fake_email() }}"
  }
}"#
    .to_string()
}

fn generate_update_template() -> String {
    r#"{
  "id": "{{ captures.id | default(value=fake_uuid()) }}",
  "type": "item",
  "name": "{{ body_json.name | default(value='Updated Item') }}",
  "description": "{{ body_json.description | default(value='') }}",
  "modified_at": "{{ now() }}",
  "modified_by": {
    "id": "{{ fake_uuid() }}",
    "type": "user",
    "name": "{{ fake_name() }}",
    "login": "{{ fake_email() }}"
  }
}"#
    .to_string()
}

fn generate_delete_template() -> String {
    // DELETE typically returns 204 No Content, but some APIs return confirmation
    String::new()
}

fn generate_error_template(status: u16) -> String {
    let (error_type, message) = match status {
        400 => ("bad_request", "Invalid request parameters"),
        401 => ("unauthorized", "Authentication required"),
        403 => ("forbidden", "Access denied"),
        404 => ("not_found", "Resource not found"),
        409 => ("conflict", "Resource already exists"),
        429 => ("rate_limit_exceeded", "Too many requests"),
        500 => ("internal_server_error", "An internal error occurred"),
        502 => ("bad_gateway", "Bad gateway"),
        503 => ("service_unavailable", "Service temporarily unavailable"),
        _ => ("error", "An error occurred"),
    };

    format!(
        r#"{{
  "type": "error",
  "status": {status},
  "code": "{error_type}",
  "message": "{message}",
  "request_id": "{{{{ fake_uuid() }}}}",
  "help_url": "https://developer.example.com/guides/api-calls/errors/"
}}"#
    )
}

fn generate_json_mock_extended(state: &WizardState, body: &str) -> anyhow::Result<String> {
    let is_template = matches!(state.body_source, BodySource::Template);
    let params = super::create::MockGeneratorParams {
        mock_id: &state.mock_id,
        method: state.methods.first().map_or("GET", String::as_str),
        url: &state.url_pattern,
        status: state.status,
        body,
        priority: state.priority,
        collection: state.collection.as_deref(),
        is_template,
    };
    generate_json_mock(&params)
}

fn generate_yaml_mock_extended(state: &WizardState, body: &str) -> anyhow::Result<String> {
    let is_template = matches!(state.body_source, BodySource::Template);
    let params = super::create::MockGeneratorParams {
        mock_id: &state.mock_id,
        method: state.methods.first().map_or("GET", String::as_str),
        url: &state.url_pattern,
        status: state.status,
        body,
        priority: state.priority,
        collection: state.collection.as_deref(),
        is_template,
    };
    generate_yaml_mock(&params)
}
