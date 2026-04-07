//! Fake PDF generation

use crate::ui;
use base64::Engine;

/// Generate a fake PDF document
pub fn generate_fake_pdf(
    pages: u32,
    text: Option<&str>,
    output: Option<&str>,
    as_base64: bool,
    as_data_uri: bool,
    open_file: bool,
) -> anyhow::Result<()> {
    let base64_data = mockpit_fake_data::fake_pdf(text, Some(pages));

    // Output handling
    if let Some(path) = output {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&base64_data)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
        std::fs::write(path, &bytes)?;
        println!("{}", ui::success(&format!("Saved to {}", ui::path(path))));

        if open_file {
            let _ = open::that(path);
        }
    } else if as_data_uri {
        println!("data:application/pdf;base64,{base64_data}");
    } else if as_base64 {
        println!("{base64_data}");
    } else {
        // Default: save to temp file
        let temp_path = std::env::temp_dir().join(format!(
            "fake-document-{}.pdf",
            mockpit_fake_data::fake_short_hash()
        ));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&base64_data)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
        std::fs::write(&temp_path, &bytes)?;
        println!(
            "{}",
            ui::success(&format!(
                "Generated: {}",
                ui::path(&temp_path.to_string_lossy())
            ))
        );

        if open_file {
            let _ = open::that(&temp_path);
        }
    }

    Ok(())
}
