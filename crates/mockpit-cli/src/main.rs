use clap::{Parser, Subcommand};
use mockpit_commands::{FakeCommand, MockCommand};

#[derive(Parser)]
#[command(
    name = "mockpit",
    about = "HTTP mocking tool with templates, recording, consolidation, and GraphQL support",
    version,
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Mock management commands (create, list, test, serve, validate, convert, etc.)
    #[command(visible_alias = "m")]
    Mock(MockCommand),

    /// Fake data generation (data, images, PDFs, templates, HTTP server)
    #[command(visible_alias = "f")]
    Fake(FakeCommand),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.verbose {
        tracing_subscriber::fmt()
            .with_env_filter("mockpit=debug")
            .init();
    }

    match cli.command {
        Command::Mock(cmd) => mockpit_commands::execute(cmd).await,
        Command::Fake(cmd) => mockpit_commands::fake::execute(cmd).await,
    }
}
