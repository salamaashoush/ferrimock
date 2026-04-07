//! Show mock details

use crate::ui;
use anyhow::Context;
use mockpit_engine::MockRegistry;

pub async fn show_mock(mock_id: &str) -> anyhow::Result<()> {
    let collections_dir =
        std::env::var("MOCKS_DIR").unwrap_or_else(|_| "mocks/collections".to_string());

    let spinner = ui::spinner("Loading mock definition...");
    let registry = MockRegistry::new();
    registry
        .load_from_directory(&collections_dir)
        .await
        .map_err(|e| anyhow::anyhow!(e))
        .context("Failed to load mocks")?;

    let mocks = registry.get_all_mocks();
    let mock_def = mocks
        .iter()
        .find(|m| m.id == mock_id)
        .ok_or_else(|| anyhow::anyhow!("Mock '{mock_id}' not found"))?;

    spinner.finish_and_clear();

    println!(
        "{}",
        ui::header(&format!("Mock Definition: {}", mock_def.id))
    );
    println!();
    println!("{}", ui::kv("Priority", &ui::number(mock_def.priority)));
    println!("{}", ui::kv("Enabled", &mock_def.enabled.to_string()));
    println!();

    println!("{}", ui::header("Request Matcher"));
    if !mock_def.request.methods.is_empty() {
        let methods: Vec<String> = mock_def
            .request
            .methods
            .iter()
            .map(|m: &axum::http::Method| m.to_string())
            .collect();
        println!("{}", ui::kv("Methods", &methods.join(", ")));
    }

    if !mock_def.request.url_patterns.is_empty() {
        println!();
        println!("  URL Patterns:");
        for pattern in &mock_def.request.url_patterns {
            println!("{}", ui::list_item(&format!("{pattern:?}")));
        }
    }

    if !mock_def.request.header_matchers.is_empty() {
        println!();
        println!("  Header Matchers:");
        for header in &mock_def.request.header_matchers {
            println!("{}", ui::list_item(&format!("{header:?}")));
        }
    }

    if let Some(ref body_matcher) = mock_def.request.body_matcher {
        println!("{}", ui::kv("Body Matcher", &format!("{body_matcher:?}")));
    }

    if !mock_def.request.query_matchers.is_empty() {
        println!();
        println!("  Query Matchers:");
        for query_matcher in &mock_def.request.query_matchers {
            println!("{}", ui::list_item(&format!("{query_matcher:?}")));
        }
    }

    println!();
    println!("{}", ui::header("Response Generator"));
    println!(
        "{}",
        ui::kv("Status", &ui::number(mock_def.response.status))
    );
    println!(
        "{}",
        ui::kv("Mode", &format!("{:?}", mock_def.response.mode))
    );

    if !mock_def.response.headers.is_empty() {
        println!();
        println!("  Headers:");
        for (key, value) in &mock_def.response.headers {
            println!("{}", ui::list_item(&format!("{key}: {value:?}")));
        }
    }

    if let Some(ref delay) = mock_def.response.delay {
        println!("{}", ui::kv("Delay", &format!("{delay:?}")));
    }

    println!(
        "{}",
        ui::kv("Body Source", &format!("{:?}", mock_def.response.body))
    );

    Ok(())
}
