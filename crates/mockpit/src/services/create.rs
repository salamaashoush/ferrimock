//! Mock creation service — generate new mock definition files.

/// Input for creating a new mock.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// URL pattern to match
    pub url: String,
    /// HTTP method
    pub method: String,
    /// Response status code
    pub status: u16,
    /// Response body (JSON string or content)
    pub body: Option<String>,
    /// Generate template with fake data
    pub template: bool,
    /// Custom mock ID (auto-generated if None)
    pub id: Option<String>,
    /// Mock priority
    pub priority: u32,
    /// Collection name
    pub collection: Option<String>,
    /// Output format: "json" or "yaml"
    pub format: String,
}

impl Default for CreateInput {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: "GET".into(),
            status: 200,
            body: None,
            template: false,
            id: None,
            priority: 100,
            collection: None,
            format: "yaml".into(),
        }
    }
}

/// Result of mock creation.
#[derive(Debug, Clone)]
pub struct CreateResult {
    /// The generated mock ID
    pub mock_id: String,
    /// Serialized mock content (JSON or YAML)
    pub content: String,
}

/// Generate a mock ID from method and URL.
fn generate_mock_id(method: &str, url: &str) -> String {
    let slug = url
        .trim_start_matches('/')
        .replace('/', "-")
        .replace(':', "")
        .to_lowercase();
    format!("{}-{slug}", method.to_lowercase())
}

/// Create a new mock definition.
pub fn create(input: CreateInput) -> Result<CreateResult, anyhow::Error> {
    let mock_id = input
        .id
        .unwrap_or_else(|| generate_mock_id(&input.method, &input.url));

    let body = if input.template {
        generate_template_body(&input.method, &input.url)
    } else {
        input
            .body
            .unwrap_or_else(|| r#"{"message": "Mock response"}"#.to_string())
    };

    let body_key = if input.template { "template" } else { "body" };

    let mock = serde_json::json!({
        "mocks": [{
            "id": mock_id,
            "priority": input.priority,
            "enabled": true,
            "match": {
                "method": input.method.to_uppercase(),
                "url": input.url,
            },
            "response": {
                "status": input.status,
                "headers": {
                    "content-type": "application/json",
                },
                body_key: body,
            },
        }],
        "name": input.collection.as_deref().unwrap_or(&mock_id),
        "enabled": true,
    });

    let content = match input.format.as_str() {
        "json" => serde_json::to_string_pretty(&mock)?,
        _ => serde_yaml::to_string(&mock)?,
    };

    Ok(CreateResult { mock_id, content })
}

/// Generate a template body with fake data based on endpoint heuristics.
fn generate_template_body(method: &str, url: &str) -> String {
    let is_list = url.ends_with('s')
        && !url.contains("/:") // e.g. /users but not /users/:id
        || url.contains("/list")
        || url.contains("/search");

    if is_list && method.eq_ignore_ascii_case("GET") {
        // List endpoint
        r#"{
  "data": [
    {% for i in range(end=5) %}
    {
      "id": "{{ fake_uuid() }}",
      "name": "{{ fake_name() }}",
      "email": "{{ fake_email() }}",
      "createdAt": "{{ fake_iso_datetime() }}"
    }{% if not loop.last %},{% endif %}
    {% endfor %}
  ],
  "total": 5
}"#
        .into()
    } else if method.eq_ignore_ascii_case("POST") {
        // Create endpoint
        r#"{
  "id": "{{ fake_uuid() }}",
  "createdAt": "{{ now() }}",
  "message": "Created successfully"
}"#
        .into()
    } else {
        // Single resource GET or other
        r#"{
  "id": "{{ fake_uuid() }}",
  "name": "{{ fake_name() }}",
  "email": "{{ fake_email() }}",
  "createdAt": "{{ fake_iso_datetime() }}"
}"#
        .into()
    }
}
