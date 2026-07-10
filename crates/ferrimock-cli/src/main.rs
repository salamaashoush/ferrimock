mod self_update;

use anyhow::Result;
use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint, builder::styling};
use colored::Colorize;
use ferrimock_cli::commands::{FakeCommand, MockCommand};
use ferrimock_cli::config;
use std::process::ExitCode;

/// Color output mode
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ColorMode {
    Auto,
    Always,
    Never,
}

/// Log verbosity level
#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[derive(Parser)]
#[command(
    name = "ferrimock",
    about = "HTTP mocking tool with templates, recording, consolidation, and GraphQL support",
    long_about = "Ferrimock is a high-performance HTTP mocking framework.\n\n\
        Create, test, and serve mock API responses with template-based generation,\n\
        HAR recording, smart consolidation, and GraphQL support.",
    version = version_string(),
    propagate_version = true,
    arg_required_else_help = true,
    styles = get_styles(),
    after_help = "Use 'ferrimock <command> --help' for more information about a specific command.\n\
        Documentation: https://github.com/salamaashoush/ferrimock"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Set log verbosity level
    #[arg(
        long = "log-level",
        global = true,
        env = "FERRIMOCK_LOG",
        value_enum,
        default_value = "warn"
    )]
    log_level: LogLevel,

    /// Enable verbose logging (shorthand for --log-level=debug)
    #[arg(short, long, global = true, action = ArgAction::SetTrue)]
    verbose: bool,

    /// Suppress all output except errors
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    quiet: bool,

    /// Color output mode
    #[arg(
        long,
        global = true,
        env = "FERRIMOCK_COLOR",
        value_enum,
        default_value = "auto"
    )]
    color: ColorMode,

    /// Path to configuration file
    #[arg(
        long,
        global = true,
        env = "FERRIMOCK_CONFIG",
        value_hint = ValueHint::FilePath
    )]
    config: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Mock management: create, list, test, serve, validate, convert, consolidate
    #[command(visible_alias = "m")]
    Mock(MockCommand),

    /// Fake data generation: data, images, PDFs, templates, HTTP server
    #[command(visible_alias = "f")]
    Fake(FakeCommand),

    /// Generate shell completions
    #[command(visible_alias = "comp")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Generate man page
    Manpage,

    /// Check for updates and self-update
    #[command(visible_alias = "up")]
    SelfUpdate {
        /// Check for updates without installing
        #[arg(long)]
        check: bool,
    },
}

fn version_string() -> &'static str {
    concat!(
        env!("CARGO_PKG_VERSION"),
        " (",
        env!("TARGET"),
        ", ",
        env!("PROFILE"),
        ")"
    )
}

fn get_styles() -> styling::Styles {
    styling::Styles::styled()
        .header(styling::AnsiColor::Cyan.on_default().bold())
        .usage(styling::AnsiColor::Cyan.on_default().bold())
        .literal(styling::AnsiColor::Green.on_default().bold())
        .placeholder(styling::AnsiColor::Yellow.on_default())
        .valid(styling::AnsiColor::Green.on_default())
        .invalid(styling::AnsiColor::Red.on_default())
        .error(styling::AnsiColor::Red.on_default().bold())
}

fn setup_logging(cli: &Cli) {
    let level = if cli.quiet {
        "error"
    } else if cli.verbose {
        "debug"
    } else {
        match cli.log_level {
            LogLevel::Error => "error",
            LogLevel::Warn => "warn",
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    };

    let filter = format!("ferrimock={level},ferrimock_cli={level}");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&filter)),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

fn setup_color(mode: ColorMode) {
    match mode {
        ColorMode::Always => colored::control::set_override(true),
        ColorMode::Never => colored::control::set_override(false),
        ColorMode::Auto => {
            // Respect NO_COLOR env var (https://no-color.org/)
            if std::env::var("NO_COLOR").is_ok() {
                colored::control::set_override(false);
            }
        }
    }
}

fn print_error(err: &anyhow::Error) {
    eprintln!("{} {err}", "error:".red().bold());
    let mut source = err.source();
    while let Some(cause) = source {
        eprintln!("  {} {cause}", "caused by:".dimmed());
        source = std::error::Error::source(cause);
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    setup_color(cli.color);
    setup_logging(&cli);

    config::set_quiet(cli.quiet);
    config::init(config::load_config(cli.config.as_deref()));

    let result: Result<()> = match cli.command {
        Command::Mock(cmd) => ferrimock_cli::commands::execute(cmd).await,
        Command::Fake(cmd) => ferrimock_cli::commands::fake::execute(cmd).await,
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "ferrimock", &mut std::io::stdout());
            Ok(())
        }
        Command::Manpage => {
            let cmd = Cli::command();
            let man = clap_mangen::Man::new(cmd);
            man.render(&mut std::io::stdout())
                .map_err(|e| anyhow::anyhow!("Failed to generate man page: {e}"))
        }
        Command::SelfUpdate { check } => self_update::run(check).await,
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            print_error(&err);
            ExitCode::FAILURE
        }
    }
}
