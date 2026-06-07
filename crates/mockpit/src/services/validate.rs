//! Mock validation service — validate mock configuration files.

use crate::engine::{MockValidator, ValidationResult};
use std::path::PathBuf;

/// Input for mock validation.
#[derive(Debug, Clone)]
pub struct ValidateInput {
    /// Path to a file or directory to validate
    pub path: String,
}

/// Input for validating mock content from a string.
#[derive(Debug, Clone)]
pub struct ValidateContentInput {
    /// Mock configuration content
    pub content: String,
    /// File format: "json", "yaml", or "yml"
    pub file_format: String,
}

/// Validation output.
#[derive(Debug, Clone)]
pub struct ValidateOutput {
    /// Results per file
    pub results: Vec<ValidationResult>,
    /// Total error count
    pub total_errors: usize,
    /// Total warning count
    pub total_warnings: usize,
    /// Whether all files are valid (no errors)
    pub is_valid: bool,
}

/// Validate mock files at a path (file or directory).
pub async fn validate(input: ValidateInput) -> Result<ValidateOutput, crate::MockpitError> {
    let path = PathBuf::from(&input.path);
    crate::mp_ensure!(path.exists(), "Path does not exist: {}", input.path);

    let validator = MockValidator::new();

    let results = if path.is_file() {
        vec![validator.validate_file(&path).await]
    } else if path.is_dir() {
        validator.validate_directory(&path).await
    } else {
        crate::mp_bail!("Path is neither a file nor a directory: {}", input.path);
    };

    let total_errors: usize = results.iter().map(ValidationResult::error_count).sum();
    let total_warnings: usize = results.iter().map(ValidationResult::warning_count).sum();

    Ok(ValidateOutput {
        is_valid: total_errors == 0,
        total_errors,
        total_warnings,
        results,
    })
}

/// Validate mock content from a string.
pub async fn validate_content(input: ValidateContentInput) -> Result<ValidateOutput, crate::MockpitError> {
    let validator = MockValidator::new();
    let result = validator.validate_content(&input.content, &input.file_format).await;
    let results = vec![result];

    let total_errors: usize = results.iter().map(ValidationResult::error_count).sum();
    let total_warnings: usize = results.iter().map(ValidationResult::warning_count).sum();

    Ok(ValidateOutput {
        is_valid: total_errors == 0,
        total_errors,
        total_warnings,
        results,
    })
}
