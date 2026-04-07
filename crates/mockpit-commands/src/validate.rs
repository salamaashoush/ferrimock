//! Validate mock configuration files

use crate::ui;
use lean_string::LeanString;
use mockpit_engine::{CodeSnippet, ValidationError, ValidationResult, ValidationWarning};
use std::path::PathBuf;

/// Flat diagnostic for JSON output (merges CodeSnippet into top-level fields)
#[derive(serde::Serialize)]
struct JsonDiagnostic {
    mock_id: Option<LeanString>,
    error_type: String,
    code: String,
    message: String,
    line_number: Option<usize>,
    column_start: Option<usize>,
    column_end: Option<usize>,
    suggestion: Option<String>,
}

#[derive(serde::Serialize)]
struct JsonFileResult {
    file_path: Option<String>,
    errors: Vec<JsonDiagnostic>,
    warnings: Vec<JsonDiagnostic>,
}

#[derive(serde::Serialize)]
struct JsonValidationOutput {
    results: Vec<JsonFileResult>,
    total_errors: usize,
    total_warnings: usize,
}

fn error_to_diagnostic(error: &ValidationError) -> JsonDiagnostic {
    let (column_start, column_end) = extract_columns(error.snippet.as_ref());
    JsonDiagnostic {
        mock_id: error.mock_id.clone(),
        error_type: format!("{:?}", error.error_type),
        code: error.error_type.code(),
        message: error.message.clone(),
        line_number: error.line_number,
        column_start,
        column_end,
        suggestion: error.suggestion.clone(),
    }
}

fn warning_to_diagnostic(warning: &ValidationWarning) -> JsonDiagnostic {
    let (column_start, column_end) = extract_columns(warning.snippet.as_ref());
    JsonDiagnostic {
        mock_id: warning.mock_id.clone(),
        error_type: format!("{:?}", warning.warning_type),
        code: warning.warning_type.code(),
        message: warning.message.clone(),
        line_number: warning.line_number,
        column_start,
        column_end,
        suggestion: warning.suggestion.clone(),
    }
}

fn extract_columns(snippet: Option<&CodeSnippet>) -> (Option<usize>, Option<usize>) {
    match snippet {
        Some(s) => (Some(s.highlight_start), Some(s.highlight_end)),
        None => (None, None),
    }
}

fn result_to_json(result: &ValidationResult) -> JsonFileResult {
    JsonFileResult {
        file_path: result.file_path.as_ref().map(|p| p.display().to_string()),
        errors: result.errors.iter().map(error_to_diagnostic).collect(),
        warnings: result.warnings.iter().map(warning_to_diagnostic).collect(),
    }
}

#[allow(clippy::large_futures)]
pub async fn validate_mocks(
    path: Option<String>,
    format: &str,
    stdin: bool,
    file_format: Option<String>,
) -> anyhow::Result<()> {
    // stdin mode: read from stdin, validate, output results
    if stdin {
        return validate_stdin(format, file_format.as_deref()).await;
    }

    let input_path = path.unwrap_or_else(|| {
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
    });

    let path = PathBuf::from(&input_path);
    if !path.exists() {
        anyhow::bail!("Path does not exist: {input_path}");
    }

    let validator = mockpit_engine::MockValidator::new();

    let results = if path.is_file() {
        vec![validator.validate_file(&path).await]
    } else if path.is_dir() {
        validator.validate_directory(&path).await
    } else {
        anyhow::bail!("Path is neither a file nor a directory: {input_path}");
    };

    if format == "json" {
        return output_json(&results);
    }

    output_text(&results, &input_path)
}

/// Read from stdin, validate, output results.
#[allow(clippy::large_futures)]
async fn validate_stdin(output_format: &str, file_format: Option<&str>) -> anyhow::Result<()> {
    use std::io::Read;

    let Some(extension @ ("json" | "yaml" | "yml")) = file_format else {
        anyhow::bail!("Cannot determine format: use --file-format with json, yaml, or yml");
    };

    let mut content = String::new();
    std::io::stdin().read_to_string(&mut content)?;

    let validator = mockpit_engine::MockValidator::new();
    let result = validator.validate_content(&content, extension).await;
    let results = vec![result];

    if output_format == "json" {
        return output_json(&results);
    }

    output_text(&results, "<stdin>")
}

fn output_json(results: &[ValidationResult]) -> anyhow::Result<()> {
    let total_errors: usize = results.iter().map(ValidationResult::error_count).sum();
    let total_warnings: usize = results.iter().map(ValidationResult::warning_count).sum();

    let output = JsonValidationOutput {
        results: results.iter().map(result_to_json).collect(),
        total_errors,
        total_warnings,
    };

    let json = serde_json::to_string(&output)?;
    println!("{json}");

    if total_errors > 0 {
        #[allow(clippy::exit)]
        std::process::exit(1);
    }

    Ok(())
}

fn output_text(results: &[ValidationResult], input_path: &str) -> anyhow::Result<()> {
    use std::io::Write;

    println!(
        "{}",
        ui::action(&format!("Validating mocks in {}", ui::path(input_path)))
    );
    println!();

    let mut total_errors = 0;
    let mut total_warnings = 0;

    for result in results {
        let filename = result
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        print!("{} ", ui::dim(&format!("Validating {filename}...")));
        std::io::stdout().flush()?;

        if !result.has_errors() && !result.has_warnings() {
            println!("{}", ui::success("OK"));
        } else if result.has_errors() {
            println!("{}", ui::error("FAILED"));
            println!();
            let formatted = result.format_all();
            for line in formatted.lines() {
                println!("{line}");
            }
            total_errors += result.error_count();
            total_warnings += result.warning_count();
        } else {
            println!("{}", ui::warning("WARN"));
            println!();
            let formatted = result.format_warnings();
            for line in formatted.lines() {
                println!("{line}");
            }
            total_warnings += result.warning_count();
        }
    }

    println!();
    if total_errors == 0 && total_warnings == 0 {
        println!(
            "{}",
            ui::success(&format!(
                "All {} file(s) are valid",
                ui::number(results.len())
            ))
        );
    } else if total_errors == 0 {
        println!(
            "{}",
            ui::warning(&format!(
                "All {} file(s) are valid with {} warning(s)",
                ui::number(results.len()),
                ui::number(total_warnings)
            ))
        );
    } else {
        println!(
            "{}",
            ui::error(&format!(
                "Validation failed: {} error(s) and {} warning(s) found in {} file(s)",
                ui::number(total_errors),
                ui::number(total_warnings),
                ui::number(results.len())
            ))
        );
        println!();
        #[allow(clippy::exit)]
        std::process::exit(1);
    }

    Ok(())
}
