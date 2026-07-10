use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsValidateInput {
    pub path: String,
}

#[napi(object, namespace = "services")]
pub struct JsValidateContentInput {
    pub content: String,
    pub file_format: String,
}

#[napi(object, namespace = "services")]
pub struct JsValidationError {
    pub mock_id: Option<String>,
    pub message: String,
    pub error_type: String,
    pub line_number: Option<u32>,
    pub suggestion: Option<String>,
}

#[napi(object, namespace = "services")]
pub struct JsValidateOutput {
    pub is_valid: bool,
    pub total_errors: u32,
    pub total_warnings: u32,
    pub errors: Vec<JsValidationError>,
    pub warnings: Vec<JsValidationError>,
}

#[napi(namespace = "services")]
pub async fn validate(input: JsValidateInput) -> Result<JsValidateOutput> {
    let result =
        ferrimock::services::validate::validate(ferrimock::services::validate::ValidateInput {
            path: input.path,
        })
        .await
        .map_err(|e| Error::from_reason(e.to_string()))?;

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for r in &result.results {
        for e in &r.errors {
            errors.push(JsValidationError {
                mock_id: e.mock_id.as_ref().map(|s| s.to_string()),
                message: e.message.clone(),
                error_type: format!("{:?}", e.error_type),
                line_number: e.line_number.map(|n| n as u32),
                suggestion: e.suggestion.clone(),
            });
        }
        for w in &r.warnings {
            warnings.push(JsValidationError {
                mock_id: w.mock_id.as_ref().map(|s| s.to_string()),
                message: w.message.clone(),
                error_type: format!("{:?}", w.warning_type),
                line_number: w.line_number.map(|n| n as u32),
                suggestion: w.suggestion.clone(),
            });
        }
    }

    Ok(JsValidateOutput {
        is_valid: result.is_valid,
        total_errors: result.total_errors as u32,
        total_warnings: result.total_warnings as u32,
        errors,
        warnings,
    })
}

#[napi(namespace = "services")]
pub async fn validate_content(input: JsValidateContentInput) -> Result<JsValidateOutput> {
    let result = ferrimock::services::validate::validate_content(
        ferrimock::services::validate::ValidateContentInput {
            content: input.content,
            file_format: input.file_format,
        },
    )
    .await
    .map_err(|e| Error::from_reason(e.to_string()))?;

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for r in &result.results {
        for e in &r.errors {
            errors.push(JsValidationError {
                mock_id: e.mock_id.as_ref().map(|s| s.to_string()),
                message: e.message.clone(),
                error_type: format!("{:?}", e.error_type),
                line_number: e.line_number.map(|n| n as u32),
                suggestion: e.suggestion.clone(),
            });
        }
        for w in &r.warnings {
            warnings.push(JsValidationError {
                mock_id: w.mock_id.as_ref().map(|s| s.to_string()),
                message: w.message.clone(),
                error_type: format!("{:?}", w.warning_type),
                line_number: w.line_number.map(|n| n as u32),
                suggestion: w.suggestion.clone(),
            });
        }
    }

    Ok(JsValidateOutput {
        is_valid: result.is_valid,
        total_errors: result.total_errors as u32,
        total_warnings: result.total_warnings as u32,
        errors,
        warnings,
    })
}
