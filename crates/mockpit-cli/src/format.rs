//! Mock file formatting

use crate::ui;
use mockpit_config::{MockCollectionConfig, format_body};
use std::path::{Path, PathBuf};

pub fn format_mocks(
    path: Option<String>,
    check: bool,
    stdin: bool,
    file_format: Option<&str>,
) -> anyhow::Result<()> {
    // stdin mode: read from stdin, format, write to stdout
    if stdin {
        return format_stdin(file_format);
    }

    let input_path = path.unwrap_or_else(|| {
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
    });

    let path = PathBuf::from(&input_path);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {input_path}");
    }

    let action = if check { "Checking" } else { "Formatting" };
    println!(
        "{}",
        ui::action(&format!("{} mocks in {}", action, ui::path(&input_path)))
    );
    println!();

    let files = collect_mock_files(&path)?;
    if files.is_empty() {
        println!("{}", ui::warning("No mock files found"));
        return Ok(());
    }

    let mut unformatted_count = 0;
    let mut formatted_count = 0;
    let mut error_count = 0;

    for file_path in &files {
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        match format_file(file_path, check) {
            Ok(FormatResult::AlreadyFormatted) => {
                formatted_count += 1;
            }
            Ok(FormatResult::Formatted) => {
                println!("{} {}", ui::success("formatted"), ui::path(filename));
                formatted_count += 1;
            }
            Ok(FormatResult::WouldChange) => {
                println!("{} {}", ui::warning("would change"), ui::path(filename));
                unformatted_count += 1;
            }
            Err(e) => {
                println!("{} {} - {}", ui::error("error"), ui::path(filename), e);
                error_count += 1;
            }
        }
    }

    println!();
    if check {
        if unformatted_count > 0 {
            println!(
                "{}",
                ui::error(&format!(
                    "{} file(s) would be reformatted, {} already formatted, {} error(s)",
                    ui::number(unformatted_count),
                    ui::number(formatted_count),
                    ui::number(error_count)
                ))
            );
            #[allow(clippy::exit)]
            std::process::exit(1);
        }
        println!(
            "{}",
            ui::success(&format!(
                "All {} file(s) are properly formatted",
                ui::number(formatted_count)
            ))
        );
    } else {
        println!(
            "{}",
            ui::success(&format!(
                "{} file(s) formatted, {} error(s)",
                ui::number(formatted_count),
                ui::number(error_count)
            ))
        );
    }

    Ok(())
}

/// Read from stdin, format, write to stdout.
fn format_stdin(file_format: Option<&str>) -> anyhow::Result<()> {
    use std::io::Read;

    let mut content = String::new();
    std::io::stdin().read_to_string(&mut content)?;

    let formatted = match file_format {
        Some("json") => format_json_file(&content)?,
        Some("yaml" | "yml") => format_yaml_file(&content)?,
        _ => {
            anyhow::bail!("Cannot determine format: use --file-format with json, yaml, or yml");
        }
    };

    print!("{formatted}");
    Ok(())
}

enum FormatResult {
    AlreadyFormatted,
    Formatted,
    WouldChange,
}

fn collect_mock_files(path: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut files = Vec::new();
    let read_dir = std::fs::read_dir(path)?;

    for entry in read_dir.filter_map(Result::ok) {
        let entry_path = entry.path();
        if !entry_path.is_file() {
            continue;
        }
        let ext = entry_path.extension().and_then(|e| e.to_str());
        if matches!(ext, Some("json" | "yaml" | "yml")) {
            files.push(entry_path);
        }
    }

    files.sort();
    Ok(files)
}

fn format_file(path: &Path, check: bool) -> anyhow::Result<FormatResult> {
    let original = std::fs::read_to_string(path)?;
    let extension = path.extension().and_then(|e| e.to_str());

    let formatted = match extension {
        Some("json") => format_json_file(&original)?,
        Some("yaml" | "yml") => format_yaml_file(&original)?,
        _ => return Err(anyhow::anyhow!("Unsupported file format")),
    };

    // Collect and format external file references
    let mock_dir = path.parent();
    if let Some(dir) = mock_dir {
        format_external_references(&original, extension, dir, check)?;
    }

    if original == formatted {
        return Ok(FormatResult::AlreadyFormatted);
    }

    if check {
        return Ok(FormatResult::WouldChange);
    }

    std::fs::write(path, &formatted)?;
    Ok(FormatResult::Formatted)
}

/// Format a JSON mock file using serde round-trip (JSON has no comments to preserve).
fn format_json_file(original: &str) -> anyhow::Result<String> {
    let config: MockCollectionConfig =
        serde_json::from_str(original).map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?;
    let mut output = serde_json::to_string_pretty(&config)
        .map_err(|e| anyhow::anyhow!("JSON serialize error: {e}"))?;
    output.push('\n');

    // Format body strings
    let mut config_mut: MockCollectionConfig =
        serde_json::from_str(&output).map_err(|e| anyhow::anyhow!("JSON re-parse error: {e}"))?;

    let mut changed = false;
    for mock in &mut config_mut.mocks {
        if let Some(ref mut response_config) = mock.response_config {
            // Format body field (static inline)
            if let Some(body) = response_config.body().cloned() {
                let formatted = format_body(&body);
                if formatted != body {
                    response_config.set_body(formatted);
                    changed = true;
                }
            }
            // Format template field (Tera inline)
            if let Some(tmpl) = response_config.template().cloned() {
                let formatted = format_body(&tmpl);
                if formatted != tmpl {
                    response_config.set_template(formatted);
                    changed = true;
                }
            }
        }
    }

    let serialized = if changed {
        let mut out = serde_json::to_string_pretty(&config_mut)
            .map_err(|e| anyhow::anyhow!("JSON re-serialize error: {e}"))?;
        out.push('\n');
        out
    } else {
        output
    };

    // Expand body strings containing valid JSON back to objects
    expand_json_body_strings(&serialized)
}

/// Format a YAML mock file using serde round-trip (YAML comment preservation deferred).
fn format_yaml_file(original: &str) -> anyhow::Result<String> {
    let config: MockCollectionConfig =
        serde_yaml::from_str(original).map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?;
    let structural =
        serde_yaml::to_string(&config).map_err(|e| anyhow::anyhow!("YAML serialize error: {e}"))?;

    // Format body and template strings
    let mut config_mut: MockCollectionConfig = serde_yaml::from_str(&structural)
        .map_err(|e| anyhow::anyhow!("YAML re-parse error: {e}"))?;

    let mut changed = false;
    for mock in &mut config_mut.mocks {
        if let Some(ref mut response_config) = mock.response_config {
            // Format body field (static inline)
            if let Some(body) = response_config.body().cloned() {
                let formatted = format_body(&body);
                if formatted != body {
                    response_config.set_body(formatted);
                    changed = true;
                }
            }
            // Format template field (Tera inline)
            if let Some(tmpl) = response_config.template().cloned() {
                let formatted = format_body(&tmpl);
                if formatted != tmpl {
                    response_config.set_template(formatted);
                    changed = true;
                }
            }
        }
    }

    let serialized = if changed {
        serde_yaml::to_string(&config_mut)
            .map_err(|e| anyhow::anyhow!("YAML re-serialize error: {e}"))?
    } else {
        structural
    };

    // Expand body strings containing valid JSON back to YAML mappings
    expand_yaml_body_strings(&serialized)
}

/// Find and format external files referenced via `file` and `template_file` fields.
fn format_external_references(
    content: &str,
    extension: Option<&str>,
    mock_dir: &Path,
    check: bool,
) -> anyhow::Result<()> {
    let config: MockCollectionConfig =
        match extension {
            Some("json") => serde_json::from_str(content)
                .map_err(|e| anyhow::anyhow!("JSON parse error: {e}"))?,
            Some("yaml" | "yml") => serde_yaml::from_str(content)
                .map_err(|e| anyhow::anyhow!("YAML parse error: {e}"))?,
            _ => return Ok(()),
        };

    for mock in &config.mocks {
        if let Some(ref response_config) = mock.response_config {
            if let Some(ref_path) = response_config.template_file_ref()
                && let Err(e) = format_external_file(mock_dir, ref_path, true, check)
            {
                let mock_id = &mock.id;
                println!(
                    "  {} formatting template for mock {}: {}",
                    ui::warning("warn"),
                    mock_id,
                    e
                );
            }
            if let Some(ref_path) = response_config.file_ref()
                && let Err(e) = format_external_file(mock_dir, ref_path, false, check)
            {
                let mock_id = &mock.id;
                println!(
                    "  {} formatting file for mock {}: {}",
                    ui::warning("warn"),
                    mock_id,
                    e
                );
            }
        }
    }

    Ok(())
}

/// Format an external file referenced from a mock body.
/// - Templates (is_template=true): format with `format_body()`
/// - JSON files (is_template=false, .json extension): pretty-print with serde_json
fn format_external_file(
    mock_dir: &Path,
    ref_path: &str,
    is_template: bool,
    check: bool,
) -> anyhow::Result<bool> {
    let full_path = mock_dir.join(ref_path);
    if !full_path.exists() {
        return Ok(false); // File doesn't exist -- validation catches this separately
    }

    let original = std::fs::read_to_string(&full_path)?;

    let formatted = if is_template {
        format_body(&original)
    } else {
        // For file: references, only format JSON files
        let ext = full_path.extension().and_then(|e| e.to_str());
        match ext {
            Some("json") => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&original) {
                    let mut pretty = serde_json::to_string_pretty(&value)?;
                    pretty.push('\n');
                    pretty
                } else {
                    return Ok(false); // Invalid JSON -- skip
                }
            }
            _ => return Ok(false), // Non-JSON file references -- skip
        }
    };

    if original == formatted {
        return Ok(false);
    }

    if !check {
        std::fs::write(&full_path, &formatted)?;
        let filename = full_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        println!(
            "  {} {}",
            ui::success("formatted external"),
            ui::path(filename)
        );
    }

    Ok(true)
}

/// Post-process JSON output: find `"body": "{ ... }"` string values that contain
/// valid JSON and expand them back to objects so they render as nested structures
/// instead of one long escaped line.
fn expand_json_body_strings(json_str: &str) -> anyhow::Result<String> {
    let mut value: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("JSON re-parse for body expansion: {e}"))?;

    expand_json_body_values(&mut value);
    sort_json_keys(&mut value);

    let mut output = serde_json::to_string_pretty(&value)?;
    output.push('\n');
    Ok(output)
}

/// Recursively walk a JSON value tree and convert `"body"` string fields
/// that contain valid JSON into parsed JSON values (objects/arrays).
fn expand_json_body_values(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // Check if this object has a "body" key with a string value that is valid JSON
            if let Some(body_val) = map.get_mut("body")
                && let Some(body_str) = body_val.as_str()
            {
                // Only expand if it's valid JSON object or array (not a template or plain text)
                if !body_str.contains("{{")
                    && !body_str.contains("{%")
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body_str)
                    && (parsed.is_object() || parsed.is_array())
                {
                    *body_val = parsed;
                }
            }
            // Recurse into all values
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

/// Post-process YAML output: find `body:` string fields that contain valid JSON
/// and expand them to nested YAML mappings instead of block scalars with JSON text.
fn expand_yaml_body_strings(yaml_str: &str) -> anyhow::Result<String> {
    let mut value: serde_yaml::Value = serde_yaml::from_str(yaml_str)
        .map_err(|e| anyhow::anyhow!("YAML re-parse for body expansion: {e}"))?;

    expand_yaml_body_values(&mut value);
    sort_yaml_keys(&mut value);

    serde_yaml::to_string(&value)
        .map_err(|e| anyhow::anyhow!("YAML re-serialize after body expansion: {e}"))
}

/// Recursively walk a YAML value tree and convert `body` string fields
/// that contain valid JSON into parsed YAML values (mappings/sequences).
fn expand_yaml_body_values(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let body_key = serde_yaml::Value::String("body".to_string());
            if let Some(body_val) = map.get_mut(&body_key)
                && let Some(body_str) = body_val.as_str()
            {
                // Only expand if it's valid JSON (not a template or plain text)
                if !body_str.contains("{{")
                    && !body_str.contains("{%")
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body_str)
                    && (parsed.is_object() || parsed.is_array())
                {
                    // Convert serde_json::Value to serde_yaml::Value
                    if let Ok(yaml_val) = serde_yaml::to_value(&parsed) {
                        *body_val = yaml_val;
                    }
                }
            }
            // Recurse into all values
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

/// Recursively sort all object keys in a JSON value using domain-aware ordering.
///
/// Mock-level keys use canonical order (id, description, priority before match, response).
/// Inner keys (match_config, response_config internals) use their own canonical order.
/// All other keys sort alphabetically for deterministic output.
fn sort_json_keys(value: &mut serde_json::Value) {
    sort_json_keys_inner(value, JsonContext::TopLevel);
}

/// JSON sorting context tracks where we are in the mock structure.
#[derive(Clone, Copy)]
enum JsonContext {
    TopLevel,       // collection level (name, description, mocks array)
    Mock,           // inside a mock object
    MatchConfig,    // inside match_config
    ResponseConfig, // inside response_config
    PatchConfig,    // inside patch (response patches)
    RequestConfig,  // inside request (request transforms)
    Other,          // anywhere else (sort alphabetically)
}

fn sort_json_keys_inner(value: &mut serde_json::Value, ctx: JsonContext) {
    match value {
        serde_json::Value::Object(map) => {
            // Determine child context based on key names
            for (key, val) in map.iter_mut() {
                let child_ctx = match ctx {
                    JsonContext::TopLevel => match key.as_str() {
                        "mocks" => JsonContext::TopLevel, // array of mocks
                        _ => JsonContext::Other,
                    },
                    JsonContext::Mock => match key.as_str() {
                        "match" | "match_config" => JsonContext::MatchConfig,
                        "response" | "response_config" => JsonContext::ResponseConfig,
                        "patch" => JsonContext::PatchConfig,
                        "request" | "request_transform" => JsonContext::RequestConfig,
                        _ => JsonContext::Other,
                    },
                    _ => JsonContext::Other,
                };
                sort_json_keys_inner(val, child_ctx);
            }
            let entries: Vec<_> = std::mem::take(map).into_iter().collect();
            let mut sorted = entries;
            sorted.sort_by(|a, b| {
                let ord_a = json_key_order(&a.0, ctx);
                let ord_b = json_key_order(&b.0, ctx);
                ord_a.cmp(&ord_b).then_with(|| a.0.cmp(&b.0))
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

/// Canonical key order for JSON objects based on context.
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
        JsonContext::Other => 0, // all equal, fallback to alphabetical
    }
}

/// Recursively sort all mapping keys in a YAML value using domain-aware ordering.
fn sort_yaml_keys(value: &mut serde_yaml::Value) {
    sort_yaml_keys_inner(value, JsonContext::TopLevel);
}

fn sort_yaml_keys_inner(value: &mut serde_yaml::Value, ctx: JsonContext) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, val) in map.iter_mut() {
                let key_str = key.as_str().unwrap_or("");
                let child_ctx = match ctx {
                    JsonContext::TopLevel => match key_str {
                        "mocks" => JsonContext::TopLevel,
                        _ => JsonContext::Other,
                    },
                    JsonContext::Mock => match key_str {
                        "match" | "match_config" => JsonContext::MatchConfig,
                        "response" | "response_config" => JsonContext::ResponseConfig,
                        "patch" => JsonContext::PatchConfig,
                        "request" | "request_transform" => JsonContext::RequestConfig,
                        _ => JsonContext::Other,
                    },
                    _ => JsonContext::Other,
                };
                sort_yaml_keys_inner(val, child_ctx);
            }
            let entries: Vec<_> = std::mem::take(map).into_iter().collect();
            let mut sorted = entries;
            sorted.sort_by(|a, b| {
                let a_str = a.0.as_str().unwrap_or("");
                let b_str = b.0.as_str().unwrap_or("");
                let ord_a = json_key_order(a_str, ctx);
                let ord_b = json_key_order(b_str, ctx);
                ord_a.cmp(&ord_b).then_with(|| a_str.cmp(b_str))
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
