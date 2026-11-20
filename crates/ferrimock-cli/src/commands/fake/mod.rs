//! Fake data CLI commands: generation, images, PDFs, templates, and HTTP server.

pub(crate) mod data;
mod generators;
pub(crate) mod image;
pub(crate) mod pdf;
pub(crate) mod preview;
pub(crate) mod server;

use clap::{Args, Subcommand};

/// Generate fake data, images, and PDFs, and preview templates
#[derive(Args, Debug, Clone)]
pub struct FakeCommand {
    #[command(subcommand)]
    pub action: FakeAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum FakeAction {
    /// Print fake values of one type: names, emails, UUIDs, and a hundred more
    #[command(visible_alias = "d")]
    Data(Box<FakeDataArgs>),

    /// Write a placeholder, avatar, gradient, or noise image
    #[command(visible_alias = "img")]
    Image(Box<FakeImageArgs>),

    /// Write a PDF: one of twenty stock documents, or your own blocks
    #[command(visible_alias = "doc")]
    Pdf(Box<FakePdfArgs>),

    /// List the generators, by category
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

    /// Render a template the way a mock response would
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

    /// Serve the generators and template rendering over HTTP
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

#[derive(Args, Debug, Clone)]
pub struct FakeDataArgs {
    /// Type of fake data to generate
    #[arg(value_name = "TYPE")]
    pub generator: String,
    /// Number of values to generate
    #[arg(short = 'n', long, default_value = "1")]
    pub count: usize,
    /// Minimum value (for numeric generators like price, number)
    #[arg(long)]
    pub min: Option<f64>,
    /// Maximum value (for numeric generators like price, number)
    #[arg(long)]
    pub max: Option<f64>,
    /// Word count (for sentence, paragraph generators)
    #[arg(short = 'w', long)]
    pub words: Option<usize>,
    /// Length (for alphanumeric, token generators)
    #[arg(short = 'l', long)]
    pub length: Option<usize>,
    /// Output format: text, json, csv
    #[arg(short = 'f', long, default_value = "text")]
    pub format: String,
    /// Copy result to clipboard
    #[arg(short = 'c', long)]
    pub copy: bool,
    /// List available generators in a category
    #[arg(long)]
    #[allow(clippy::option_option)]
    pub list: Option<Option<String>>,
}

#[derive(Args, Debug, Clone)]
pub struct FakeImageArgs {
    /// Type of image; `fake image --help` lists all 21
    #[arg(value_name = "TYPE", default_value = "placeholder")]
    pub image_type: String,
    /// Image width in pixels
    #[arg(short = 'W', long, default_value = "200")]
    pub width: u32,
    /// Image height in pixels
    #[arg(short = 'H', long, default_value = "200")]
    pub height: u32,
    /// Background color (hex, e.g., "#FF0000")
    #[arg(short = 'b', long)]
    pub bg_color: Option<String>,
    /// Text color (hex, for placeholder/avatar)
    #[arg(short = 't', long)]
    pub text_color: Option<String>,
    /// Text to display on image
    #[arg(long)]
    pub text: Option<String>,
    /// Initials for avatar (e.g., "JS")
    #[arg(short = 'i', long)]
    pub initials: Option<String>,
    /// Avatar/placeholder size (shorthand for equal width/height)
    #[arg(short = 's', long)]
    pub size: Option<u32>,
    /// Start color for gradient
    #[arg(long)]
    pub start: Option<String>,
    /// End color for gradient
    #[arg(long)]
    pub end: Option<String>,
    /// Direction: horizontal, vertical, diagonal
    #[arg(short = 'd', long, default_value = "horizontal")]
    pub direction: String,
    /// Image format: png, jpeg
    #[arg(short = 'F', long, default_value = "png")]
    pub image_format: String,
    /// JPEG quality (1-100)
    #[arg(short = 'q', long, default_value = "85")]
    pub quality: u8,
    /// Output file path
    #[arg(short = 'o', long)]
    pub output: Option<String>,
    /// Output as base64 string
    #[arg(long)]
    pub base64: bool,
    /// Output as data URI
    #[arg(long)]
    pub data_uri: bool,
    /// Generate colored noise (vs grayscale)
    #[arg(long)]
    pub colored: bool,
    /// Detail octaves for `plasma`
    #[arg(long)]
    pub octaves: Option<u32>,
    /// Grid or module count: `qr`, `heatmap`, `identicon`, `scan`
    #[arg(long)]
    pub cells: Option<u32>,
    /// Row count for `heatmap`
    #[arg(long)]
    pub rows: Option<u32>,
    /// Chart shape for `chart`: bar, line, area
    #[arg(long)]
    pub kind: Option<String>,
    /// Data points for `chart`, digits for `barcode`
    #[arg(long)]
    pub points: Option<u32>,
    /// Seed string an `identicon` is derived from
    #[arg(long = "id-seed")]
    pub id_seed: Option<String>,
    /// Dark chrome for `screenshot`
    #[arg(long)]
    pub dark: bool,
    /// How many images to write; needs --output with a %n or an extension
    #[arg(short = 'n', long, default_value = "1")]
    pub count: usize,
    /// Open generated image in default viewer
    #[arg(long)]
    pub open: bool,
}

#[derive(Args, Debug, Clone)]
pub struct FakePdfArgs {
    /// Least number of pages; longer content flows past it
    #[arg(short = 'p', long, default_value = "1")]
    pub pages: u32,
    /// Custom text content, one source line per line
    #[arg(short = 't', long)]
    pub text: Option<String>,
    /// Heading at the top of the document
    #[arg(long)]
    pub title: Option<String>,
    /// Stock document; `fake list --category pdf` lists all twenty
    #[arg(long, default_value = "plain")]
    pub preset: String,
    /// Repeat the preset body to fill --pages rather than padding blanks
    #[arg(long)]
    pub repeat: bool,
    /// Generated prose paragraphs to add
    #[arg(long, default_value = "0")]
    pub paragraphs: usize,
    /// Table to add, as ROWSxCOLS (repeatable)
    #[arg(long, value_name = "ROWSxCOLS")]
    pub table: Vec<String>,
    /// Image to embed, as WIDTHxHEIGHT in pixels (repeatable)
    #[arg(long, value_name = "WxH")]
    pub image: Vec<String>,
    /// Chart to draw, as KIND:POINTS such as bar:8 (repeatable)
    #[arg(long, value_name = "KIND:POINTS")]
    pub chart: Vec<String>,
    /// Bulleted list to add, by item count (repeatable)
    #[arg(long, value_name = "ITEMS")]
    pub list: Vec<usize>,
    /// Label and value block to add, by pair count (repeatable)
    #[arg(long = "kv", value_name = "PAIRS")]
    pub key_values: Vec<usize>,
    /// Tinted callouts to add
    #[arg(long, default_value = "0")]
    pub callouts: usize,
    /// Newspaper-column prose, as COLUMNSxPARAGRAPHS
    #[arg(long, value_name = "COLSxPARAS")]
    pub columns: Option<String>,
    /// Paper: a4, a3, a5, letter, legal, tabloid
    #[arg(long, default_value = "a4")]
    pub page_size: String,
    /// portrait or landscape; unset takes the preset's choice
    #[arg(long)]
    pub orientation: Option<String>,
    /// Body font: helvetica, times, courier
    #[arg(long)]
    pub font: Option<String>,
    /// Accent colour as hex; unset draws one from the seed
    #[arg(long)]
    pub accent: Option<String>,
    /// Page margin in points
    #[arg(long)]
    pub margin: Option<f64>,
    /// Diagonal stamp on every page
    #[arg(long)]
    pub watermark: Option<String>,
    /// Running head; pass an empty string to drop the preset's
    #[arg(long)]
    pub header: Option<String>,
    /// Running foot; pass an empty string to drop the preset's
    #[arg(long)]
    pub footer: Option<String>,
    /// Leave `Page n of m` off the footer
    #[arg(long)]
    pub no_page_numbers: bool,
    /// PDF Info author
    #[arg(long)]
    pub author: Option<String>,
    /// PDF Info subject
    #[arg(long)]
    pub subject: Option<String>,
    /// PDF Info keywords
    #[arg(long)]
    pub keywords: Option<String>,
    /// How many documents to write; needs --output with a %n or an extension
    #[arg(short = 'n', long, default_value = "1")]
    pub count: usize,
    /// Output file path
    #[arg(short = 'o', long)]
    pub output: Option<String>,
    /// Output as base64 string
    #[arg(long)]
    pub base64: bool,
    /// Output as data URI
    #[arg(long)]
    pub data_uri: bool,
    /// Open generated PDF in default viewer
    #[arg(long)]
    pub open: bool,
}

/// Execute fake command
pub async fn execute(cmd: FakeCommand) -> anyhow::Result<()> {
    use crate::ops::fake as ops;
    match cmd.action {
        FakeAction::Data(args) => {
            let FakeDataArgs {
                generator,
                count,
                min,
                max,
                words,
                length,
                format,
                copy,
                list,
            } = *args;
            if let Some(category) = list {
                ops::list_category(category.as_deref(), &format)
            } else {
                ops::data(&ops::Data {
                    generator,
                    count,
                    min,
                    max,
                    words,
                    length,
                    format,
                    copy,
                })
            }
        }
        FakeAction::Image(args) => {
            let FakeImageArgs {
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
                octaves,
                cells,
                rows,
                kind,
                points,
                id_seed,
                dark,
                count,
                open,
            } = *args;
            let (width, height) = size.map_or((width, height), |s| (s, s));
            ops::image(&ops::Image {
                image_type,
                width,
                height,
                bg_color,
                text_color,
                text,
                initials,
                start,
                end,
                direction,
                format: image_format,
                quality,
                output,
                base64,
                data_uri,
                colored,
                octaves,
                cells,
                rows,
                kind,
                points,
                seed: id_seed,
                dark,
                count,
                open,
            })
        }
        FakeAction::Pdf(args) => {
            let FakePdfArgs {
                pages,
                text,
                title,
                preset,
                repeat,
                paragraphs,
                table,
                image,
                chart,
                list,
                key_values,
                callouts,
                columns,
                page_size,
                orientation,
                font,
                accent,
                margin,
                watermark,
                header,
                footer,
                no_page_numbers,
                author,
                subject,
                keywords,
                count,
                output,
                base64,
                data_uri,
                open,
            } = *args;
            ops::pdf(&ops::Pdf {
                pages,
                text,
                title,
                preset,
                repeat,
                paragraphs,
                tables: table,
                images: image,
                charts: chart,
                lists: list,
                key_values,
                callouts,
                columns,
                page_size,
                orientation,
                font,
                accent,
                margin,
                watermark,
                header,
                footer,
                no_page_numbers,
                author,
                subject,
                keywords,
                count,
                output,
                base64,
                data_uri,
                open,
            })
        }
        FakeAction::List {
            category,
            search,
            verbose,
            format,
        } => ops::list(&ops::ListGenerators {
            category,
            search,
            verbose,
            format,
        }),
        FakeAction::Preview {
            template,
            file,
            context,
            count,
            format,
        } => {
            ops::preview(ops::Preview {
                template,
                file,
                context,
                count,
                format,
            })
            .await
        }
        FakeAction::Serve {
            port,
            host,
            cors,
            open,
            verbose,
        } => {
            ops::serve(ops::Server {
                port,
                host,
                cors,
                open,
                verbose,
            })
            .await
        }
    }
}
