//! Mock file formatting service.

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
pub fn format_path(input: FormatInput) -> Result<FormatOutput, anyhow::Error> {
    let path = PathBuf::from(&input.path);
    anyhow::ensure!(path.exists(), "Path does not exist: {}", input.path);

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
pub fn format_content(input: FormatContentInput) -> Result<String, anyhow::Error> {
    match input.file_format.as_str() {
        "json" => format_json(&input.content),
        "yaml" | "yml" => format_yaml(&input.content),
        other => anyhow::bail!("Unsupported format: {other}"),
    }
}

fn collect_mock_files(dir: &Path) -> Result<Vec<PathBuf>, anyhow::Error> {
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

fn format_single_file(path: &Path, check: bool) -> Result<bool, anyhow::Error> {
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

fn format_json(content: &str) -> Result<String, anyhow::Error> {
    let mut value: serde_json::Value = serde_json::from_str(content)?;
    sort_json_keys(&mut value);
    Ok(serde_json::to_string_pretty(&value)? + "\n")
}

fn format_yaml(content: &str) -> Result<String, anyhow::Error> {
    let value: serde_yaml::Value = serde_yaml::from_str(content)?;
    Ok(serde_yaml::to_string(&value)?)
}

fn sort_json_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                sort_json_keys(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                sort_json_keys(v);
            }
        }
        _ => {}
    }
}
