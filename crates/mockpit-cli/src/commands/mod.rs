//! Mockpit CLI commands: mock management and fake data generation.

mod consolidate;
mod convert;
mod create;
mod dispatch;
mod export;
pub mod fake;
mod format;
mod list;
mod recordings;
mod reload;
mod serve;
mod show;
mod test;
pub mod ui;
mod validate;
mod wizard;

// Re-export the mock command entry point
pub use dispatch::execute;
// Re-export the fake command types and entry point
pub use fake::{FakeAction, FakeCommand};

// ---------------------------------------------------------------------------
// CLI argument types
// ---------------------------------------------------------------------------

use clap::{Args, Subcommand};

/// Mock management subcommand
#[derive(Args, Debug, Clone)]
pub struct MockCommand {
    #[command(subcommand)]
    pub action: MockAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MockAction {
    /// Create a new mock definition
    ///
    /// Create mocks with either quick flags or an interactive wizard.
    ///
    /// Quick mode (with flags):
    ///   mock create "/api/users/:id" -m GET -s 200 --template
    ///
    /// Interactive wizard mode:
    ///   mock create --interactive
    ///   mock create -I
    ///   mock create              # (no URL triggers interactive mode)
    ///
    /// The interactive wizard provides step-by-step guidance for:
    ///   - URL pattern with auto-detection (Express/:id, regex, glob)
    ///   - HTTP method selection (single or multiple)
    ///   - Header/query/body matchers
    ///   - Smart template selection based on endpoint patterns
    ///   - Response configuration (status, content-type, delay)
    ///   - Preview and confirmation before saving
    #[command(visible_alias = "new")]
    Create {
        /// URL pattern to match (omit to start interactive wizard)
        #[arg(value_name = "URL")]
        url: Option<String>,

        /// Output file path (defaults to `mocks/collections/MOCK_ID.yaml`)
        #[arg(short = 'o', long, value_name = "FILE")]
        output: Option<String>,

        /// HTTP method (GET, POST, etc.)
        #[arg(short = 'm', long, value_name = "METHOD", default_value = "GET")]
        method: String,

        /// Response status code
        #[arg(short = 's', long, value_name = "CODE", default_value = "200")]
        status: u16,

        /// Response body (JSON string or @file.json)
        #[arg(short = 'b', long, value_name = "BODY")]
        body: Option<String>,

        /// Use template with fake data
        #[arg(short = 't', long)]
        template: bool,

        /// Mock ID (auto-generated if not provided)
        #[arg(short = 'i', long, value_name = "ID")]
        id: Option<String>,

        /// Mock priority (higher = matched first)
        #[arg(short = 'p', long, value_name = "PRIORITY", default_value = "100")]
        priority: u32,

        /// Collection name/scope
        #[arg(short = 'c', long, value_name = "NAME")]
        collection: Option<String>,

        /// Launch interactive wizard for step-by-step mock creation
        #[arg(short = 'I', long)]
        interactive: bool,
    },

    /// List all loaded mock definitions
    #[command(visible_alias = "ls")]
    List {
        /// Filter by collection name
        #[arg(short = 'c', long, value_name = "NAME")]
        collection: Option<String>,

        /// Show detailed information
        #[arg(short = 'v', long)]
        verbose: bool,
    },

    /// Show details of a specific mock
    #[command(visible_alias = "s")]
    Show {
        /// Mock ID
        #[arg(value_name = "MOCK_ID")]
        mock_id: String,
    },

    /// Test if a request matches any mocks
    ///
    /// Test mock matching with full request simulation including headers, body,
    /// and optionally render the response with fake data.
    ///
    /// Examples:
    ///   # Basic matching test
    ///   mock test -m GET /api/users/123
    ///
    ///   # With rendered response preview
    ///   mock test -m GET /api/users/123 --render
    ///
    ///   # With headers
    ///   mock test -m POST /api/users -H "Content-Type: application/json" -H "Authorization: Bearer token"
    ///
    ///   # With request body
    ///   mock test -m POST /api/users --body '{"name": "John"}'
    ///
    ///   # Debug mode showing why mocks matched/didn't match
    ///   mock test -m GET /api/users/123 --debug
    ///
    ///   # JSON output for programmatic use
    ///   mock test -m GET /api/users/123 --render --json
    #[command(visible_alias = "t")]
    Test {
        /// HTTP method
        #[arg(short = 'm', long, value_name = "METHOD", default_value = "GET")]
        method: String,

        /// Request path
        #[arg(value_name = "PATH")]
        path: String,

        /// Query string (optional)
        #[arg(short = 'q', long, value_name = "QUERY")]
        query: Option<String>,

        /// Request headers (can be used multiple times, format: "Name: Value")
        #[arg(short = 'H', long = "header", value_name = "HEADER", action = clap::ArgAction::Append)]
        headers: Vec<String>,

        /// Request body (JSON string or @file.json)
        #[arg(short = 'b', long, value_name = "BODY")]
        body: Option<String>,

        /// Render the response with fake data (show actual mock output)
        #[arg(short = 'r', long)]
        render: bool,

        /// Debug mode - show verbose matching information for all mocks
        #[arg(short = 'd', long)]
        debug: bool,

        /// Load mocks from a specific file instead of the collections directory
        #[arg(short = 'f', long = "mock-file", value_name = "FILE")]
        mock_file: Option<String>,

        /// Output in JSON format for programmatic use
        #[arg(short = 'j', long)]
        json: bool,
    },

    /// Reload mock collections from disk
    #[command(visible_alias = "r")]
    Reload {
        /// Mock collections directory
        #[arg(short = 'd', long, value_name = "DIR")]
        dir: Option<String>,
    },

    /// List recordings
    #[command(visible_alias = "rec")]
    Recordings {
        /// Recordings directory
        #[arg(short = 'd', long, value_name = "DIR")]
        dir: Option<String>,
    },

    /// Validate mock configuration files
    #[command(visible_alias = "v")]
    Validate {
        /// Mock collections directory or specific file path (defaults to mocks/collections)
        #[arg(value_name = "PATH")]
        path: Option<String>,

        /// Output format (text for human-readable, json for machine-readable)
        #[arg(short = 'f', long, value_parser = ["text", "json"], default_value = "text")]
        format: String,

        /// Read from stdin instead of a file (requires --file-format)
        #[arg(long)]
        stdin: bool,

        /// File format for stdin input (json, yaml, yml)
        #[arg(long, value_name = "FORMAT", value_parser = ["json", "yaml", "yml"], requires = "stdin")]
        file_format: Option<String>,
    },

    /// Format mock configuration files
    ///
    /// Normalize mock files with consistent formatting: sorted keys, aligned values,
    /// and standard field ordering (id, priority, enabled, scope, match, response, request).
    ///
    /// Examples:
    ///   mock format mocks/collections/
    ///   mock format mocks/api.yaml
    ///   mock format --check mocks/    # Check without modifying (exit 1 if unformatted)
    ///   cat mock.yaml | mock format --stdin --file-format yaml
    #[command(visible_alias = "fmt")]
    Format {
        /// Mock collections directory or specific file path (defaults to mocks/collections)
        #[arg(value_name = "PATH")]
        path: Option<String>,

        /// Check formatting without modifying files (exit 1 if any file would change)
        #[arg(long)]
        check: bool,

        /// Read from stdin and write formatted output to stdout (requires --file-format)
        #[arg(long)]
        stdin: bool,

        /// File format for stdin input (json, yaml, yml)
        #[arg(long, value_name = "FORMAT", value_parser = ["json", "yaml", "yml"], requires = "stdin")]
        file_format: Option<String>,
    },

    /// Convert HAR file to mock collection
    ///
    /// By default, produces clean replay-ready mocks: normalizes absolute URLs
    /// to relative paths, filters domains and static assets, strips
    /// sensitive/infrastructure headers, and removes access_token from query strings.
    #[command(visible_alias = "conv")]
    Convert {
        /// Input HAR file
        #[arg(value_name = "INPUT")]
        input: String,

        /// Output mock collection file
        #[arg(value_name = "OUTPUT")]
        output: String,

        /// Output format: json, yaml
        #[arg(short = 'f', long, value_name = "FORMAT", value_parser = ["json", "yaml"], default_value = "yaml")]
        format: String,

        /// Matching: exact (preserve URLs), pattern (detect IDs/UUIDs)
        #[arg(short = 'm', long, value_name = "STRATEGY", value_parser = ["exact", "pattern"], default_value = "pattern")]
        matching: String,

        /// Interactive pattern editing
        #[arg(short = 'I', long)]
        interactive: bool,

        /// Include OPTIONS preflight
        #[arg(long)]
        preflight: bool,

        /// Include redirect responses (3xx)
        #[arg(long)]
        redirects: bool,

        /// Keep browser headers
        #[arg(long)]
        browser_headers: bool,

        /// Keep absolute URLs (don't normalize to relative paths)
        #[arg(long)]
        absolute_urls: bool,

        /// Only include entries from these domains (comma-separated, e.g. "api.example.com,cdn.example.com").
        /// Subdomains are included automatically. When not set, all domains are included.
        #[arg(long, value_name = "DOMAINS", value_delimiter = ',')]
        domains: Vec<String>,

        /// Include static assets (.js, .css, .png, etc.)
        #[arg(long)]
        static_assets: bool,

        /// Keep sensitive headers (Authorization, Cookie, Set-Cookie)
        #[arg(long)]
        keep_sensitive_headers: bool,

        /// Keep infrastructure headers (date, server, x-envoy-*, alt-svc)
        #[arg(long)]
        keep_infra_headers: bool,

        /// Extract large/binary response bodies to separate files
        #[arg(long)]
        extract_bodies: bool,

        /// Body size threshold in KB for extraction (default: 100)
        #[arg(long, value_name = "KB", default_value = "100")]
        body_threshold_kb: usize,
    },

    /// Export mock collection to HAR format
    #[command(visible_alias = "exp")]
    Export {
        /// Mock collections directory
        #[arg(short = 'd', long, value_name = "DIR")]
        dir: Option<String>,

        /// Output HAR file path
        #[arg(short = 'o', long, value_name = "FILE")]
        output: String,

        /// Filter by collection name
        #[arg(short = 'c', long, value_name = "NAME")]
        collection: Option<String>,
    },

    /// Consolidate and optimize recorded mocks to reduce file size
    ///
    /// Smart consolidation engine that dramatically reduces file size while maintaining
    /// 100% behavioral accuracy. Uses intelligent pattern detection to group similar requests.
    ///
    /// Features (all automatic):
    /// - Pagination pattern detection (page=1,2,3... -> single prefix pattern)
    /// - ID-based path consolidation (/users/123,456... -> regex pattern)
    /// - Smart templates with dynamic fake data generators
    ///
    /// Examples:
    ///   # Consolidate with all optimizations
    ///   mock consolidate recordings/session-123.json optimized.json
    ///
    ///   # Use templates with fake data for maximum size reduction
    ///   mock consolidate recordings/large.json tiny.json
    ///
    ///   # Disable template generation
    ///   mock consolidate input.json output.json --no-templates
    #[command(visible_alias = "opt")]
    Consolidate {
        /// Input mock collection
        #[arg(value_name = "INPUT")]
        input: String,

        /// Output consolidated mocks
        #[arg(value_name = "OUTPUT")]
        output: String,

        /// Output format: json, yaml
        #[arg(short = 'f', long, value_name = "FORMAT", value_parser = ["json", "yaml"], default_value = "json")]
        format: String,

        /// Min similar requests to form pattern
        #[arg(long, value_name = "N", default_value = "3")]
        min_pattern: usize,

        /// Skip template extraction
        #[arg(long)]
        no_templates: bool,

        /// Show detailed stats
        #[arg(short = 'v', long)]
        verbose: bool,
    },

    /// Start a standalone mock server
    ///
    /// Spin up a lightweight HTTP server that serves mock responses.
    /// Perfect for frontend development without running the full proxy.
    ///
    /// Features:
    /// - Lightweight - no proxy overhead, just mock matching
    /// - Hot reload - watches mock files and reloads on change
    /// - Request logging - shows matched/unmatched requests
    /// - CORS support - for frontend development
    /// - Fake data endpoint - render templates via HTTP
    ///
    /// Examples:
    ///   # Start mock server on default port
    ///   mock serve
    ///
    ///   # With custom port and hot reload
    ///   mock serve --port 3006 --watch
    ///
    ///   # With CORS for frontend development
    ///   mock serve --cors --verbose
    ///
    ///   # From specific mock directory
    ///   mock serve --mocks ./mocks/api/
    ///
    ///   # Load a specific mock file
    ///   mock serve -f mocks/api-users.yaml
    ///
    ///   # Log which mock matched each request
    ///   mock serve --mocks ./mocks/ --log-matches
    #[command(visible_alias = "sv")]
    Serve {
        /// Port to listen on
        #[arg(short = 'p', long, default_value = "3006")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Mock collections directory
        #[arg(short = 'm', long, value_name = "DIR")]
        mocks: Option<String>,

        /// Load a specific mock file (can be combined with --mocks)
        #[arg(short = 'f', long = "mock-file", value_name = "FILE")]
        mock_file: Option<String>,

        /// Watch mock files and hot-reload on change
        #[arg(short = 'w', long)]
        watch: bool,

        /// Enable CORS headers for browser access
        #[arg(long)]
        cors: bool,

        /// Enable template rendering endpoint (POST /__mock/render)
        #[arg(long)]
        enable_render_endpoint: bool,

        /// Log mock match details for every request (mock ID, captures, elapsed time)
        #[arg(long)]
        log_matches: bool,

        /// Enable verbose request logging
        #[arg(short = 'v', long)]
        verbose: bool,

        /// Open browser to server URL
        #[arg(short = 'o', long)]
        open: bool,
    },
}
