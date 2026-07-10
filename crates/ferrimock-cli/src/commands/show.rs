//! Show mock details

use super::ui;
use anyhow::Context;
use ferrimock::engine::MockRegistry;

pub async fn show_mock(mock_id: &str) -> anyhow::Result<()> {
    let collections_dir = crate::config::mocks_dir();

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
    crate::say!();
    crate::say!("{}", ui::kv("Priority", &ui::number(mock_def.priority)));
    crate::say!("{}", ui::kv("Enabled", &mock_def.enabled.to_string()));
    crate::say!();

    crate::say!("{}", ui::header("Request Matcher"));
    if !mock_def.request.methods.is_empty() {
        let methods: Vec<String> = mock_def
            .request
            .methods
            .iter()
            .map(|m: &axum::http::Method| m.to_string())
            .collect();
        crate::say!("{}", ui::kv("Methods", &methods.join(", ")));
    }

    if !mock_def.request.url_patterns.is_empty() {
        crate::say!();
        println!("  URL Patterns:");
        for pattern in &mock_def.request.url_patterns {
            crate::say!("{}", ui::list_item(&format!("{pattern:?}")));
        }
    }

    if !mock_def.request.header_matchers.is_empty() {
        crate::say!();
        println!("  Header Matchers:");
        for header in &mock_def.request.header_matchers {
            crate::say!("{}", ui::list_item(&format!("{header:?}")));
        }
    }

    if let Some(ref body_matcher) = mock_def.request.body_matcher {
        crate::say!("{}", ui::kv("Body Matcher", &format!("{body_matcher:?}")));
    }

    if !mock_def.request.query_matchers.is_empty() {
        crate::say!();
        println!("  Query Matchers:");
        for query_matcher in &mock_def.request.query_matchers {
            crate::say!("{}", ui::list_item(&format!("{query_matcher:?}")));
        }
    }

    crate::say!();
    crate::say!("{}", ui::header("Response Generator"));
    println!(
        "{}",
        ui::kv("Status", &ui::number(mock_def.response.status))
    );
    println!(
        "{}",
        ui::kv("Mode", &format!("{:?}", mock_def.response.mode))
    );

    if !mock_def.response.headers.is_empty() {
        crate::say!();
        println!("  Headers:");
        for (key, value) in &mock_def.response.headers {
            crate::say!("{}", ui::list_item(&format!("{key}: {value:?}")));
        }
    }

    if let Some(ref delay) = mock_def.response.delay {
        crate::say!("{}", ui::kv("Delay", &format!("{delay:?}")));
    }

    println!(
        "{}",
        ui::kv("Body Source", &format!("{:?}", mock_def.response.body))
    );

    Ok(())
}
