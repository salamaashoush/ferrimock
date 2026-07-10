/// Self-update: check for and install new versions from GitHub releases
use anyhow::{Context, Result};
use colored::Colorize;

const REPO: &str = "salamaashoush/ferrimock";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

async fn get_latest_release() -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = reqwest::Client::builder()
        .user_agent("ferrimock-cli")
        .build()?;
    let response = client
        .get(&url)
        .send()
        .await
        .context("Failed to fetch latest release")?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!("No releases found. This project may not have published a release yet.");
    }

    let release: GitHubRelease = response
        .json()
        .await
        .context("Failed to parse release response")?;
    Ok(release)
}

fn parse_version(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}

pub async fn run(check_only: bool) -> Result<()> {
    println!("{} Checking for updates...", "info:".cyan().bold());

    let release = get_latest_release().await?;
    let latest = parse_version(&release.tag_name);

    if latest == CURRENT_VERSION {
        println!(
            "{}  ferrimock {} is already the latest version",
            "ok:".green().bold(),
            CURRENT_VERSION
        );
        return Ok(());
    }

    println!(
        "{}  New version available: {} -> {}",
        "update:".yellow().bold(),
        CURRENT_VERSION.dimmed(),
        latest.green().bold()
    );
    println!("   {}", release.html_url.dimmed());

    if check_only {
        println!("\n   Run {} to update", "ferrimock self-update".green());
        return Ok(());
    }

    // Determine install command based on how ferrimock was likely installed
    let cargo_install = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.contains(".cargo")))
        .unwrap_or(false);

    if cargo_install {
        println!("\n{}  Updating via cargo install...", "info:".cyan().bold());
        let status = tokio::process::Command::new("cargo")
            .args(["install", "ferrimock-cli", "--locked"])
            .status()
            .await
            .context("Failed to run cargo install")?;

        if status.success() {
            println!("{}  Updated to ferrimock {}", "ok:".green().bold(), latest);
        } else {
            anyhow::bail!("cargo install failed with exit code: {status}");
        }
    } else {
        // Installed via install script or manual download
        println!(
            "\n{}  To update, run the install script:",
            "info:".cyan().bold()
        );
        println!(
            "   curl -sSf https://raw.githubusercontent.com/{REPO}/main/scripts/install.sh | sh"
        );
        println!("\n   Or download from: {}", release.html_url);
    }

    Ok(())
}
