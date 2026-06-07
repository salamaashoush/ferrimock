//! Consolidate and optimize mock collections

use super::ui;
use anyhow::Context;

pub async fn consolidate_mocks(
    input: String,
    output: String,
    format: String,
    min_pattern: usize,
    enable_templates: bool,
    verbose: bool,
) -> anyhow::Result<()> {
    use mockpit::consolidator::{ConsolidatorOptions, MockConsolidator};

    crate::say!("{}", ui::action("Consolidating mock collection"));
    crate::say!();
    crate::say!("{}", ui::kv("Input", &ui::path(&input)));
    crate::say!("{}", ui::kv("Output", &ui::path(&output)));
    crate::say!("{}", ui::kv("Format", &format));
    crate::say!();

    if verbose {
        crate::say!("{}", ui::header("Optimization Settings"));
        println!(
            "{}",
            ui::kv("  Min pattern threshold", &ui::number(min_pattern))
        );
        crate::say!("{}", ui::kv("  Pagination detection", "automatic"));
        crate::say!("{}", ui::kv("  ID pattern detection", "automatic"));
        println!(
            "{}",
            ui::kv("  Template extraction", &enable_templates.to_string())
        );
        crate::say!();
    }

    // Create consolidator with simplified options
    let options = ConsolidatorOptions {
        enable_consolidation: true,
        enable_templates,
        min_pattern_threshold: min_pattern,
        enable_stateful_pagination: true,
        pagination_storage_key_template: "api.{path}.total".to_string(),
    };

    let mut consolidator = MockConsolidator::with_options(options);

    // Load and consolidate
    let spinner = ui::spinner("Loading and analyzing mocks...");
    let consolidated = consolidator
        .consolidate_file(&input)
        .await
        .context("Failed to consolidate mocks")?;
    spinner.finish_and_clear();

    // Print statistics
    crate::say!();
    consolidator.stats().print_report();

    // Save to output file
    let spinner = ui::spinner("Saving consolidated mocks...");

    let content = match format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(&consolidated)?,
        "yaml" | "yml" => {
            serde_yaml::to_string(&consolidated).context("YAML serialization error")?
        }
        _ => {
            anyhow::bail!("Invalid format: {format}. Use 'json' or 'yaml'");
        }
    };

    tokio::fs::write(&output, content).await?;
    spinner.finish_and_clear();

    // Calculate file size savings
    if let (Ok(input_metadata), Ok(output_metadata)) =
        (std::fs::metadata(&input), std::fs::metadata(&output))
    {
        let input_size = input_metadata.len();
        let output_size = output_metadata.len();
        #[allow(clippy::cast_precision_loss)]
        let savings = (1.0 - (output_size as f64 / input_size as f64)) * 100.0;

        crate::say!("{}", ui::success("Successfully consolidated mocks!"));
        crate::say!();
        crate::say!("{}", ui::kv("Output file", &ui::path(&output)));
        crate::say!("{}", ui::kv("Input size", &ui::format_bytes(input_size)));
        crate::say!("{}", ui::kv("Output size", &ui::format_bytes(output_size)));
        crate::say!("{}", ui::kv("Space saved", &format!("{savings:.1}%")));
    } else {
        crate::say!("{}", ui::success("Successfully consolidated mocks!"));
        crate::say!();
        crate::say!("{}", ui::kv("Output file", &ui::path(&output)));
    }

    Ok(())
}
