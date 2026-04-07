//! List mock definitions

use crate::ui;
use anyhow::Context;
use mockpit_engine::MockRegistry;

pub async fn list_mocks(collection_filter: Option<String>, verbose: bool) -> anyhow::Result<()> {
    let collections_dir =
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());

    let spinner = ui::spinner(&format!(
        "Loading mocks from {}...",
        ui::path(&collections_dir)
    ));

    let registry = MockRegistry::new();
    let count = registry
        .load_from_directory(&collections_dir)
        .await
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to load mocks")?;

    spinner.finish_and_clear();

    if count == 0 {
        println!(
            "{}",
            ui::warning(&format!(
                "No mock definitions found in {}",
                ui::path(&collections_dir)
            ))
        );
        return Ok(());
    }

    println!(
        "{}",
        ui::success(&format!("Loaded {} mock definition(s)", ui::number(count)))
    );
    println!();

    let mocks = registry.get_all_mocks();

    if mocks.is_empty() {
        println!("{}", ui::info("No mocks loaded"));
        return Ok(());
    }

    // Create table
    let mut table = ui::table();
    table.set_header(vec![
        ui::table_header("Mock ID"),
        ui::table_header("Priority"),
        ui::table_header("Methods"),
        ui::table_header("URL Patterns"),
        ui::table_header("Status"),
    ]);

    let mut filtered_count = 0;
    for mock_def in &mocks {
        // Apply collection filter if provided
        if let Some(ref filter) = collection_filter {
            if !mock_def.id.contains(filter) {
                continue;
            }
        }

        filtered_count += 1;

        let patterns = if mock_def.request.url_patterns.is_empty() {
            "ANY".to_string()
        } else {
            mock_def
                .request
                .url_patterns
                .iter()
                .take(2)
                .map(|p| format!("{p:?}"))
                .collect::<Vec<_>>()
                .join("\n")
        };

        table.add_row(vec![
            ui::table_emphasis_cell(&mock_def.id),
            ui::table_number_cell(mock_def.priority),
            mock_def
                .request
                .methods
                .iter()
                .map(|m: &axum::http::Method| m.to_string())
                .collect::<Vec<_>>()
                .join(", ")
                .into(),
            patterns.into(),
            ui::table_number_cell(mock_def.response.status),
        ]);

        if verbose {
            // Show additional details in verbose mode
            if !mock_def.request.header_matchers.is_empty() || mock_def.response.delay.is_some() {
                let mut details = Vec::new();
                if !mock_def.request.header_matchers.is_empty() {
                    details.push(format!(
                        "Headers: {}",
                        mock_def.request.header_matchers.len()
                    ));
                }
                if let Some(ref delay) = mock_def.response.delay {
                    details.push(format!("Delay: {delay:?}"));
                }
                table.add_row(vec![
                    ui::table_dim_cell(&format!("  |-- {}", details.join(", "))),
                    "".into(),
                    "".into(),
                    "".into(),
                    "".into(),
                ]);
            }
        }
    }

    println!("{table}");
    println!();
    println!("{}", ui::kv("Total", &ui::number(filtered_count)));

    Ok(())
}
