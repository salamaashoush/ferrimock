//! Mock file formatting service.
//!
//! Canonical, domain-aware formatter (key ordering + body expansion) shared by
//! the CLI `mock format` command and the NAPI `services.format` binding.

use crate::config::{format_body, MockCollectionConfig};
use std::path::{Path, PathBuf};

/// Input for formatting mock files.
#[derive(Debug, Clone)]
pub struct FormatInput {
    /// Path to a file or directory
    pub path: String,
    /// Check-only mode (don't write, just report)
    pub check: bool,
}

/// Input for formatting content from a string.
#[derive(Debug, Clone)]
pub struct FormatContentInput {
    /// Mock configuration content
    pub content: String,
    /// File format: "json", "yaml", or "yml"
    pub file_format: String,
}

/// Result of formatting a single file.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FormatFileResult {
    /// File path
    pub path: String,
    /// Whether the file was changed (or would be changed in check mode)
    pub changed: bool,
    /// Error message if formatting failed
    pub error: Option<String>,
}

/// Result of formatting.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FormatOutput {
    /// Results per file
    pub files: Vec<FormatFileResult>,
    /// Number of files formatted/changed
    pub formatted_count: usize,
    /// Number of files with errors
    pub error_count: usize,
    /// Number of files already formatted
    pub unchanged_count: usize,
}

/// Format mock files at a path.
pub fn format_path(input: FormatInput) -> Result<FormatOutput, crate::MockpitError> {
    let path = PathBuf::from(&input.path);
    crate::mp_ensure!(path.exists(), "Path does not exist: {}", input.path);

    let files = if path.is_file() {
        vec![path]
    } else {
        collect_mock_files(&path)?
    };

    let mut results = Vec::new();
    let mut formatted = 0;
    let mut errors = 0;
    let mut unchanged = 0;

    for file in files {
        match format_single_file(&file, input.check) {
            Ok(changed) => {
                if changed {
                    formatted += 1;
                } else {
                    unchanged += 1;
                }
                results.push(FormatFileResult {
                    path: file.display().to_string(),
                    changed,
                    error: None,
                });
            }
            Err(e) => {
                errors += 1;
                results.push(FormatFileResult {
                    path: file.display().to_string(),
                    changed: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(FormatOutput {
        files: results,
        formatted_count: formatted,
        error_count: errors,
        unchanged_count: unchanged,
    })
}

/// Format content from a string.
pub fn format_content(input: FormatContentInput) -> Result<String, crate::MockpitError> {
    match input.file_format.as_str() {
        "json" => format_json(&input.content),
        "yaml" | "yml" => format_yaml(&input.content),
        other => crate::mp_bail!("Unsupported format: {other}"),
    }
}

fn collect_mock_files(dir: &Path) -> Result<Vec<PathBuf>, crate::MockpitError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "json" | "yaml" | "yml") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn format_single_file(path: &Path, check: bool) -> Result<bool, crate::MockpitError> {
    let original = std::fs::read_to_string(path)?;

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("yaml");

    let formatted = match ext {
        "json" => format_json(&original)?,
        "yaml" | "yml" => format_yaml(&original)?,
        _ => return Ok(false),
    };

    if formatted == original {
        return Ok(false);
    }

    if !check {
        std::fs::write(path, &formatted)?;
    }

    Ok(true)
}

/// Format a JSON mock file: serde round-trip, format inline body/template strings
/// (Prettier-style via `format_body`), expand JSON-in-string bodies to objects,
/// then apply domain-aware key ordering.
fn format_json(content: &str) -> Result<String, crate::MockpitError> {
    let config: MockCollectionConfig =
        serde_json::from_str(content).map_err(|e| crate::mp_err!("JSON parse error: {e}"))?;
    let mut output = serde_json::to_string_pretty(&config)
        .map_err(|e| crate::mp_err!("JSON serialize error: {e}"))?;
    output.push('\n');

    let mut config_mut: MockCollectionConfig =
        serde_json::from_str(&output).map_err(|e| crate::mp_err!("JSON re-parse error: {e}"))?;
    let mut changed = false;
    for mock in &mut config_mut.mocks {
        if let Some(ref mut rc) = mock.response_config {
            if let Some(body) = rc.body().cloned() {
                let f = format_body(&body);
                if f != body {
                    rc.set_body(f);
                    changed = true;
                }
            }
            if let Some(tmpl) = rc.template().cloned() {
                let f = format_body(&tmpl);
                if f != tmpl {
                    rc.set_template(f);
                    changed = true;
                }
            }
        }
    }

    let serialized = if changed {
        let mut out = serde_json::to_string_pretty(&config_mut)
            .map_err(|e| crate::mp_err!("JSON re-serialize error: {e}"))?;
        out.push('\n');
        out
    } else {
        output
    };

    expand_json_body_strings(&serialized)
}

/// Format a YAML mock file (comment preservation deferred).
fn format_yaml(content: &str) -> Result<String, crate::MockpitError> {
    let config: MockCollectionConfig =
        serde_yaml::from_str(content).map_err(|e| crate::mp_err!("YAML parse error: {e}"))?;
    let structural =
        serde_yaml::to_string(&config).map_err(|e| crate::mp_err!("YAML serialize error: {e}"))?;

    let mut config_mut: MockCollectionConfig = serde_yaml::from_str(&structural)
        .map_err(|e| crate::mp_err!("YAML re-parse error: {e}"))?;
    let mut changed = false;
    for mock in &mut config_mut.mocks {
        if let Some(ref mut rc) = mock.response_config {
            if let Some(body) = rc.body().cloned() {
                let f = format_body(&body);
                if f != body {
                    rc.set_body(f);
                    changed = true;
                }
            }
            if let Some(tmpl) = rc.template().cloned() {
                let f = format_body(&tmpl);
                if f != tmpl {
                    rc.set_template(f);
                    changed = true;
                }
            }
        }
    }

    let serialized = if changed {
        serde_yaml::to_string(&config_mut)
            .map_err(|e| crate::mp_err!("YAML re-serialize error: {e}"))?
    } else {
        structural
    };

    expand_yaml_body_strings(&serialized)
}

fn expand_json_body_strings(json_str: &str) -> Result<String, crate::MockpitError> {
    let mut value: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| crate::mp_err!("JSON re-parse for body expansion: {e}"))?;
    expand_json_body_values(&mut value);
    sort_json_keys(&mut value);
    let mut output = serde_json::to_string_pretty(&value)?;
    output.push('\n');
    Ok(output)
}

fn expand_json_body_values(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(body_val) = map.get_mut("body")
                && let Some(body_str) = body_val.as_str()
                && !body_str.contains("{{")
                && !body_str.contains("{%")
                && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body_str)
                && (parsed.is_object() || parsed.is_array())
            {
                *body_val = parsed;
            }
            for val in map.values_mut() {
                expand_json_body_values(val);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                expand_json_body_values(item);
            }
        }
        _ => {}
    }
}

fn expand_yaml_body_strings(yaml_str: &str) -> Result<String, crate::MockpitError> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(yaml_str)
        .map_err(|e| crate::mp_err!("YAML re-parse for body expansion: {e}"))?;
    expand_yaml_body_values(&mut value);
    sort_yaml_keys(&mut value);
    serde_yaml::to_string(&value)
        .map_err(|e| crate::mp_err!("YAML re-serialize after body expansion: {e}"))
}

fn expand_yaml_body_values(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let body_key = serde_yaml::Value::String("body".to_string());
            if let Some(body_val) = map.get_mut(&body_key)
                && let Some(body_str) = body_val.as_str()
                && !body_str.contains("{{")
                && !body_str.contains("{%")
                && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body_str)
                && (parsed.is_object() || parsed.is_array())
                && let Ok(yaml_val) = serde_yaml::to_value(&parsed)
            {
                *body_val = yaml_val;
            }
            for (_, val) in map.iter_mut() {
                expand_yaml_body_values(val);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for item in seq {
                expand_yaml_body_values(item);
            }
        }
        _ => {}
    }
}

/// JSON sorting context tracks where we are in the mock structure.
#[derive(Clone, Copy)]
enum JsonContext {
    TopLevel,
    Mock,
    MatchConfig,
    ResponseConfig,
    PatchConfig,
    RequestConfig,
    Other,
}

/// Recursively sort all object keys with domain-aware ordering.
fn sort_json_keys(value: &mut serde_json::Value) {
    sort_json_keys_inner(value, JsonContext::TopLevel);
}

fn sort_json_keys_inner(value: &mut serde_json::Value, ctx: JsonContext) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map.iter_mut() {
                let child_ctx = child_context(ctx, key);
                sort_json_keys_inner(val, child_ctx);
            }
            let mut sorted: Vec<_> = std::mem::take(map).into_iter().collect();
            sorted.sort_by(|a, b| {
                json_key_order(&a.0, ctx)
                    .cmp(&json_key_order(&b.0, ctx))
                    .then_with(|| a.0.cmp(&b.0))
            });
            *map = sorted.into_iter().collect();
        }
        serde_json::Value::Array(arr) => {
            let child_ctx = match ctx {
                JsonContext::TopLevel => JsonContext::Mock,
                _ => ctx,
            };
            for item in arr {
                sort_json_keys_inner(item, child_ctx);
            }
        }
        _ => {}
    }
}

fn sort_yaml_keys(value: &mut serde_yaml::Value) {
    sort_yaml_keys_inner(value, JsonContext::TopLevel);
}

fn sort_yaml_keys_inner(value: &mut serde_yaml::Value, ctx: JsonContext) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, val) in map.iter_mut() {
                let child_ctx = child_context(ctx, key.as_str().unwrap_or(""));
                sort_yaml_keys_inner(val, child_ctx);
            }
            let mut sorted: Vec<_> = std::mem::take(map).into_iter().collect();
            sorted.sort_by(|a, b| {
                let a_str = a.0.as_str().unwrap_or("");
                let b_str = b.0.as_str().unwrap_or("");
                json_key_order(a_str, ctx)
                    .cmp(&json_key_order(b_str, ctx))
                    .then_with(|| a_str.cmp(b_str))
            });
            *map = sorted.into_iter().collect();
        }
        serde_yaml::Value::Sequence(seq) => {
            let child_ctx = match ctx {
                JsonContext::TopLevel => JsonContext::Mock,
                _ => ctx,
            };
            for item in seq {
                sort_yaml_keys_inner(item, child_ctx);
            }
        }
        _ => {}
    }
}

/// Determine the child sorting context from the current context and key name.
fn child_context(ctx: JsonContext, key: &str) -> JsonContext {
    match ctx {
        JsonContext::TopLevel => match key {
            "mocks" => JsonContext::TopLevel,
            _ => JsonContext::Other,
        },
        JsonContext::Mock => match key {
            "match" | "match_config" => JsonContext::MatchConfig,
            "response" | "response_config" => JsonContext::ResponseConfig,
            "patch" => JsonContext::PatchConfig,
            "request" | "request_transform" => JsonContext::RequestConfig,
            _ => JsonContext::Other,
        },
        _ => JsonContext::Other,
    }
}

/// Canonical key order for mock objects based on context.
fn json_key_order(key: &str, ctx: JsonContext) -> u32 {
    match ctx {
        JsonContext::TopLevel => match key {
            "name" => 0,
            "description" => 1,
            "enabled" => 2,
            "vars" => 3,
            "mocks" => 10,
            _ => 5,
        },
        JsonContext::Mock => match key {
            "id" => 0,
            "description" => 1,
            "priority" => 2,
            "enabled" => 3,
            "scope" => 4,
            "vars" => 5,
            "match" | "match_config" => 10,
            "request" | "request_transform" => 20,
            "response" | "response_config" => 30,
            "patch" => 35,
            "delay" => 36,
            _ => 6,
        },
        JsonContext::MatchConfig => match key {
            "methods" | "method" => 0,
            "url_pattern" | "url" | "urls" => 1,
            "headers" => 2,
            "query" => 3,
            "body" => 4,
            _ => 5,
        },
        JsonContext::ResponseConfig => match key {
            "status" => 0,
            "headers" => 1,
            "body" | "template" | "file" | "template_file" | "json" => 2,
            _ => 3,
        },
        JsonContext::PatchConfig => match key {
            "jsonpath" => 0,
            "regex" => 1,
            "headers" => 2,
            "operations" => 3,
            _ => 4,
        },
        JsonContext::RequestConfig => match key {
            "delay" => 0,
            "timeout" => 1,
            "forward_to" => 2,
            "rewrite_path" => 3,
            "headers" => 4,
            "query" => 5,
            "body" => 6,
            _ => 7,
        },
        JsonContext::Other => 0,
    }
}
