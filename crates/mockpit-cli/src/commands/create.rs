//! Mock creation functionality

use std::io::{self, Write};

use super::ui;
use std::path::PathBuf;

/// Create a new mock definition
#[allow(clippy::too_many_arguments)]
pub fn create_mock(
    output: Option<String>,
    method: &str,
    url: &str,
    status: u16,
    body: Option<String>,
    template: bool,
    id: Option<String>,
    priority: u32,
    collection: Option<&str>,
    interactive: bool,
) -> anyhow::Result<()> {
    println!("{}", ui::header("Create New Mock"));
    println!();

    // Generate mock ID if not provided
    let mock_id = id.unwrap_or_else(|| {
        // Create ID from method and URL
        let url_part = url
            .trim_start_matches('^')
            .trim_end_matches('$')
            .trim_matches('/')
            .replace(['/', '.'], "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>();
        format!("{}-{}", method.to_lowercase(), url_part)
    });

    // Determine output path and format
    let (output_path, format) = if let Some(out) = output {
        // User provided path - extract format from extension
        let path = PathBuf::from(&out);
        let fmt = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("yaml")
            .to_string();
        (path, fmt)
    } else {
        // Default to mocks/collections/<mock-id>.yaml
        let default_dir =
            std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());
        let filename = format!("{mock_id}.yaml");
        let path = PathBuf::from(default_dir).join(filename);
        (path, "yaml".to_string())
    };

    // Interactive mode - show what will be created and confirm
    if interactive {
        println!("{}", ui::kv("Mock ID", &mock_id));
        println!("{}", ui::kv("Method", &method.to_uppercase()));
        println!("{}", ui::kv("URL", url));
        println!("{}", ui::kv("Status", &status.to_string()));
        println!("{}", ui::kv("Priority", &priority.to_string()));
        if let Some(coll) = collection {
            println!("{}", ui::kv("Collection", coll));
        }
        println!("{}", ui::kv("Format", &format));
        println!(
            "{}",
            ui::kv("Template", if template { "Yes" } else { "No" })
        );
        println!();

        print!("{} ", ui::emphasis("Continue? (y/N):"));
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("{}", ui::warning("Cancelled"));
            return Ok(());
        }
    }

    // Build the mock configuration
    let mock_body = if let Some(body_input) = body {
        // Check if body is a file reference (@file.json)
        if body_input.starts_with('@') {
            let file_path = body_input.trim_start_matches('@');
            std::fs::read_to_string(file_path)
                .map_err(|e| anyhow::anyhow!("Failed to read body file: {e}"))?
        } else {
            body_input
        }
    } else if template {
        // Generate template with fake data
        generate_template_body(method, url)
    } else {
        // Default empty JSON object
        r#"{"message": "Mock response"}"#.to_string()
    };

    // Create the mock configuration string based on format
    let params = MockGeneratorParams {
        mock_id: &mock_id,
        method,
        url,
        status,
        body: &mock_body,
        priority,
        collection,
        is_template: template,
    };
    let mock_config = match format.as_str() {
        "json" => generate_json_mock(&params)?,
        "yaml" | "yml" => generate_yaml_mock(&params)?,
        _ => {
            anyhow::bail!("Unsupported format: {format}");
        }
    };

    // Create parent directories if they don't exist
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create directory: {e}"))?;
    }

    // Write the mock file
    std::fs::write(&output_path, mock_config)
        .map_err(|e| anyhow::anyhow!("Failed to write mock file: {e}"))?;

    let output_display = output_path.display().to_string();
    println!(
        "{}",
        ui::success(&format!("Created mock: {}", ui::path(&output_display)))
    );
    println!();
    println!("{}", ui::kv("Mock ID", &mock_id));
    println!("{}", ui::kv("Method", &method.to_uppercase()));
    println!("{}", ui::kv("URL Pattern", url));
    println!("{}", ui::kv("Status", &status.to_string()));
    println!("{}", ui::kv("File", &output_display));
    println!();
    println!(
        "{}",
        ui::dim(&format!(
            "Tip: Edit {} to customize the mock",
            ui::path(&output_display)
        ))
    );

    Ok(())
}

/// Generate a template body with fake data based on the endpoint
pub fn generate_template_body(method: &str, url: &str) -> String {
    // Detect if this looks like a list endpoint
    let is_list = url.contains("/users")
        || url.contains("/files")
        || url.contains("/folders")
        || url.contains("/items");

    if method.to_uppercase() == "GET" && is_list {
        // List endpoint template
        r#"{
  "items": [
    {% for i in range(start=0, end=10) %}
    {
      "id": "{{ fake_uuid() }}",
      "name": "{{ fake_sentence(words=4) }}",
      "created_at": "{{ fake_iso_date() }}",
      "modified_at": "{{ fake_iso_date() }}"
    }{% if not loop.last %},{% endif %}
    {% endfor %}
  ],
  "total_count": 10,
  "limit": 10,
  "offset": 0
}"#
        .to_string()
    } else if method.to_uppercase() == "GET" {
        // Single resource endpoint template
        r#"{
  "id": "{{ fake_uuid() }}",
  "name": "{{ fake_sentence(words=4) }}",
  "type": "{{ ["file", "folder", "document"] | random_choice }}",
  "created_at": "{{ fake_iso_date() }}",
  "modified_at": "{{ fake_iso_date() }}"
}"#
        .to_string()
    } else if method.to_uppercase() == "POST" {
        // Create endpoint template
        r#"{
  "id": "{{ fake_uuid() }}",
  "name": "{{ body_json.name | default(value='New Item') }}",
  "created_at": "{{ now() }}",
  "status": "created"
}"#
        .to_string()
    } else {
        // Generic template
        r#"{
  "success": true,
  "message": "{{ method }} request completed",
  "id": "{{ fake_uuid() }}",
  "timestamp": "{{ now() }}"
}"#
        .to_string()
    }
}

/// Parameters for mock generation
pub struct MockGeneratorParams<'a> {
    pub mock_id: &'a str,
    pub method: &'a str,
    pub url: &'a str,
    pub status: u16,
    pub body: &'a str,
    pub priority: u32,
    pub collection: Option<&'a str>,
    pub is_template: bool,
}

/// Generate JSON mock configuration
pub fn generate_json_mock(params: &MockGeneratorParams<'_>) -> anyhow::Result<String> {
    let MockGeneratorParams {
        mock_id,
        method,
        url,
        status,
        body,
        priority,
        collection,
        is_template,
    } = params;
    let mut config = serde_json::Map::new();

    // Build the mock object
    let mut mock_obj = serde_json::Map::new();
    mock_obj.insert(
        "id".to_string(),
        serde_json::Value::String(mock_id.to_string()),
    );
    mock_obj.insert(
        "priority".to_string(),
        serde_json::Value::Number((*priority).into()),
    );
    mock_obj.insert("enabled".to_string(), serde_json::Value::Bool(true));

    // Build match object
    let mut match_obj = serde_json::Map::new();
    match_obj.insert(
        "methods".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::String(method.to_uppercase())]),
    );
    match_obj.insert(
        "url".to_string(),
        serde_json::Value::String(url.to_string()),
    );
    mock_obj.insert("match".to_string(), serde_json::Value::Object(match_obj));

    // Build response object
    let mut response_obj = serde_json::Map::new();
    response_obj.insert(
        "status".to_string(),
        serde_json::Value::Number((*status).into()),
    );

    let mut headers_obj = serde_json::Map::new();
    headers_obj.insert(
        "content-type".to_string(),
        serde_json::Value::String("application/json".to_string()),
    );
    response_obj.insert(
        "headers".to_string(),
        serde_json::Value::Object(headers_obj),
    );
    let field_name = if *is_template { "template" } else { "body" };
    response_obj.insert(
        field_name.to_string(),
        serde_json::Value::String(body.to_string()),
    );
    mock_obj.insert(
        "response".to_string(),
        serde_json::Value::Object(response_obj),
    );

    // Add mock to mocks array
    config.insert(
        "mocks".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::Object(mock_obj)]),
    );

    if let Some(coll) = collection {
        config.insert(
            "name".to_string(),
            serde_json::Value::String(coll.to_string()),
        );
        config.insert("enabled".to_string(), serde_json::Value::Bool(true));
    }

    serde_json::to_string_pretty(&serde_json::Value::Object(config))
        .map_err(|e| anyhow::anyhow!("Failed to serialize JSON: {e}"))
}

/// Generate YAML mock configuration
pub fn generate_yaml_mock(params: &MockGeneratorParams<'_>) -> anyhow::Result<String> {
    let MockGeneratorParams {
        mock_id,
        method,
        url,
        status,
        body,
        priority,
        collection,
        is_template,
    } = params;
    let mut config = serde_json::Map::new();

    // Build the mock object
    let mut mock_obj = serde_json::Map::new();
    mock_obj.insert(
        "id".to_string(),
        serde_json::Value::String(mock_id.to_string()),
    );
    mock_obj.insert(
        "priority".to_string(),
        serde_json::Value::Number((*priority).into()),
    );
    mock_obj.insert("enabled".to_string(), serde_json::Value::Bool(true));

    // Build match object
    let mut match_obj = serde_json::Map::new();
    match_obj.insert(
        "methods".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::String(method.to_uppercase())]),
    );
    match_obj.insert(
        "url".to_string(),
        serde_json::Value::String(url.to_string()),
    );
    mock_obj.insert("match".to_string(), serde_json::Value::Object(match_obj));

    // Build response object
    let mut response_obj = serde_json::Map::new();
    response_obj.insert(
        "status".to_string(),
        serde_json::Value::Number((*status).into()),
    );

    let mut headers_obj = serde_json::Map::new();
    headers_obj.insert(
        "content-type".to_string(),
        serde_json::Value::String("application/json".to_string()),
    );
    response_obj.insert(
        "headers".to_string(),
        serde_json::Value::Object(headers_obj),
    );
    let field_name = if *is_template { "template" } else { "body" };
    response_obj.insert(
        field_name.to_string(),
        serde_json::Value::String(body.to_string()),
    );
    mock_obj.insert(
        "response".to_string(),
        serde_json::Value::Object(response_obj),
    );

    // Add mock to mocks array
    config.insert(
        "mocks".to_string(),
        serde_json::Value::Array(vec![serde_json::Value::Object(mock_obj)]),
    );

    if let Some(coll) = collection {
        config.insert(
            "name".to_string(),
            serde_json::Value::String(coll.to_string()),
        );
        config.insert("enabled".to_string(), serde_json::Value::Bool(true));
    }

    serde_yaml::to_string(&serde_json::Value::Object(config))
        .map_err(|e| anyhow::anyhow!("Failed to serialize YAML: {e}"))
}
