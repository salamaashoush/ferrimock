//! Fake data generation functions

use crate::commands::ui;

use super::generators::{GeneratorInfo, get_all_generators};

/// Generate fake data values
#[allow(clippy::too_many_arguments)]
pub fn generate_fake_data(
    generator: &str,
    count: usize,
    min: Option<f64>,
    max: Option<f64>,
    words: Option<usize>,
    length: Option<usize>,
    format: &str,
    copy: bool,
) -> anyhow::Result<()> {
    let mut results: Vec<String> = Vec::with_capacity(count);

    for _ in 0..count {
        let value = generate_single_value(generator, min, max, words, length)?;
        results.push(value);
    }

    // Format output
    let output = match format {
        "json" => {
            // Parse results that are already valid JSON (e.g. composite generators
            // like `user` and `address` return JSON objects/arrays, and `number`/
            // `boolean` return parseable literals). Fall back to string for plain text.
            let json_values: Vec<serde_json::Value> = results
                .iter()
                .map(|s| serde_json::from_str(s).unwrap_or(serde_json::Value::String(s.clone())))
                .collect();
            serde_json::to_string_pretty(&json_values)?
        }
        "csv" => results.join(","),
        _ => results.join("\n"),
    };

    // Copy to clipboard if requested (feature not currently enabled)
    if copy {
        eprintln!(
            "{}",
            ui::warning("Clipboard support not enabled. Output is printed below.")
        );
    }

    println!("{output}");
    Ok(())
}

/// Generate a single fake value — delegates to the canonical service so the CLI,
/// HTTP templates, and NAPI all share one generator + alias table.
pub fn generate_single_value(
    generator: &str,
    min: Option<f64>,
    max: Option<f64>,
    words: Option<usize>,
    length: Option<usize>,
) -> anyhow::Result<String> {
    mockpit::services::fake_data::generate_single(
        &generator.to_lowercase(),
        min,
        max,
        words,
        length,
    )
    .map_err(|_| {
        anyhow::anyhow!(
            "Unknown generator: '{generator}'. Use 'fake list' to see available generators."
        )
    })
}

/// List generators for a specific category
pub fn list_generators_for_category(category: Option<&str>, format: &str) -> anyhow::Result<()> {
    list_generators(category, None, true, format)
}

/// List all available generators with optional filtering
pub fn list_generators(
    category: Option<&str>,
    search: Option<&str>,
    verbose: bool,
    format: &str,
) -> anyhow::Result<()> {
    let all_generators = get_all_generators();

    // Filter by category and/or search
    let filtered: Vec<_> = all_generators
        .iter()
        .filter(|g| {
            let category_match = category.is_none_or(|c| g.category.eq_ignore_ascii_case(c));
            let search_match = search.is_none_or(|s| {
                let s = s.to_lowercase();
                g.name.to_lowercase().contains(&s)
                    || g.description.to_lowercase().contains(&s)
                    || g.category.to_lowercase().contains(&s)
            });
            category_match && search_match
        })
        .collect();

    if format == "json" {
        let json: Vec<_> = filtered
            .iter()
            .map(|g| {
                serde_json::json!({
                  "name": g.name,
                  "category": g.category,
                  "description": g.description,
                  "example": g.example,
                  "params": g.params
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    crate::say!("{}", ui::header("Available Fake Data Generators"));
    crate::say!();

    // Group by category
    let mut categories: Vec<&str> = filtered.iter().map(|g| g.category).collect();
    categories.sort_unstable();
    categories.dedup();

    for cat in categories {
        crate::say!("{}", ui::emphasis(&cat.to_uppercase()));
        crate::say!();

        let cat_generators: Vec<&&GeneratorInfo> =
            filtered.iter().filter(|g| g.category == cat).collect();

        for g in cat_generators {
            if verbose {
                println!(
                    "  {} {} {}",
                    ui::action(g.name),
                    g.description,
                    ui::dim(g.params)
                );
                println!("    {} {}", ui::dim("Example:"), g.example);
                crate::say!();
            } else {
                println!("  {} {}", ui::action(g.name), g.description);
            }
        }

        if !verbose {
            crate::say!();
        }
    }

    Ok(())
}
