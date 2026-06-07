//! Mock creation functionality

use std::io::{self, Write};
use std::path::PathBuf;

use super::ui;
use mockpit::services::create::{CreateInput, create};

/// Create a new mock definition
#[allow(clippy::too_many_arguments)]
pub fn create_mock(
    output: Option<String>,
    method: &str,
    url: &str,
    status: u16,
    body: Option<String>,
    template: bool,
    id: Option<String>,
    priority: u32,
    collection: Option<&str>,
    interactive: bool,
) -> anyhow::Result<()> {
    crate::say!("{}", ui::header("Create New Mock"));
    crate::say!();

    // Output format is derived from the output path extension (default yaml).
    let format = output
        .as_deref()
        .and_then(|out| {
            PathBuf::from(out)
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_string)
        })
        .map_or_else(
            || "yaml".to_string(),
            |ext| {
                if ext == "yml" {
                    "yaml".to_string()
                } else {
                    ext
                }
            },
        );
    if format != "yaml" && format != "json" {
        anyhow::bail!("Unsupported format: {format}");
    }

    // Resolve a `@file.json` body reference (a CLI input convenience).
    let resolved_body = match body {
        Some(b) if b.starts_with('@') => Some(
            std::fs::read_to_string(b.trim_start_matches('@'))
                .map_err(|e| anyhow::anyhow!("Failed to read body file: {e}"))?,
        ),
        other => other,
    };

    // Generate the mock via the shared service (single source of truth for ID
    // slugging, template bodies, and JSON/YAML serialization).
    let result = create(CreateInput {
        url: url.to_string(),
        method: method.to_string(),
        status,
        body: resolved_body,
        template,
        id,
        priority,
        collection: collection.map(str::to_string),
        format: format.clone(),
    })?;

    // Resolve the output path (default: <mocks-dir>/<mock-id>.<ext>).
    let output_path = if let Some(out) = output {
        PathBuf::from(out)
    } else {
        let dir = crate::config::mocks_dir();
        PathBuf::from(dir).join(format!("{}.{format}", result.mock_id))
    };

    if interactive {
        crate::say!("{}", ui::kv("Mock ID", &result.mock_id));
        crate::say!("{}", ui::kv("Method", &method.to_uppercase()));
        crate::say!("{}", ui::kv("URL", url));
        crate::say!("{}", ui::kv("Status", &status.to_string()));
        crate::say!("{}", ui::kv("Priority", &priority.to_string()));
        if let Some(coll) = collection {
            crate::say!("{}", ui::kv("Collection", coll));
        }
        crate::say!("{}", ui::kv("Format", &format));
        crate::say!(
            "{}",
            ui::kv("Template", if template { "Yes" } else { "No" })
        );
        crate::say!();
        print!("{} ", ui::emphasis("Continue? (y/N):"));
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            crate::say!("{}", ui::warning("Cancelled"));
            return Ok(());
        }
    }

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create directory: {e}"))?;
    }
    std::fs::write(&output_path, result.content)
        .map_err(|e| anyhow::anyhow!("Failed to write mock file: {e}"))?;

    let output_display = output_path.display().to_string();
    crate::say!(
        "{}",
        ui::success(&format!("Created mock: {}", ui::path(&output_display)))
    );
    crate::say!();
    crate::say!("{}", ui::kv("Mock ID", &result.mock_id));
    crate::say!("{}", ui::kv("Method", &method.to_uppercase()));
    crate::say!("{}", ui::kv("URL Pattern", url));
    crate::say!("{}", ui::kv("Status", &status.to_string()));
    crate::say!("{}", ui::kv("File", &output_display));
    crate::say!();
    crate::say!(
        "{}",
        ui::dim(&format!(
            "Tip: Edit {} to customize the mock",
            ui::path(&output_display)
        ))
    );

    Ok(())
}
