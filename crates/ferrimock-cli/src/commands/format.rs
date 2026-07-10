//! Mock file formatting

use super::ui;
use ferrimock::config::{MockCollectionConfig, format_body};
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

    let input_path = path.unwrap_or_else(crate::config::mocks_dir);

    let path = PathBuf::from(&input_path);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {input_path}");
    }

    let action = if check { "Checking" } else { "Formatting" };
    println!(
        "{}",
        ui::action(&format!("{} mocks in {}", action, ui::path(&input_path)))
    );
    crate::say!();

    let files = collect_mock_files(&path)?;
    if files.is_empty() {
        crate::say!("{}", ui::warning("No mock files found"));
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

    crate::say!();
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

    let Some(fmt @ ("json" | "yaml" | "yml")) = file_format else {
        anyhow::bail!("Cannot determine format: use --file-format with json, yaml, or yml");
    };
    print!("{}", format_str(&content, fmt)?);
    Ok(())
}

/// Format mock content via the canonical service formatter.
fn format_str(content: &str, file_format: &str) -> anyhow::Result<String> {
    Ok(ferrimock::services::format::format_content(
        ferrimock::services::format::FormatContentInput {
            content: content.to_string(),
            file_format: file_format.to_string(),
        },
    )?)
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
        Some(f @ ("json" | "yaml" | "yml")) => format_str(&original, f)?,
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
