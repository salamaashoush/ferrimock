//! Convert HAR files to mock collections

use std::io::Write;

use crate::ui;
use anyhow::Context;

/// Options for HAR-to-mock conversion, mapped from CLI flags
pub struct ConvertHarOptions {
    pub input: String,
    pub output: String,
    pub format: String,
    pub interactive: bool,
    pub exclude_preflight: bool,
    pub exclude_redirects: bool,
    pub strip_browser_headers: bool,
    pub normalize_urls: bool,
    pub filter_non_box_domains: bool,
    pub exclude_static_assets: bool,
    pub strip_sensitive_headers: bool,
    pub strip_infrastructure_headers: bool,
    pub extract_bodies: bool,
    pub body_threshold_kb: usize,
    pub extra_domains: Vec<String>,
}

pub async fn convert_har(opts: ConvertHarOptions) -> anyhow::Result<()> {
    use mockpit_config::{HarLoadOptions, HarLoader, MockCollectionConfig};

    println!("{}", ui::action("Converting HAR file to mock collection"));
    println!();
    println!("{}", ui::kv("Input", &ui::path(&opts.input)));
    println!("{}", ui::kv("Output", &ui::path(&opts.output)));
    println!("{}", ui::kv("Format", &opts.format));
    println!();

    // Derive body_output_dir from output file's parent directory if extraction is enabled
    let body_output_dir = if opts.extract_bodies {
        std::path::Path::new(&opts.output)
            .parent()
            .map(std::path::Path::to_path_buf)
    } else {
        None
    };

    // Create HAR loader with all options
    let options = HarLoadOptions {
        exclude_preflight: opts.exclude_preflight,
        exclude_redirects: opts.exclude_redirects,
        strip_browser_headers: opts.strip_browser_headers,
        normalize_urls: opts.normalize_urls,
        filter_non_box_domains: opts.filter_non_box_domains,
        exclude_static_assets: opts.exclude_static_assets,
        strip_sensitive_headers: opts.strip_sensitive_headers,
        strip_infrastructure_headers: opts.strip_infrastructure_headers,
        strip_sensitive_query_params: opts.strip_sensitive_headers, // tied to sensitive headers flag
        body_output_dir,
        body_size_threshold: opts.body_threshold_kb * 1024,
        extra_box_domains: opts.extra_domains,
    };

    let loader = HarLoader::with_options(options);

    // Load and convert HAR file
    let spinner = ui::spinner("Loading HAR file...");
    let har = {
        let content = tokio::fs::read_to_string(&opts.input).await?;
        serde_json::from_str::<har::Har>(&content).context("Failed to parse HAR file")?
    };
    let mocks = loader
        .convert_har_to_mocks(har)
        .await
        .context("Failed to convert HAR to mocks")?;
    spinner.finish_and_clear();

    println!(
        "{}",
        ui::success(&format!(
            "Loaded {} mock definition(s)",
            ui::number(mocks.len())
        ))
    );
    println!();

    // Interactive mode: show each mock and allow editing
    let final_mocks = if opts.interactive {
        println!(
            "{}",
            ui::info("Interactive mode enabled - Review and edit each mock definition")
        );
        println!();

        let mut edited_mocks = Vec::new();

        for (idx, mock) in mocks.iter().enumerate() {
            ui::divider();
            println!("{}", ui::step(idx + 1, mocks.len(), "Reviewing mock"));
            ui::divider();
            println!();
            println!("{}", ui::kv("ID", &mock.id));
            if let Some(ref match_config) = mock.match_config {
                println!(
                    "{}",
                    ui::kv("Method", &format!("{:?}", match_config.methods))
                );
                println!("{}", ui::kv("Pattern", &format!("{:?}", match_config.urls)));
            }
            if let Some(ref response_config) = mock.response_config {
                if let Some(status) = response_config.status() {
                    println!("{}", ui::kv("Status", &ui::number(status)));
                }
            }
            println!();

            // Ask user what to do
            println!("{}:", ui::emphasis("Options"));
            println!("{}", ui::list_item("[k]eep   - Include this mock"));
            println!("{}", ui::list_item("[s]kip   - Exclude this mock"));
            println!("{}", ui::list_item("[e]dit   - Edit URL pattern"));
            println!("{}", ui::list_item("[q]uit   - Stop and save what we have"));
            println!();

            let mut input = String::new();
            print!("{} ", ui::emphasis("Your choice [k/s/e/q]:"));
            std::io::stdout().flush()?;
            std::io::stdin().read_line(&mut input)?;
            let choice = input.trim().to_lowercase();

            match choice.as_str() {
                "k" | "keep" | "" => {
                    edited_mocks.push(mock.clone());
                    println!("{}", ui::success("Kept"));
                    println!();
                }
                "s" | "skip" => {
                    println!("{}", ui::warning("Skipped"));
                    println!();
                }
                "e" | "edit" => {
                    println!();
                    if let Some(ref match_config) = mock.match_config {
                        if let Some(first_url) = match_config.urls.first() {
                            println!("{}", ui::kv("Current pattern", &format!("{first_url:?}")));
                        }
                    }
                    println!();
                    print!("{} ", ui::emphasis("New pattern:"));
                    std::io::stdout().flush()?;

                    let mut new_pattern = String::new();
                    std::io::stdin().read_line(&mut new_pattern)?;
                    let new_pattern = new_pattern.trim();

                    if new_pattern.is_empty() {
                        edited_mocks.push(mock.clone());
                        println!("{}", ui::info("Kept original pattern"));
                        println!();
                    } else {
                        let mut edited_mock = mock.clone();
                        if let Some(ref mut match_config) = edited_mock.match_config {
                            match_config.urls = vec![new_pattern.to_string()];
                        }
                        edited_mocks.push(edited_mock);
                        println!("{}", ui::success("Updated pattern"));
                        println!();
                    }
                }
                "q" | "quit" => {
                    println!("{}", ui::info("Stopping..."));
                    println!();
                    break;
                }
                _ => {
                    println!("{}", ui::warning("Invalid choice, keeping mock"));
                    println!();
                    edited_mocks.push(mock.clone());
                }
            }
        }

        edited_mocks
    } else {
        mocks
    };

    if final_mocks.is_empty() {
        println!("{}", ui::warning("No mocks to save (all were skipped)"));
        return Ok(());
    }

    // Create mock collection
    let collection = MockCollectionConfig {
        name: Some(format!(
            "Converted from {}",
            std::path::Path::new(&opts.input)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("HAR file")
        )),
        description: Some(format!(
            "Auto-converted from HAR file on {}. Static mocks with exact URL matching.",
            chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        )),
        enabled: true,
        vars: None,
        mocks: final_mocks,
    };

    // Write to output file based on format
    let spinner = ui::spinner("Saving mock collection...");

    let content = match opts.format.to_lowercase().as_str() {
        "json" => serde_json::to_string_pretty(&collection)?,
        "yaml" | "yml" => serde_yaml::to_string(&collection).context("YAML serialization error")?,
        _ => {
            anyhow::bail!("Invalid format: {}. Use 'json' or 'yaml'", opts.format);
        }
    };

    tokio::fs::write(&opts.output, content).await?;
    spinner.finish_and_clear();

    println!(
        "{}",
        ui::success("Successfully converted HAR to mock collection")
    );
    println!();
    println!("{}", ui::kv("Output", &ui::path(&opts.output)));
    println!("{}", ui::kv("Mocks", &ui::number(collection.mocks.len())));

    Ok(())
}
