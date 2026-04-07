//! Reload mock definitions

use crate::ui;
use anyhow::Context;
use mockpit_engine::MockRegistry;

pub async fn reload_mocks(dir: Option<String>) -> anyhow::Result<()> {
    let collections_dir = dir.unwrap_or_else(|| {
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string())
    });

    println!(
        "{}",
        ui::action(&format!(
            "Reloading mocks from {}",
            ui::path(&collections_dir)
        ))
    );
    println!();

    let spinner = ui::spinner("Reloading mocks...");
    let registry = MockRegistry::new();
    let count = registry
        .load_from_directory(&collections_dir)
        .await
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to reload mocks")?;
    spinner.finish_and_clear();

    println!(
        "{}",
        ui::success(&format!(
            "Successfully reloaded {} mock definition(s)",
            ui::number(count)
        ))
    );

    Ok(())
}
