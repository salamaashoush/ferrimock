//! Export mocks to HAR format

use anyhow::Context;
use mockpit_engine::MockRegistry;
use crate::ui;

pub async fn export_to_har(
  dir: Option<String>,
  output: String,
  collection_filter: Option<String>,
) -> anyhow::Result<()> {
  use mockpit_engine::export_mocks_to_har;

  let collections_dir =
    dir.unwrap_or_else(|| std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string()));

  println!("{}", ui::action("Exporting mocks to HAR format"));
  println!();
  println!("{}", ui::kv("Source", &ui::path(&collections_dir)));
  println!("{}", ui::kv("Output", &ui::path(&output)));
  println!();

  // Load all mocks
  let spinner = ui::spinner("Loading mocks...");
  let registry = MockRegistry::new();
  let count = registry
    .load_from_directory(&collections_dir)
    .await
    .map_err(|e| anyhow::anyhow!(e))
    .context("Failed to load mocks")?;
  spinner.finish_and_clear();

  println!(
    "{}",
    ui::success(&format!("Loaded {} mock definition(s)", ui::number(count)))
  );
  println!();

  // Get mocks and optionally filter by collection
  let mocks = registry.get_all_mocks();
  let filtered_mocks: Vec<_> = if let Some(ref filter) = collection_filter {
    println!(
      "{}",
      ui::info(&format!("Filtering by collection: {}", ui::emphasis(filter)))
    );
    mocks.into_iter().filter(|m| m.id.contains(filter)).collect()
  } else {
    mocks
  };

  if filtered_mocks.is_empty() {
    println!("{}", ui::warning("No mocks found matching the filter"));
    return Ok(());
  }

  println!(
    "{}",
    ui::info(&format!(
      "Exporting {} mock(s) to HAR...",
      ui::number(filtered_mocks.len())
    ))
  );
  println!();

  // Convert to HAR
  let spinner = ui::spinner("Converting to HAR format...");
  let har = export_mocks_to_har(&filtered_mocks).context("Failed to export to HAR")?;
  spinner.finish_and_clear();

  // Write to file
  let content = serde_json::to_string_pretty(&har)?;
  tokio::fs::write(&output, content).await?;

  println!("{}", ui::success("Successfully exported mocks to HAR"));
  println!();
  println!("{}", ui::kv("Output", &ui::path(&output)));
  println!("{}", ui::kv("Entries", &ui::number(filtered_mocks.len())));

  Ok(())
}
