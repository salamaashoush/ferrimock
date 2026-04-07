//! Fake data CLI commands: generation, images, PDFs, templates, and HTTP server.

mod data;
mod generators;
mod image;
mod pdf;
mod preview;
mod server;

use clap::{Args, Subcommand};

/// Generate fake data, images, PDFs, and more
///
/// Access 100+ fake data generators directly from the CLI. Generate test data,
/// placeholder images, PDF documents, and preview templates with fake data.
#[derive(Args, Debug, Clone)]
pub struct FakeCommand {
    #[command(subcommand)]
    pub action: FakeAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum FakeAction {
    /// Generate fake data values (names, emails, UUIDs, etc.)
    #[command(visible_alias = "d")]
    Data {
        /// Type of fake data to generate
        #[arg(value_name = "TYPE")]
        generator: String,
        /// Number of values to generate
        #[arg(short = 'n', long, default_value = "1")]
        count: usize,
        /// Minimum value (for numeric generators like price, number)
        #[arg(long)]
        min: Option<f64>,
        /// Maximum value (for numeric generators like price, number)
        #[arg(long)]
        max: Option<f64>,
        /// Word count (for sentence, paragraph generators)
        #[arg(short = 'w', long)]
        words: Option<usize>,
        /// Length (for alphanumeric, token generators)
        #[arg(short = 'l', long)]
        length: Option<usize>,
        /// Output format: text, json, csv
        #[arg(short = 'f', long, default_value = "text")]
        format: String,
        /// Copy result to clipboard
        #[arg(short = 'c', long)]
        copy: bool,
        /// List available generators in a category
        #[arg(long)]
        #[allow(clippy::option_option)]
        list: Option<Option<String>>,
    },

    /// Generate fake images (PNG, JPEG, avatars, placeholders)
    #[command(visible_alias = "img")]
    Image {
        /// Type of image: placeholder, avatar, gradient, checkerboard, noise, stripes
        #[arg(value_name = "TYPE", default_value = "placeholder")]
        image_type: String,
        /// Image width in pixels
        #[arg(short = 'W', long, default_value = "200")]
        width: u32,
        /// Image height in pixels
        #[arg(short = 'H', long, default_value = "200")]
        height: u32,
        /// Background color (hex, e.g., "#FF0000")
        #[arg(short = 'b', long)]
        bg_color: Option<String>,
        /// Text color (hex, for placeholder/avatar)
        #[arg(short = 't', long)]
        text_color: Option<String>,
        /// Text to display on image
        #[arg(long)]
        text: Option<String>,
        /// Initials for avatar (e.g., "JS")
        #[arg(short = 'i', long)]
        initials: Option<String>,
        /// Avatar/placeholder size (shorthand for equal width/height)
        #[arg(short = 's', long)]
        size: Option<u32>,
        /// Start color for gradient
        #[arg(long)]
        start: Option<String>,
        /// End color for gradient
        #[arg(long)]
        end: Option<String>,
        /// Direction: horizontal, vertical, diagonal
        #[arg(short = 'd', long, default_value = "horizontal")]
        direction: String,
        /// Image format: png, jpeg
        #[arg(short = 'F', long, default_value = "png")]
        image_format: String,
        /// JPEG quality (1-100)
        #[arg(short = 'q', long, default_value = "85")]
        quality: u8,
        /// Output file path
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// Output as base64 string
        #[arg(long)]
        base64: bool,
        /// Output as data URI
        #[arg(long)]
        data_uri: bool,
        /// Generate colored noise (vs grayscale)
        #[arg(long)]
        colored: bool,
        /// Open generated image in default viewer
        #[arg(long)]
        open: bool,
    },

    /// Generate fake PDF documents
    #[command(visible_alias = "doc")]
    Pdf {
        /// Number of pages
        #[arg(short = 'p', long, default_value = "1")]
        pages: u32,
        /// Custom text content
        #[arg(short = 't', long)]
        text: Option<String>,
        /// Output file path
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// Output as base64 string
        #[arg(long)]
        base64: bool,
        /// Output as data URI
        #[arg(long)]
        data_uri: bool,
        /// Open generated PDF in default viewer
        #[arg(long)]
        open: bool,
    },

    /// List all available fake data generators
    #[command(visible_alias = "ls")]
    List {
        /// Filter by category
        #[arg(short = 'c', long)]
        category: Option<String>,
        /// Search for generators by name
        #[arg(short = 's', long)]
        search: Option<String>,
        /// Show detailed descriptions and examples
        #[arg(short = 'v', long)]
        verbose: bool,
        /// Output format: text, json
        #[arg(short = 'f', long, default_value = "text")]
        format: String,
    },

    /// Preview template rendering with fake data
    #[command(visible_alias = "tpl")]
    Preview {
        /// Template string to render
        #[arg(value_name = "TEMPLATE")]
        template: Option<String>,
        /// Template file to render
        #[arg(short = 'f', long)]
        file: Option<String>,
        /// Context data as JSON
        #[arg(short = 'c', long)]
        context: Option<String>,
        /// Number of times to render
        #[arg(short = 'n', long, default_value = "1")]
        count: usize,
        /// Output format: text, json
        #[arg(short = 'F', long, default_value = "text")]
        format: String,
    },

    /// Start a fake data HTTP server
    #[command(visible_alias = "s")]
    Serve {
        /// Port to listen on
        #[arg(short = 'p', long, default_value = "3005")]
        port: u16,
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Enable CORS headers
        #[arg(long)]
        cors: bool,
        /// Open browser
        #[arg(short = 'o', long)]
        open: bool,
        /// Enable verbose request logging
        #[arg(short = 'v', long)]
        verbose: bool,
    },
}

/// Execute fake command
pub async fn execute(cmd: FakeCommand) -> anyhow::Result<()> {
    match cmd.action {
        FakeAction::Data {
            generator,
            count,
            min,
            max,
            words,
            length,
            format,
            copy,
            list,
        } => {
            if let Some(category) = list {
                data::list_generators_for_category(category.as_deref(), &format)
            } else {
                data::generate_fake_data(&generator, count, min, max, words, length, &format, copy)
            }
        }
        FakeAction::Image {
            image_type,
            width,
            height,
            bg_color,
            text_color,
            text,
            initials,
            size,
            start,
            end,
            direction,
            image_format,
            quality,
            output,
            base64,
            data_uri,
            colored,
            open,
        } => {
            let (w, h) = if let Some(s) = size {
                (s, s)
            } else {
                (width, height)
            };
            image::generate_fake_image(
                &image_type,
                w,
                h,
                bg_color.as_deref(),
                text_color.as_deref(),
                text.as_deref(),
                initials.as_deref(),
                start.as_deref(),
                end.as_deref(),
                &direction,
                &image_format,
                quality,
                output.as_deref(),
                base64,
                data_uri,
                colored,
                open,
            )
        }
        FakeAction::Pdf {
            pages,
            text,
            output,
            base64,
            data_uri,
            open,
        } => pdf::generate_fake_pdf(
            pages,
            text.as_deref(),
            output.as_deref(),
            base64,
            data_uri,
            open,
        ),
        FakeAction::List {
            category,
            search,
            verbose,
            format,
        } => data::list_generators(category.as_deref(), search.as_deref(), verbose, &format),
        FakeAction::Preview {
            template,
            file,
            context,
            count,
            format,
        } => {
            preview::preview_template(
                template.as_deref(),
                file.as_deref(),
                context.as_deref(),
                count,
                &format,
            )
            .await
        }
        FakeAction::Serve {
            port,
            host,
            cors,
            open,
            verbose,
        } => server::serve_fake_data(port, &host, cors, open, verbose).await,
    }
}
