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

    /// Write a placeholder, avatar, gradient, or noise image
    #[command(visible_alias = "img")]
    Image {
        /// Type of image; `fake image --help` lists all 21
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
        /// Detail octaves for `plasma`
        #[arg(long)]
        octaves: Option<u32>,
        /// Grid or module count: `qr`, `heatmap`, `identicon`, `scan`
        #[arg(long)]
        cells: Option<u32>,
        /// Row count for `heatmap`
        #[arg(long)]
        rows: Option<u32>,
        /// Chart shape for `chart`: bar, line, area
        #[arg(long)]
        kind: Option<String>,
        /// Data points for `chart`, digits for `barcode`
        #[arg(long)]
        points: Option<u32>,
        /// Seed string an `identicon` is derived from
        #[arg(long = "id-seed")]
        id_seed: Option<String>,
        /// Dark chrome for `screenshot`
        #[arg(long)]
        dark: bool,
        /// How many images to write; needs --output with a %n or an extension
        #[arg(short = 'n', long, default_value = "1")]
        count: usize,
        /// Open generated image in default viewer
        #[arg(long)]
        open: bool,
    },

    /// Write a PDF: one of twenty stock documents, or your own blocks
    #[command(visible_alias = "doc")]
    Pdf {
        /// Least number of pages; longer content flows past it
        #[arg(short = 'p', long, default_value = "1")]
        pages: u32,
        /// Custom text content, one source line per line
        #[arg(short = 't', long)]
        text: Option<String>,
        /// Heading at the top of the document
        #[arg(long)]
        title: Option<String>,
        /// Stock document; `fake list --category pdf` lists all twenty
        #[arg(long, default_value = "plain")]
        preset: String,
        /// Repeat the preset body to fill --pages rather than padding blanks
        #[arg(long)]
        repeat: bool,
        /// Generated prose paragraphs to add
        #[arg(long, default_value = "0")]
        paragraphs: usize,
        /// Table to add, as ROWSxCOLS (repeatable)
        #[arg(long, value_name = "ROWSxCOLS")]
        table: Vec<String>,
        /// Image to embed, as WIDTHxHEIGHT in pixels (repeatable)
        #[arg(long, value_name = "WxH")]
        image: Vec<String>,
        /// Chart to draw, as KIND:POINTS such as bar:8 (repeatable)
        #[arg(long, value_name = "KIND:POINTS")]
        chart: Vec<String>,
        /// Bulleted list to add, by item count (repeatable)
        #[arg(long, value_name = "ITEMS")]
        list: Vec<usize>,
        /// Label and value block to add, by pair count (repeatable)
        #[arg(long = "kv", value_name = "PAIRS")]
        key_values: Vec<usize>,
        /// Tinted callouts to add
        #[arg(long, default_value = "0")]
        callouts: usize,
        /// Newspaper-column prose, as COLUMNSxPARAGRAPHS
        #[arg(long, value_name = "COLSxPARAS")]
        columns: Option<String>,
        /// Paper: a4, a3, a5, letter, legal, tabloid
        #[arg(long, default_value = "a4")]
        page_size: String,
        /// portrait or landscape; unset takes the preset's choice
        #[arg(long)]
        orientation: Option<String>,
        /// Body font: helvetica, times, courier
        #[arg(long)]
        font: Option<String>,
        /// Accent colour as hex; unset draws one from the seed
        #[arg(long)]
        accent: Option<String>,
        /// Page margin in points
        #[arg(long)]
        margin: Option<f64>,
        /// Diagonal stamp on every page
        #[arg(long)]
        watermark: Option<String>,
        /// Running head; pass an empty string to drop the preset's
        #[arg(long)]
        header: Option<String>,
        /// Running foot; pass an empty string to drop the preset's
        #[arg(long)]
        footer: Option<String>,
        /// Leave `Page n of m` off the footer
        #[arg(long)]
        no_page_numbers: bool,
        /// PDF Info author
        #[arg(long)]
        author: Option<String>,
        /// PDF Info subject
        #[arg(long)]
        subject: Option<String>,
        /// PDF Info keywords
        #[arg(long)]
        keywords: Option<String>,
        /// How many documents to write; needs --output with a %n or an extension
        #[arg(short = 'n', long, default_value = "1")]
        count: usize,
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

/// Execute fake command
pub async fn execute(cmd: FakeCommand) -> anyhow::Result<()> {
    use crate::ops::fake as ops;
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
            octaves,
            cells,
            rows,
            kind,
            points,
            id_seed,
            dark,
            count,
            open,
        } => {
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
        FakeAction::Pdf {
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
        } => ops::pdf(&ops::Pdf {
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
        }),
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
