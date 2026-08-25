mod self_update;

use anyhow::Result;
use clap::{ArgAction, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint, builder::styling};
use colored::Colorize;
use ferrimock_cli::commands::world::WorldCommand;
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

    /// Seed fake data generation so responses are reproducible across runs
    #[arg(long, global = true, env = "FERRIMOCK_SEED", value_name = "N")]
    seed: Option<u64>,
}

#[derive(Subcommand)]
enum Command {
    /// Mock management: create, list, test, serve, validate, convert, consolidate
    #[command(visible_alias = "m")]
    Mock(MockCommand),

    /// Fake data generation: data, images, PDFs, templates, HTTP server
    #[command(visible_alias = "f")]
    Fake(FakeCommand),

    /// The entity world a mocks directory builds: what is in it, and from where
    #[command(visible_alias = "w")]
    World(WorldCommand),

    /// Reverse-proxy a dev server or backend, answering from mocks first
    ///
    /// Put ferrimock in front of vite, rspack, webpack or an API and point the
    /// browser at it instead. A request that matches a mock is answered
    /// locally; everything else reaches the real thing. One origin covers
    /// both, so there is no CORS to configure and nothing in the application
    /// changes.
    ///
    /// Bodies are never collected on the forwarding path: uploads, bundles,
    /// event streams and WebSockets all pass through frame by frame.
    ///
    /// Examples:
    ///   # In front of a vite dev server
    ///   ferrimock proxy http://localhost:5173
    ///
    ///   # Split the API off to a backend, everything else to vite
    ///   ferrimock proxy -r /api=http://localhost:8080 -r /=http://localhost:5173
    ///
    ///   # With mocks loaded, so /api/users is answered locally
    ///   ferrimock proxy --mocks ./mocks http://localhost:5173
    ///
    ///   # Record everything that reaches the real backend
    ///   ferrimock proxy --record ./recordings https://api.example.com
    ///
    ///   # Serve over TLS so the page gets a secure context
    ///   ferrimock proxy --tls http://localhost:5173
    #[command(visible_alias = "px")]
    Proxy {
        /// Upstream everything is forwarded to, as a catch-all route
        #[arg(value_name = "UPSTREAM", value_hint = ValueHint::Url)]
        upstream: Option<String>,

        /// Route, as <prefix>=<upstream>. Repeatable; the longest prefix wins
        #[arg(short = 'r', long = "route", value_name = "SPEC")]
        routes: Vec<String>,

        /// Port to listen on
        #[arg(short = 'p', long, default_value = "3010")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Mock collections directory
        #[arg(short = 'm', long, value_name = "DIR", value_hint = ValueHint::DirPath)]
        mocks: Option<String>,

        /// Load a specific mock file
        #[arg(short = 'f', long = "mock-file", value_name = "FILE", value_hint = ValueHint::FilePath)]
        mock_file: Option<String>,

        /// Watch mock files and hot-reload on change
        #[arg(short = 'w', long)]
        watch: bool,

        /// Drop the route prefix before forwarding
        #[arg(long)]
        strip_prefix: bool,

        /// Send the browser's Host upstream instead of the target's
        #[arg(long)]
        preserve_host: bool,

        /// Forward everything, matching no mocks at all
        #[arg(long)]
        no_mocks: bool,

        /// Terminate TLS with a generated self-signed certificate
        #[arg(long)]
        tls: bool,

        /// PEM certificate chain to terminate TLS with
        #[arg(long, value_name = "FILE", requires = "tls_key", value_hint = ValueHint::FilePath)]
        tls_cert: Option<std::path::PathBuf>,

        /// PEM private key for --tls-cert
        #[arg(long, value_name = "FILE", requires = "tls_cert", value_hint = ValueHint::FilePath)]
        tls_key: Option<std::path::PathBuf>,

        /// Name to issue the generated certificate for. Repeatable
        #[arg(long = "tls-name", value_name = "NAME")]
        tls_names: Vec<String>,

        /// Record forwarded traffic into this directory
        #[arg(long, value_name = "DIR", value_hint = ValueHint::DirPath)]
        record: Option<String>,

        /// Recording format
        #[arg(long, value_name = "FORMAT", default_value = "json")]
        record_format: String,

        /// Accept upstream certificates that do not validate
        #[arg(long)]
        insecure: bool,

        /// Speak HTTP/1.1 to upstreams, never HTTP/2
        #[arg(long)]
        no_http2: bool,

        /// Seconds to wait for upstream response headers. 0 disables
        #[arg(long, value_name = "SECS", default_value = "60")]
        timeout: u64,

        /// Log one line per request
        #[arg(short = 'v', long)]
        verbose: bool,
    },

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

    if let Some(seed) = cli.seed {
        ferrimock::fake_data::rng::set_global_seed(Some(seed));
    }

    let result: Result<()> = match cli.command {
        Command::Mock(cmd) => ferrimock_cli::commands::execute(cmd).await,
        Command::Fake(cmd) => ferrimock_cli::commands::fake::execute(cmd).await,
        Command::World(cmd) => ferrimock_cli::commands::world::execute(cmd).await,
        Command::Proxy {
            upstream,
            routes,
            port,
            host,
            mocks,
            mock_file,
            watch,
            strip_prefix,
            preserve_host,
            no_mocks,
            tls,
            tls_cert,
            tls_key,
            tls_names,
            record,
            record_format,
            insecure,
            no_http2,
            timeout,
            verbose,
        } => {
            ferrimock_cli::commands::proxy::run(ferrimock_cli::commands::proxy::ProxyOptions {
                upstream,
                routes,
                port,
                host,
                mocks,
                mock_file,
                watch,
                strip_prefix,
                preserve_host,
                no_mocks,
                tls,
                tls_cert,
                tls_key,
                tls_names,
                record,
                record_format,
                insecure,
                no_http2,
                timeout,
                verbose,
            })
            .await
        }
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
