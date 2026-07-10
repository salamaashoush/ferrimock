//! Fake PDF generation service.

/// Input for generating a fake PDF.
#[derive(Debug, Clone)]
pub struct FakePdfInput {
    /// Number of pages
    pub pages: u32,
    /// Custom text content
    pub text: Option<String>,
}

impl Default for FakePdfInput {
    fn default() -> Self {
        Self {
            pages: 1,
            text: None,
        }
    }
}

/// Result of PDF generation.
#[derive(Debug, Clone)]
pub struct FakePdfResult {
    /// PDF data as base64 string
    pub base64: String,
    /// Raw bytes
    pub bytes: Vec<u8>,
}

/// Generate a fake PDF document.
#[allow(clippy::needless_pass_by_value)] // owned input is the service API boundary
pub fn generate(input: FakePdfInput) -> Result<FakePdfResult, crate::FerrimockError> {
    let base64_data = crate::fake_data::fake_pdf(input.text.as_deref(), Some(input.pages));

    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &base64_data)
        .map_err(|e| crate::mp_err!("Failed to decode PDF: {e}"))?;

    Ok(FakePdfResult {
        base64: base64_data,
        bytes,
    })
}
