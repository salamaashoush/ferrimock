//! Consolidate and optimize mock collections

use anyhow::Context;
use crate::ui;

pub async fn consolidate_mocks(
  input: String,
  output: String,
  format: String,
  min_pattern: usize,
  enable_templates: bool,
  verbose: bool,
) -> anyhow::Result<()> {
  use mockpit_consolidator::{ConsolidatorOptions, MockConsolidator};

  println!("{}", ui::action("Consolidating mock collection"));
  println!();
  println!("{}", ui::kv("Input", &ui::path(&input)));
  println!("{}", ui::kv("Output", &ui::path(&output)));
  println!("{}", ui::kv("Format", &format));
  println!();

  if verbose {
    println!("{}", ui::header("Optimization Settings"));
    println!("{}", ui::kv("  Min pattern threshold", &ui::number(min_pattern)));
    println!("{}", ui::kv("  Pagination detection", "automatic"));
    println!("{}", ui::kv("  ID pattern detection", "automatic"));
    println!("{}", ui::kv("  Template extraction", &enable_templates.to_string()));
    println!();
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
  println!();
  consolidator.stats().print_report();

  // Save to output file
  let spinner = ui::spinner("Saving consolidated mocks...");

  let content = match format.to_lowercase().as_str() {
    "json" => serde_json::to_string_pretty(&consolidated)?,
    "yaml" | "yml" => serde_yaml::to_string(&consolidated).context("YAML serialization error")?,
    _ => {
      anyhow::bail!("Invalid format: {format}. Use 'json' or 'yaml'");
    },
  };

  tokio::fs::write(&output, content).await?;
  spinner.finish_and_clear();

  // Calculate file size savings
  if let (Ok(input_metadata), Ok(output_metadata)) = (std::fs::metadata(&input), std::fs::metadata(&output)) {
    let input_size = input_metadata.len();
    let output_size = output_metadata.len();
    #[allow(clippy::cast_precision_loss)]
    let savings = (1.0 - (output_size as f64 / input_size as f64)) * 100.0;

    println!("{}", ui::success("Successfully consolidated mocks!"));
    println!();
    println!("{}", ui::kv("Output file", &ui::path(&output)));
    println!("{}", ui::kv("Input size", &ui::format_bytes(input_size)));
    println!("{}", ui::kv("Output size", &ui::format_bytes(output_size)));
    println!("{}", ui::kv("Space saved", &format!("{savings:.1}%")));
  } else {
    println!("{}", ui::success("Successfully consolidated mocks!"));
    println!();
    println!("{}", ui::kv("Output file", &ui::path(&output)));
  }

  Ok(())
}
