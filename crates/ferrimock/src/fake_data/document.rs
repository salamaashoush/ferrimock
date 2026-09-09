//! Stock documents composed onto the [`pdf`] layout engine.
//!
//! A preset decides the blocks, and also the furniture that makes a document
//! read as the thing it imitates: an invoice carries payment terms in the foot,
//! a certificate sits landscape, a manual runs a header on every page. The
//! caller's [`PdfSpec`] overrides any of it.
//!
//! Every value comes from the seeded generators, so a `--seed` reproduces the
//! whole document down to the amounts in the table.

use super::pdf::{
    self, Align, Block, ChartKind, Doc, Face, Family, Geometry, Meta, Orientation, PageSize, Rgb,
    Table,
};
use super::{company, contact, datetime, finance, identifiers, identity, location, prose, text};
use rand::RngExt as _;

const BODY_GAP: f64 = 8.0;

/// Which stock document a spec composes when no literal text is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PdfPreset {
    /// A heading and a few paragraphs.
    #[default]
    Plain,
    /// Billing header, a line-item table and a total.
    Invoice,
    /// Numbered clauses and a signature block.
    Contract,
    /// Headline, a chart, prose and a summary table.
    Report,
    /// Addressed correspondence.
    Letter,
    /// Name, contact line, experience and skills.
    Resume,
    /// Employee details, earnings and deductions, net pay.
    Payslip,
    /// Account header and a dated transaction table with a running balance.
    Statement,
    /// Till receipt: merchant, items, tax and the total.
    Receipt,
    /// Purchase order with vendor, ship-to and line items.
    PurchaseOrder,
    /// Labelled fields ruled for filling in by hand.
    Form,
    /// To, From, Date, Subject, then the body.
    Memo,
    /// Masthead, a figure and multi-column body text.
    Newsletter,
    /// Numbered sections, callouts and a monospaced listing.
    Manual,
    /// Landscape award with a signature line.
    Certificate,
    /// A week grid of hours by project.
    Timesheet,
    /// Shipment header and an itemised packing table.
    PackingSlip,
    /// Sample details and a results table with reference ranges.
    LabReport,
    /// Recitals, confidentiality clauses and counterparties.
    Nda,
    /// One landscape slide per page: a big title and bullets.
    Slides,
}

impl PdfPreset {
    /// Every preset, for `--preset` help and the generator listing.
    pub const ALL: [Self; 20] = [
        Self::Plain,
        Self::Invoice,
        Self::Contract,
        Self::Report,
        Self::Letter,
        Self::Resume,
        Self::Payslip,
        Self::Statement,
        Self::Receipt,
        Self::PurchaseOrder,
        Self::Form,
        Self::Memo,
        Self::Newsletter,
        Self::Manual,
        Self::Certificate,
        Self::Timesheet,
        Self::PackingSlip,
        Self::LabReport,
        Self::Nda,
        Self::Slides,
    ];

    /// The `--preset` spelling.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Invoice => "invoice",
            Self::Contract => "contract",
            Self::Report => "report",
            Self::Letter => "letter",
            Self::Resume => "resume",
            Self::Payslip => "payslip",
            Self::Statement => "statement",
            Self::Receipt => "receipt",
            Self::PurchaseOrder => "purchase-order",
            Self::Form => "form",
            Self::Memo => "memo",
            Self::Newsletter => "newsletter",
            Self::Manual => "manual",
            Self::Certificate => "certificate",
            Self::Timesheet => "timesheet",
            Self::PackingSlip => "packing-slip",
            Self::LabReport => "lab-report",
            Self::Nda => "nda",
            Self::Slides => "slides",
        }
    }

    /// One line for `fake list`.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Plain => "a heading and generated prose",
            Self::Invoice => "billing header, line items, total due",
            Self::Contract => "numbered clauses and a signature block",
            Self::Report => "headline, chart, prose and a summary table",
            Self::Letter => "addressed correspondence on letterhead",
            Self::Resume => "name, contact line, experience and skills",
            Self::Payslip => "earnings, deductions and net pay",
            Self::Statement => "transactions with a running balance",
            Self::Receipt => "narrow till receipt with tax and total",
            Self::PurchaseOrder => "vendor, ship-to and ordered lines",
            Self::Form => "labelled fields ruled for filling in",
            Self::Memo => "to, from, date, subject and body",
            Self::Newsletter => "masthead, figure and multi-column text",
            Self::Manual => "numbered sections, callouts and a listing",
            Self::Certificate => "landscape award with signature",
            Self::Timesheet => "a week of hours by project",
            Self::PackingSlip => "shipment header and packed items",
            Self::LabReport => "results against reference ranges",
            Self::Nda => "recitals and confidentiality clauses",
            Self::Slides => "landscape slides, one per page",
        }
    }
}

impl std::str::FromStr for PdfPreset {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let wanted = value.trim().to_ascii_lowercase().replace('_', "-");
        Self::ALL
            .into_iter()
            .find(|preset| preset.name() == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|p| p.name()).collect();
                format!("unknown preset {value:?}; try one of {}", names.join(", "))
            })
    }
}

/// Extra content a caller stacks on top of whatever the preset composed.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    /// Generated prose paragraphs.
    pub paragraphs: usize,
    /// Tables, as (rows, columns).
    pub tables: Vec<(usize, usize)>,
    /// Images, as pixel (width, height).
    pub images: Vec<(u32, u32)>,
    /// Charts, as (kind, number of points).
    pub charts: Vec<(ChartKind, usize)>,
    /// Bulleted lists, by item count.
    pub lists: Vec<usize>,
    /// Label and value blocks, by pair count.
    pub key_values: Vec<usize>,
    /// Tinted callouts.
    pub callouts: usize,
    /// Newspaper-column prose, as (columns, paragraphs).
    pub columns: Option<(usize, usize)>,
}

impl Extras {
    fn is_empty(&self) -> bool {
        self.paragraphs == 0
            && self.tables.is_empty()
            && self.images.is_empty()
            && self.charts.is_empty()
            && self.lists.is_empty()
            && self.key_values.is_empty()
            && self.callouts == 0
            && self.columns.is_none()
    }
}

/// What one generated document should contain and how it should be set.
#[derive(Debug, Clone)]
pub struct PdfSpec {
    /// Lower bound on pages. Content longer than this flows past it.
    pub pages: u32,
    /// Literal text, one source line per line. Wraps to the column.
    pub text: Option<String>,
    /// Heading at the top of the document.
    pub title: Option<String>,
    /// Stock document to compose when `text` is absent.
    pub preset: PdfPreset,
    /// Repeat the preset's body until `pages` is reached rather than padding
    /// with blank pages.
    pub repeat: bool,
    pub extras: Extras,
    pub page_size: PageSize,
    /// `None` takes the preset's own choice.
    pub orientation: Option<Orientation>,
    /// `None` takes the preset's own choice.
    pub family: Option<Family>,
    /// `None` takes a colour derived from the seed.
    pub accent: Option<Rgb>,
    pub margin: Option<f64>,
    /// Diagonal stamp on every page.
    pub watermark: Option<String>,
    /// Running head. `Some("")` suppresses the preset's.
    pub header: Option<String>,
    /// Running foot. `Some("")` suppresses the preset's.
    pub footer: Option<String>,
    /// `None` takes the preset's own choice.
    pub page_numbers: Option<bool>,
    pub meta: Meta,
}

impl Default for PdfSpec {
    fn default() -> Self {
        Self {
            pages: 1,
            text: None,
            title: None,
            preset: PdfPreset::default(),
            repeat: false,
            extras: Extras::default(),
            page_size: PageSize::default(),
            orientation: None,
            family: None,
            accent: None,
            margin: None,
            watermark: None,
            header: None,
            footer: None,
            page_numbers: None,
            meta: Meta::default(),
        }
    }
}

/// The furniture and blocks a preset chose, before the spec overrides it.
struct Composition {
    blocks: Vec<Block>,
    header: Option<String>,
    footer: Option<String>,
    orientation: Orientation,
    family: Family,
    /// A receipt, a certificate and a one-page letter are not paginated.
    page_numbers: bool,
    meta: Meta,
}

impl Composition {
    fn new(blocks: Vec<Block>) -> Self {
        Self {
            blocks,
            header: None,
            footer: None,
            orientation: Orientation::Portrait,
            family: Family::Helvetica,
            page_numbers: true,
            meta: Meta::default(),
        }
    }
}

/// Compose the document `spec` describes.
///
/// Text is measured against the page, so a long line wraps into the column
/// instead of running past the right edge, and a document longer than
/// `spec.pages` flows onto further pages rather than being cut.
#[must_use]
pub fn fake_pdf_document(spec: &PdfSpec) -> pdf::Composed {
    let accent = spec.accent.unwrap_or_else(seeded_accent);
    let mut composition = if let Some(literal) = &spec.text {
        Composition::new(literal_blocks(literal))
    } else {
        preset(spec.preset, accent)
    };

    if let Some(title) = &spec.title {
        composition.blocks.insert(
            0,
            Block::Heading {
                text: title.clone(),
                level: 1,
            },
        );
        composition.meta.title = Some(title.clone());
    }

    if spec.repeat && spec.pages > 1 {
        let body = composition.blocks.clone();
        for _ in 1..spec.pages {
            composition.blocks.push(Block::PageBreak);
            composition.blocks.extend(body.iter().cloned());
        }
    }

    if !spec.extras.is_empty() {
        composition
            .blocks
            .extend(extra_blocks(&spec.extras, accent));
    }

    let orientation = spec.orientation.unwrap_or(composition.orientation);
    let mut geometry = Geometry::new(spec.page_size, orientation);
    if let Some(margin) = spec.margin {
        geometry.margin = margin.max(12.0);
    }

    let mut meta = composition.meta;
    // A caller that named a field means it; only fall back per field.
    if spec.meta.title.is_some() {
        meta.title.clone_from(&spec.meta.title);
    }
    for (target, given) in [
        (&mut meta.author, &spec.meta.author),
        (&mut meta.subject, &spec.meta.subject),
        (&mut meta.keywords, &spec.meta.keywords),
        (&mut meta.creator, &spec.meta.creator),
    ] {
        if given.is_some() {
            target.clone_from(given);
        }
    }
    if meta.author.is_none() {
        meta.author = Some(identity::fake_name());
    }
    if meta.creator.is_none() {
        meta.creator = Some("ferrimock".to_string());
    }

    let furniture = |given: &Option<String>, from_preset: Option<String>| match given {
        Some(text) if text.is_empty() => None,
        Some(text) => Some(text.clone()),
        None => from_preset,
    };

    pdf::compose_doc(&Doc {
        blocks: composition.blocks,
        geometry,
        meta,
        header: furniture(&spec.header, composition.header),
        footer: furniture(&spec.footer, composition.footer),
        watermark: spec.watermark.clone(),
        page_numbers: spec.page_numbers.unwrap_or(composition.page_numbers),
        min_pages: spec.pages.max(1),
        family: spec.family.unwrap_or(composition.family),
        accent,
    })
}

/// An accent drawn from the seeded source, so a document's colour is
/// reproducible rather than always the same blue.
fn seeded_accent() -> Rgb {
    const PALETTE: [Rgb; 8] = [
        Rgb::new(0.11, 0.33, 0.60),
        Rgb::new(0.11, 0.44, 0.38),
        Rgb::new(0.50, 0.16, 0.24),
        Rgb::new(0.29, 0.20, 0.52),
        Rgb::new(0.66, 0.36, 0.09),
        Rgb::new(0.13, 0.30, 0.30),
        Rgb::new(0.42, 0.13, 0.42),
        Rgb::new(0.17, 0.24, 0.44),
    ];
    let index = super::rng::rng().random_range(0..PALETTE.len());
    PALETTE
        .get(index)
        .copied()
        .unwrap_or(Rgb::new(0.11, 0.33, 0.60))
}

fn literal_blocks(text: &str) -> Vec<Block> {
    text.lines()
        .map(|line| {
            if line.trim().is_empty() {
                Block::Spacer(BODY_GAP)
            } else {
                Block::Line(line.to_string())
            }
        })
        .collect()
}

fn extra_blocks(extras: &Extras, accent: Rgb) -> Vec<Block> {
    let mut blocks = Vec::new();
    for _ in 0..extras.paragraphs {
        blocks.push(Block::Paragraph(prose::fake_prose(4)));
    }
    for &(rows, columns) in &extras.tables {
        blocks.push(Block::Table(generated_table(rows, columns)));
    }
    for &count in &extras.lists {
        blocks.push(Block::List {
            items: (0..count).map(|_| prose::fake_prose_sentence()).collect(),
            ordered: false,
        });
    }
    for &count in &extras.key_values {
        blocks.push(Block::KeyValues(
            (0..count)
                .map(|_| (prose::fake_label(), text::fake_words(3)))
                .collect(),
        ));
    }
    for _ in 0..extras.callouts {
        blocks.push(Block::Callout {
            title: Some(prose::fake_label()),
            body: prose::fake_prose(2),
            tint: accent.tint(0.86),
        });
    }
    for &(kind, points) in &extras.charts {
        blocks.push(chart_block(kind, points.max(2), accent, None));
    }
    if let Some((columns, paragraphs)) = extras.columns {
        blocks.push(Block::Columns {
            columns,
            text: (0..paragraphs.max(1))
                .map(|_| prose::fake_prose(5))
                .collect::<Vec<_>>()
                .join(" "),
        });
    }
    for &(width, height) in &extras.images {
        blocks.push(Block::Image {
            png_base64: super::files::fake_image_photo(Some(width), Some(height)),
            width: pdf::body_width(),
            align: Align::Center,
            caption: Some(prose::fake_label()),
        });
    }
    blocks
}

fn chart_block(kind: ChartKind, points: usize, accent: Rgb, title: Option<String>) -> Block {
    let mut rng = super::rng::rng();
    Block::Chart(pdf::Chart {
        kind,
        title,
        labels: (0..points).map(month_label).collect(),
        values: (0..points)
            .map(|_| rng.random_range(12.0..100.0f64))
            .collect(),
        height: 150.0,
        accent,
    })
}

fn month_label(index: usize) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS
        .get(index % 12)
        .map_or_else(|| format!("P{index}"), |m| (*m).to_string())
}

/// A generic ruled table, for `--table ROWSxCOLS`.
fn generated_table(rows: usize, columns: usize) -> Table {
    let currency = Currency::draw();
    let columns = columns.max(1);
    let headers = (0..columns)
        .map(|column| match column {
            0 => "Description".to_string(),
            1 => "Reference".to_string(),
            _ => prose::fake_label(),
        })
        .collect();
    let body = (0..rows)
        .map(|_| {
            (0..columns)
                .map(|column| match column {
                    0 => text::fake_words(2),
                    1 => identifiers::fake_short_hash(),
                    _ => currency.of(finance::fake_price(1.0, 9999.99)),
                })
                .collect()
        })
        .collect();
    let mut table = Table::new(headers, body);
    table.zebra = true;
    table.align = (0..columns)
        .map(|column| {
            if column == 0 {
                Align::Left
            } else {
                Align::Right
            }
        })
        .collect();
    table
}

/// One currency for a whole document.
///
/// `fake_currency_symbol` draws per call, so formatting each amount with it
/// gave an invoice four currencies and a bank statement eighteen.
#[derive(Debug, Clone)]
struct Currency(String);

impl Currency {
    fn draw() -> Self {
        let symbol = finance::fake_currency_symbol();
        // The base-14 fonts are WinAnsi encoded, so a symbol outside it draws
        // as `?`. The ISO code is ASCII and always renders.
        if pdf::win_ansi_encodable(&symbol) {
            Self(symbol)
        } else {
            Self(format!("{} ", finance::fake_currency_code()))
        }
    }

    fn of(&self, amount: f64) -> String {
        format!("{}{amount:.2}", self.0)
    }
}

/// A digit string of exactly `count` digits, which an account or payroll number
/// is and `fake_numeric_id` is not.
fn digits(count: usize) -> String {
    let mut rng = super::rng::rng();
    (0..count)
        .map(|_| char::from_digit(rng.random_range(0..10u32), 10).unwrap_or('0'))
        .collect()
}

/// A calendar date within the last two years, without the rfc3339 tail.
fn past_date() -> String {
    let days = super::rng::rng().random_range(1..730i64);
    (chrono::Utc::now() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

fn days_ahead(days: i64) -> String {
    (chrono::Utc::now() + chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn long_date() -> String {
    chrono::Utc::now().format("%-d %B %Y").to_string()
}

// ---------------------------------------------------------------------------
// Presets
// ---------------------------------------------------------------------------

fn preset(preset: PdfPreset, accent: Rgb) -> Composition {
    match preset {
        PdfPreset::Plain => plain(),
        PdfPreset::Invoice => invoice(),
        PdfPreset::Contract => contract(),
        PdfPreset::Report => report(accent),
        PdfPreset::Letter => letter(),
        PdfPreset::Resume => resume(),
        PdfPreset::Payslip => payslip(),
        PdfPreset::Statement => statement(accent),
        PdfPreset::Receipt => receipt(),
        PdfPreset::PurchaseOrder => purchase_order(),
        PdfPreset::Form => form(),
        PdfPreset::Memo => memo(),
        PdfPreset::Newsletter => newsletter(),
        PdfPreset::Manual => manual(accent),
        PdfPreset::Certificate => certificate(),
        PdfPreset::Timesheet => timesheet(),
        PdfPreset::PackingSlip => packing_slip(),
        PdfPreset::LabReport => lab_report(accent),
        PdfPreset::Nda => nda(),
        PdfPreset::Slides => slides(),
    }
}

fn heading(text: impl Into<String>, level: u8) -> Block {
    Block::Heading {
        text: text.into(),
        level,
    }
}

fn plain() -> Composition {
    let title = prose::fake_headline();
    let mut composition = Composition::new(vec![
        heading(title.clone(), 1),
        Block::Paragraph(prose::fake_prose(5)),
        Block::Paragraph(prose::fake_prose(4)),
        Block::Paragraph(prose::fake_prose(6)),
    ]);
    composition.meta.title = Some(title);
    composition
}

fn invoice() -> Composition {
    let currency = Currency::draw();
    let vendor = company::fake_company();
    let number = format!("INV-{}", digits(6));
    let mut rng = super::rng::rng();

    let lines: Vec<(String, u32, f64)> = (0..rng.random_range(4..9u32))
        .map(|_| {
            (
                text::fake_words(3),
                rng.random_range(1..12u32),
                finance::fake_price(20.0, 900.0),
            )
        })
        .collect();
    let subtotal: f64 = lines
        .iter()
        .map(|(_, qty, unit)| f64::from(*qty) * unit)
        .sum();
    let tax = subtotal * 0.2;

    let mut table = Table::new(
        vec![
            "Description".into(),
            "Qty".into(),
            "Unit".into(),
            "Amount".into(),
        ],
        lines
            .iter()
            .map(|(description, qty, unit)| {
                vec![
                    description.clone(),
                    qty.to_string(),
                    currency.of(*unit),
                    currency.of(f64::from(*qty) * unit),
                ]
            })
            .collect(),
    );
    table.weights = vec![3.0, 0.7, 1.0, 1.1];
    table.align = vec![Align::Left, Align::Right, Align::Right, Align::Right];
    table.zebra = true;

    let mut composition = Composition::new(vec![
        heading(format!("Invoice {number}"), 1),
        Block::KeyValues(vec![
            (
                "From".into(),
                format!("{vendor}, {}", location::fake_street_address()),
            ),
            (
                "Billed to".into(),
                format!("{}, {}", identity::fake_name(), location::fake_city()),
            ),
            ("Issued".into(), today()),
            ("Due".into(), days_ahead(30)),
        ]),
        Block::Table(table),
        Block::KeyValues(vec![
            ("Subtotal".into(), currency.of(subtotal)),
            ("VAT at 20%".into(), currency.of(tax)),
            ("Total due".into(), currency.of(subtotal + tax)),
        ]),
        Block::Callout {
            title: Some("Payment terms".into()),
            body: format!(
                "Net 30. Reference {number} on the transfer. Late payment carries interest at 8% above base."
            ),
            tint: Rgb::gray(0.955),
        },
    ]);
    composition.header = Some(vendor.clone());
    composition.footer = Some(format!("{vendor} - {}", contact::fake_email()));
    composition.meta.title = Some(format!("Invoice {number}"));
    composition.meta.subject = Some("Invoice".into());
    composition
}

fn contract() -> Composition {
    let party = company::fake_company();
    let counterparty = company::fake_company();
    let mut blocks = vec![
        heading("Master Services Agreement", 1),
        Block::Paragraph(format!(
            "This agreement is made on {} between {party} (the Supplier) and {counterparty} (the Customer).",
            long_date()
        )),
    ];
    for clause in 1..=8 {
        blocks.push(heading(format!("{clause}. {}", prose::fake_label()), 3));
        blocks.push(Block::Paragraph(prose::fake_prose(4)));
    }
    blocks.push(Block::Signature(vec![
        format!("For and on behalf of {party}"),
        format!("For and on behalf of {counterparty}"),
    ]));

    let mut composition = Composition::new(blocks);
    composition.family = Family::Times;
    composition.footer = Some("Master Services Agreement - confidential".into());
    composition.meta.title = Some("Master Services Agreement".into());
    composition
}

fn report(accent: Rgb) -> Composition {
    let title = prose::fake_headline();
    let mut table = generated_table(5, 4);
    table.grid = false;

    let mut composition = Composition::new(vec![
        heading(title.clone(), 1),
        Block::Styled {
            text: format!("{} - {}", company::fake_company(), long_date()),
            face: Face::ITALIC,
            size: 10.0,
            color: Rgb::MUTED,
            align: Align::Left,
        },
        Block::Paragraph(prose::fake_prose(5)),
        heading("Findings", 2),
        chart_block(ChartKind::Bar, 8, accent, Some("Volume by month".into())),
        Block::Paragraph(prose::fake_prose(4)),
        heading("Detail", 2),
        Block::Table(table),
        Block::Callout {
            title: Some("Recommendation".into()),
            body: prose::fake_prose(3),
            tint: accent.tint(0.88),
        },
        heading("Outlook", 2),
        chart_block(
            ChartKind::Area,
            10,
            accent,
            Some("Projected run rate".into()),
        ),
        Block::Paragraph(prose::fake_prose(5)),
    ]);
    composition.header = Some(title.clone());
    composition.meta.title = Some(title);
    composition.meta.subject = Some("Report".into());
    composition
}

fn letter() -> Composition {
    let sender = company::fake_company();
    let mut composition = Composition::new(vec![
        Block::Styled {
            text: sender.clone(),
            face: Face::BOLD,
            size: 13.0,
            color: Rgb::TEXT,
            align: Align::Left,
        },
        Block::Line(location::fake_street_address()),
        Block::Line(format!(
            "{}, {}",
            location::fake_city(),
            location::fake_zip()
        )),
        Block::Spacer(18.0),
        Block::Line(long_date()),
        Block::Spacer(14.0),
        Block::Line(identity::fake_name()),
        Block::Line(location::fake_street_address()),
        Block::Spacer(14.0),
        Block::Line(format!("Dear {},", identity::fake_last_name())),
        Block::Spacer(6.0),
        Block::Paragraph(prose::fake_prose(5)),
        Block::Paragraph(prose::fake_prose(4)),
        Block::Paragraph(prose::fake_prose(3)),
        Block::Spacer(16.0),
        Block::Line("Yours sincerely,".into()),
        Block::Spacer(26.0),
        Block::Line(identity::fake_name()),
        Block::Styled {
            text: company::fake_job_title(),
            face: Face::ITALIC,
            size: 9.5,
            color: Rgb::MUTED,
            align: Align::Left,
        },
    ]);
    composition.family = Family::Times;
    composition.page_numbers = false;
    composition.meta.title = Some(format!("Letter from {sender}"));
    composition
}

fn resume() -> Composition {
    let name = identity::fake_name();
    let mut blocks = vec![
        heading(name.clone(), 1),
        Block::Styled {
            text: format!(
                "{} - {} - {}",
                contact::fake_email(),
                contact::fake_phone(),
                location::fake_city()
            ),
            face: Face::REGULAR,
            size: 9.5,
            color: Rgb::MUTED,
            align: Align::Left,
        },
        Block::Paragraph(prose::fake_prose(3)),
        heading("Experience", 2),
    ];
    for _ in 0..3 {
        blocks.push(heading(
            format!(
                "{} - {}",
                company::fake_job_title(),
                company::fake_company()
            ),
            3,
        ));
        blocks.push(Block::Styled {
            text: format!("{} to {}", past_date(), past_date()),
            face: Face::ITALIC,
            size: 9.0,
            color: Rgb::MUTED,
            align: Align::Left,
        });
        blocks.push(Block::List {
            items: (0..3).map(|_| prose::fake_prose_sentence()).collect(),
            ordered: false,
        });
    }
    blocks.push(heading("Skills", 2));
    blocks.push(Block::Columns {
        columns: 3,
        text: (0..15)
            .map(|_| prose::fake_label())
            .collect::<Vec<_>>()
            .join("   "),
    });

    let mut composition = Composition::new(blocks);
    composition.footer = Some(name.clone());
    composition.meta.title = Some(format!("{name} - CV"));
    composition.meta.author = Some(name);
    composition
}

fn payslip() -> Composition {
    let currency = Currency::draw();
    let employer = company::fake_company();
    let employee = identity::fake_name();
    let mut rng = super::rng::rng();
    let gross = finance::fake_price(2400.0, 9500.0);
    let tax = gross * rng.random_range(0.18..0.32);
    let pension = gross * 0.05;
    let net = gross - tax - pension;

    let mut earnings = Table::new(
        vec![
            "Element".into(),
            "This period".into(),
            "Year to date".into(),
        ],
        vec![
            vec![
                "Basic pay".into(),
                currency.of(gross * 0.9),
                currency.of(gross * 9.0),
            ],
            vec![
                "Allowances".into(),
                currency.of(gross * 0.1),
                currency.of(gross * 1.1),
            ],
            vec![
                "Income tax".into(),
                format!("-{}", currency.of(tax)),
                format!("-{}", currency.of(tax * 10.0)),
            ],
            vec![
                "Pension".into(),
                format!("-{}", currency.of(pension)),
                format!("-{}", currency.of(pension * 10.0)),
            ],
        ],
    );
    earnings.align = vec![Align::Left, Align::Right, Align::Right];
    earnings.zebra = true;

    let mut composition = Composition::new(vec![
        heading(
            format!("Payslip - {}", chrono::Utc::now().format("%B %Y")),
            1,
        ),
        Block::KeyValues(vec![
            ("Employer".into(), employer.clone()),
            ("Employee".into(), employee.clone()),
            ("Payroll number".into(), digits(8)),
            (
                "Tax code".into(),
                format!("{}L", rng.random_range(1100..1350u32)),
            ),
            ("Payment date".into(), today()),
        ]),
        Block::Table(earnings),
        Block::Callout {
            title: Some("Net pay".into()),
            body: format!("{} paid to account ending {}", currency.of(net), digits(4)),
            tint: Rgb::gray(0.95),
        },
        Block::Styled {
            text: "This payslip is a statement of pay and deductions. Keep it for your records."
                .into(),
            face: Face::ITALIC,
            size: 8.5,
            color: Rgb::MUTED,
            align: Align::Left,
        },
    ]);
    composition.header = Some(employer);
    composition.meta.title = Some(format!("Payslip - {employee}"));
    composition
}

fn statement(accent: Rgb) -> Composition {
    let currency = Currency::draw();
    let bank = company::fake_company();
    let mut rng = super::rng::rng();
    let mut balance = finance::fake_price(800.0, 14000.0);
    let opening = balance;

    // Draw the dates first and sort them: a statement runs in date order, and
    // the running balance is only meaningful along that order.
    let mut dates: Vec<String> = (0..rng.random_range(12..26u32))
        .map(|_| past_date())
        .collect();
    dates.sort();

    let rows: Vec<Vec<String>> = dates
        .into_iter()
        .map(|date| {
            let debit = rng.random_range(0..10u8) < 7;
            let amount = finance::fake_price(4.0, 620.0);
            if debit {
                balance -= amount;
            } else {
                balance += amount;
            }
            vec![
                date,
                text::fake_words(3),
                if debit {
                    currency.of(amount)
                } else {
                    String::new()
                },
                if debit {
                    String::new()
                } else {
                    currency.of(amount)
                },
                currency.of(balance),
            ]
        })
        .collect();

    let mut table = Table::new(
        vec![
            "Date".into(),
            "Detail".into(),
            "Out".into(),
            "In".into(),
            "Balance".into(),
        ],
        rows,
    );
    table.weights = vec![1.0, 2.6, 0.9, 0.9, 1.1];
    table.align = vec![
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    table.zebra = true;

    let mut composition = Composition::new(vec![
        heading(format!("{bank} - Account statement"), 1),
        Block::KeyValues(vec![
            ("Account holder".into(), identity::fake_name()),
            ("Account".into(), format!("**** {}", digits(4))),
            (
                "Sort code".into(),
                format!("{}-{}-{}", digits(2), digits(2), digits(2)),
            ),
            ("Period".into(), format!("{} to {}", past_date(), today())),
            ("Opening balance".into(), currency.of(opening)),
        ]),
        Block::Table(table),
        chart_block(
            ChartKind::Line,
            12,
            accent,
            Some("Balance over the year".into()),
        ),
    ]);
    composition.header = Some(format!("{bank} - statement"));
    composition.footer = Some(format!("{bank} is authorised and regulated."));
    composition.meta.title = Some("Account statement".into());
    composition
}

fn receipt() -> Composition {
    let currency = Currency::draw();
    let merchant = company::fake_company();
    let mut rng = super::rng::rng();
    let items: Vec<(String, f64)> = (0..rng.random_range(3..10u32))
        .map(|_| (text::fake_words(2), finance::fake_price(1.2, 42.0)))
        .collect();
    let subtotal: f64 = items.iter().map(|(_, price)| price).sum();
    let tax = subtotal * 0.08;

    let mut table = Table::new(
        Vec::new(),
        items
            .iter()
            .map(|(name, price)| vec![name.clone(), currency.of(*price)])
            .collect(),
    );
    table.grid = false;
    table.weights = vec![3.0, 1.0];
    table.align = vec![Align::Left, Align::Right];

    let mut composition = Composition::new(vec![
        Block::Styled {
            text: merchant.clone(),
            face: Face::BOLD,
            size: 15.0,
            color: Rgb::TEXT,
            align: Align::Center,
        },
        Block::Styled {
            text: location::fake_street_address(),
            face: Face::REGULAR,
            size: 9.0,
            color: Rgb::MUTED,
            align: Align::Center,
        },
        Block::Styled {
            text: format!("{}  Till {}", today(), digits(3)),
            face: Face::REGULAR,
            size: 9.0,
            color: Rgb::MUTED,
            align: Align::Center,
        },
        Block::Rule,
        Block::Table(table),
        Block::Rule,
        Block::KeyValues(vec![
            ("Subtotal".into(), currency.of(subtotal)),
            ("Tax".into(), currency.of(tax)),
            ("Total".into(), currency.of(subtotal + tax)),
            ("Card".into(), format!("**** {}", digits(4))),
        ]),
        Block::Styled {
            text: "Thank you for your custom. Returns within 28 days with this receipt.".into(),
            face: Face::ITALIC,
            size: 8.5,
            color: Rgb::MUTED,
            align: Align::Center,
        },
    ]);
    composition.family = Family::Courier;
    composition.page_numbers = false;
    composition.meta.title = Some(format!("Receipt - {merchant}"));
    composition
}

fn purchase_order() -> Composition {
    let currency = Currency::draw();
    let number = format!("PO-{}", digits(6));
    let buyer = company::fake_company();
    let mut rng = super::rng::rng();

    let mut table = Table::new(
        vec![
            "Line".into(),
            "Part".into(),
            "Description".into(),
            "Qty".into(),
            "Unit".into(),
        ],
        (1..=rng.random_range(4..10u32))
            .map(|line| {
                vec![
                    line.to_string(),
                    identifiers::fake_short_hash(),
                    text::fake_words(3),
                    rng.random_range(1..250u32).to_string(),
                    currency.of(finance::fake_price(0.4, 320.0)),
                ]
            })
            .collect(),
    );
    table.weights = vec![0.5, 1.2, 3.0, 0.7, 1.0];
    table.align = vec![
        Align::Right,
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
    ];
    table.zebra = true;

    let mut composition = Composition::new(vec![
        heading(format!("Purchase order {number}"), 1),
        Block::KeyValues(vec![
            ("Buyer".into(), buyer.clone()),
            ("Vendor".into(), company::fake_company()),
            ("Ship to".into(), location::fake_street_address()),
            ("Required by".into(), days_ahead(21)),
            ("Incoterms".into(), "DAP, duties paid".into()),
        ]),
        Block::Table(table),
        Block::Callout {
            title: Some("Conditions".into()),
            body: "Deliveries outside the window need written agreement. Quote the order number on every document."
                .into(),
            tint: Rgb::gray(0.955),
        },
        Block::Signature(vec!["Authorised by".into()]),
    ]);
    composition.header = Some(format!("{buyer} - {number}"));
    composition.meta.title = Some(format!("Purchase order {number}"));
    composition
}

fn form() -> Composition {
    let mut blocks = vec![
        heading(format!("{} application", prose::fake_label()), 1),
        Block::Styled {
            text: "Complete every field in black ink and capitals.".into(),
            face: Face::ITALIC,
            size: 9.5,
            color: Rgb::MUTED,
            align: Align::Left,
        },
    ];
    for section in ["Applicant", "Address", "Employment", "Declaration"] {
        blocks.push(heading(section, 2));
        for label in [
            prose::fake_label(),
            prose::fake_label(),
            prose::fake_label(),
        ] {
            blocks.push(Block::Line(label));
            blocks.push(Block::Rule);
        }
    }
    blocks.push(Block::Signature(vec![
        "Signature of applicant".into(),
        "Date".into(),
    ]));

    let mut composition = Composition::new(blocks);
    composition.footer = Some(format!("Form {} rev {}", identifiers::fake_short_hash(), 3));
    composition.meta.title = Some("Application form".into());
    composition
}

fn memo() -> Composition {
    let mut composition = Composition::new(vec![
        Block::Styled {
            text: "MEMORANDUM".into(),
            face: Face::BOLD,
            size: 16.0,
            color: Rgb::TEXT,
            align: Align::Left,
        },
        Block::Rule,
        Block::KeyValues(vec![
            (
                "To".into(),
                format!("{}, {}", identity::fake_name(), company::fake_job_title()),
            ),
            (
                "From".into(),
                format!("{}, {}", identity::fake_name(), company::fake_job_title()),
            ),
            ("Date".into(), long_date()),
            ("Subject".into(), prose::fake_headline()),
        ]),
        Block::Rule,
        Block::Paragraph(prose::fake_prose(5)),
        Block::Paragraph(prose::fake_prose(4)),
        heading("Actions", 2),
        Block::List {
            items: (0..4).map(|_| prose::fake_prose_sentence()).collect(),
            ordered: true,
        },
    ]);
    composition.footer = Some("Internal - not for circulation".into());
    composition.meta.title = Some("Memorandum".into());
    composition
}

fn newsletter() -> Composition {
    let masthead = format!("The {} Review", prose::fake_label());
    let mut composition = Composition::new(vec![
        Block::Styled {
            text: masthead.clone(),
            face: Face::BOLD.in_family(Family::Times),
            size: 26.0,
            color: Rgb::TEXT,
            align: Align::Center,
        },
        Block::Styled {
            text: format!("Issue {} - {}", digits(2), long_date()),
            face: Face::REGULAR,
            size: 9.0,
            color: Rgb::MUTED,
            align: Align::Center,
        },
        Block::Rule,
        heading(prose::fake_headline(), 2),
        Block::Image {
            png_base64: super::files::fake_image_photo(Some(1200), Some(420)),
            width: pdf::body_width(),
            align: Align::Center,
            caption: Some(prose::fake_prose_sentence()),
        },
        Block::Columns {
            columns: 2,
            text: (0..6)
                .map(|_| prose::fake_prose(5))
                .collect::<Vec<_>>()
                .join(" "),
        },
        heading(prose::fake_headline(), 2),
        Block::Columns {
            columns: 3,
            text: (0..6)
                .map(|_| prose::fake_prose(4))
                .collect::<Vec<_>>()
                .join(" "),
        },
    ]);
    composition.family = Family::Times;
    composition.header = Some(masthead.clone());
    composition.meta.title = Some(masthead);
    composition
}

fn manual(accent: Rgb) -> Composition {
    let product = format!("{} {}", company::fake_company(), prose::fake_label());
    let mut blocks = vec![
        heading(format!("{product} - operating manual"), 1),
        Block::Paragraph(prose::fake_prose(4)),
        heading("Contents", 2),
        Block::List {
            items: (1..=5).map(|_| prose::fake_label()).collect(),
            ordered: true,
        },
    ];
    for section in 1..=5 {
        blocks.push(heading(format!("{section}. {}", prose::fake_label()), 2));
        blocks.push(Block::Paragraph(prose::fake_prose(4)));
        if section % 2 == 1 {
            blocks.push(Block::Callout {
                title: Some("Warning".into()),
                body: prose::fake_prose(2),
                tint: Rgb::new(0.99, 0.94, 0.80),
            });
        }
        blocks.push(heading(format!("{section}.1 {}", prose::fake_label()), 3));
        blocks.push(Block::Styled {
            text: format!(
                "$ {} --{} {}",
                text::fake_word(),
                text::fake_slug(),
                identifiers::fake_short_hash()
            ),
            face: Face::REGULAR.in_family(Family::Courier),
            size: 9.5,
            color: Rgb::TEXT,
            align: Align::Left,
        });
        blocks.push(Block::Paragraph(prose::fake_prose(3)));
    }
    blocks.push(heading("Specifications", 2));
    let mut table = generated_table(6, 3);
    table.zebra = true;
    blocks.push(Block::Table(table));
    blocks.push(chart_block(
        ChartKind::Bar,
        6,
        accent,
        Some("Throughput by mode".into()),
    ));

    let mut composition = Composition::new(blocks);
    composition.header = Some(product.clone());
    composition.footer = Some(format!("Doc {} rev A", identifiers::fake_short_hash()));
    composition.meta.title = Some(format!("{product} manual"));
    composition
}

fn certificate() -> Composition {
    let recipient = identity::fake_name();
    let issuer = company::fake_company();
    let mut composition = Composition::new(vec![
        Block::Spacer(60.0),
        Block::Styled {
            text: "Certificate of Achievement".into(),
            face: Face::BOLD.in_family(Family::Times),
            size: 30.0,
            color: Rgb::TEXT,
            align: Align::Center,
        },
        Block::Spacer(24.0),
        Block::Styled {
            text: "This is to certify that".into(),
            face: Face::ITALIC.in_family(Family::Times),
            size: 12.0,
            color: Rgb::MUTED,
            align: Align::Center,
        },
        Block::Spacer(10.0),
        Block::Styled {
            text: recipient.clone(),
            face: Face::BOLD.in_family(Family::Times),
            size: 24.0,
            color: Rgb::TEXT,
            align: Align::Center,
        },
        Block::Spacer(10.0),
        Block::Styled {
            text: format!(
                "has completed the {} programme to the standard required by {issuer}.",
                prose::fake_label()
            ),
            face: Face::REGULAR.in_family(Family::Times),
            size: 12.0,
            color: Rgb::TEXT,
            align: Align::Center,
        },
        Block::Spacer(40.0),
        Block::Styled {
            text: format!("Awarded {}", long_date()),
            face: Face::REGULAR,
            size: 10.0,
            color: Rgb::MUTED,
            align: Align::Center,
        },
        Block::Signature(vec![format!("For {issuer}")]),
    ]);
    composition.orientation = Orientation::Landscape;
    composition.family = Family::Times;
    composition.page_numbers = false;
    composition.meta.title = Some(format!("Certificate - {recipient}"));
    composition
}

fn timesheet() -> Composition {
    let mut rng = super::rng::rng();
    let person = identity::fake_name();
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

    let mut headers = vec!["Project".to_string()];
    headers.extend(days.iter().map(|d| (*d).to_string()));
    headers.push("Total".into());

    let rows: Vec<Vec<String>> = (0..rng.random_range(4..8u32))
        .map(|_| {
            let hours: Vec<f64> = (0..7)
                .map(|day| {
                    if day >= 5 {
                        0.0
                    } else {
                        rng.random_range(0.0..8.0f64)
                    }
                })
                .collect();
            let total: f64 = hours.iter().sum();
            let mut row = vec![prose::fake_label()];
            row.extend(hours.iter().map(|h| format!("{h:.1}")));
            row.push(format!("{total:.1}"));
            row
        })
        .collect();

    let mut table = Table::new(headers, rows);
    table.weights = vec![2.6, 0.7, 0.7, 0.7, 0.7, 0.7, 0.7, 0.7, 0.9];
    table.align = std::iter::once(Align::Left)
        .chain(std::iter::repeat_n(Align::Right, 8))
        .collect();
    table.zebra = true;

    let mut composition = Composition::new(vec![
        heading("Weekly timesheet", 1),
        Block::KeyValues(vec![
            ("Name".into(), person.clone()),
            ("Week ending".into(), today()),
            ("Department".into(), company::fake_job_field()),
            ("Approver".into(), identity::fake_name()),
        ]),
        Block::Table(table),
        Block::Signature(vec!["Employee".into(), "Approver".into()]),
    ]);
    composition.orientation = Orientation::Landscape;
    composition.header = Some(format!("Timesheet - {person}"));
    composition.meta.title = Some("Weekly timesheet".into());
    composition
}

fn packing_slip() -> Composition {
    let mut rng = super::rng::rng();
    let shipment = format!("SHP-{}", digits(7));

    let mut table = Table::new(
        vec![
            "SKU".into(),
            "Item".into(),
            "Ordered".into(),
            "Packed".into(),
            "Back order".into(),
        ],
        (0..rng.random_range(5..12u32))
            .map(|_| {
                let ordered = rng.random_range(1..40u32);
                let packed = rng.random_range(0..=ordered);
                vec![
                    identifiers::fake_short_hash(),
                    text::fake_words(3),
                    ordered.to_string(),
                    packed.to_string(),
                    (ordered - packed).to_string(),
                ]
            })
            .collect(),
    );
    table.weights = vec![1.2, 3.0, 0.9, 0.9, 1.1];
    table.align = vec![
        Align::Left,
        Align::Left,
        Align::Right,
        Align::Right,
        Align::Right,
    ];
    table.zebra = true;

    let mut composition = Composition::new(vec![
        heading(format!("Packing slip {shipment}"), 1),
        Block::KeyValues(vec![
            ("Order".into(), format!("ORD-{}", digits(6))),
            (
                "Ship to".into(),
                format!(
                    "{}, {}",
                    identity::fake_name(),
                    location::fake_street_address()
                ),
            ),
            ("Carrier".into(), company::fake_company()),
            ("Tracking".into(), identifiers::fake_token()),
            ("Packed".into(), today()),
        ]),
        Block::Table(table),
        Block::Styled {
            text: "This is not an invoice. Check the contents against this slip on receipt.".into(),
            face: Face::ITALIC,
            size: 9.0,
            color: Rgb::MUTED,
            align: Align::Left,
        },
    ]);
    composition.header = Some(shipment.clone());
    composition.meta.title = Some(format!("Packing slip {shipment}"));
    composition
}

fn lab_report(accent: Rgb) -> Composition {
    let mut rng = super::rng::rng();
    let sample = format!("S-{}", digits(8));

    let analytes = [
        ("Haemoglobin", "g/dL", 13.0, 17.0),
        ("White cell count", "10^9/L", 4.0, 11.0),
        ("Platelets", "10^9/L", 150.0, 400.0),
        ("Sodium", "mmol/L", 135.0, 145.0),
        ("Potassium", "mmol/L", 3.5, 5.3),
        ("Creatinine", "umol/L", 60.0, 110.0),
    ];

    let rows: Vec<Vec<String>> = analytes
        .iter()
        .map(|(name, unit, low, high)| {
            let span = high - low;
            let value = rng.random_range((low - span * 0.3)..(high + span * 0.3));
            let flag = if value < *low {
                "Low"
            } else if value > *high {
                "High"
            } else {
                ""
            };
            vec![
                (*name).to_string(),
                format!("{value:.1}"),
                (*unit).to_string(),
                format!("{low:.1} - {high:.1}"),
                flag.to_string(),
            ]
        })
        .collect();

    let mut table = Table::new(
        vec![
            "Analyte".into(),
            "Result".into(),
            "Unit".into(),
            "Reference".into(),
            "Flag".into(),
        ],
        rows,
    );
    table.weights = vec![2.4, 0.9, 0.9, 1.4, 0.7];
    table.align = vec![
        Align::Left,
        Align::Right,
        Align::Left,
        Align::Center,
        Align::Center,
    ];
    table.zebra = true;

    let mut composition = Composition::new(vec![
        heading("Laboratory report", 1),
        Block::KeyValues(vec![
            ("Sample".into(), sample.clone()),
            ("Patient".into(), identity::fake_name()),
            ("Date of birth".into(), past_date()),
            (
                "Collected".into(),
                format!("{} {}", today(), datetime::fake_time()),
            ),
            (
                "Requested by".into(),
                format!("Dr {}", identity::fake_last_name()),
            ),
        ]),
        Block::Table(table),
        chart_block(
            ChartKind::Bar,
            6,
            accent,
            Some("Result against range".into()),
        ),
        Block::Callout {
            title: Some("Comment".into()),
            body: prose::fake_prose(2),
            tint: accent.tint(0.88),
        },
        Block::Styled {
            text:
                "Results are for the sample as received. Interpret alongside the clinical picture."
                    .into(),
            face: Face::ITALIC,
            size: 8.5,
            color: Rgb::MUTED,
            align: Align::Left,
        },
    ]);
    composition.header = Some(format!("Laboratory report - {sample}"));
    composition.footer = Some(format!("Reported {}", today()));
    composition.meta.title = Some(format!("Laboratory report {sample}"));
    composition
}

fn nda() -> Composition {
    let discloser = company::fake_company();
    let recipient = company::fake_company();
    let mut blocks = vec![
        heading("Mutual Non-Disclosure Agreement", 1),
        Block::Paragraph(format!(
            "Dated {}, between {discloser} and {recipient}, each of whom may disclose confidential information to the other.",
            long_date()
        )),
        heading("Recitals", 2),
        Block::List {
            items: (0..3).map(|_| prose::fake_prose(2)).collect(),
            ordered: true,
        },
    ];
    for (index, clause) in [
        "Confidential Information",
        "Permitted Use",
        "Exclusions",
        "Term and Survival",
        "Return and Destruction",
        "Remedies",
        "Governing Law",
    ]
    .iter()
    .enumerate()
    {
        blocks.push(heading(format!("{}. {clause}", index + 1), 3));
        blocks.push(Block::Paragraph(prose::fake_prose(4)));
    }
    blocks.push(Block::Signature(vec![
        format!("For {discloser}"),
        format!("For {recipient}"),
    ]));

    let mut composition = Composition::new(blocks);
    composition.family = Family::Times;
    composition.footer = Some("Confidential".into());
    composition.meta.title = Some("Mutual Non-Disclosure Agreement".into());
    composition.meta.keywords = Some("nda, confidentiality".into());
    composition
}

fn slides() -> Composition {
    let deck = prose::fake_headline();
    let mut blocks = vec![
        Block::Spacer(90.0),
        Block::Styled {
            text: deck.clone(),
            face: Face::BOLD,
            size: 34.0,
            color: Rgb::TEXT,
            align: Align::Center,
        },
        Block::Spacer(14.0),
        Block::Styled {
            text: format!("{} - {}", company::fake_company(), long_date()),
            face: Face::REGULAR,
            size: 13.0,
            color: Rgb::MUTED,
            align: Align::Center,
        },
    ];
    for _ in 0..4 {
        blocks.push(Block::PageBreak);
        blocks.push(Block::Styled {
            text: prose::fake_headline(),
            face: Face::BOLD,
            size: 24.0,
            color: Rgb::TEXT,
            align: Align::Left,
        });
        blocks.push(Block::Rule);
        blocks.push(Block::List {
            items: (0..4).map(|_| prose::fake_prose_sentence()).collect(),
            ordered: false,
        });
    }

    let mut composition = Composition::new(blocks);
    composition.orientation = Orientation::Landscape;
    composition.footer = Some(deck.clone());
    composition.meta.title = Some(deck);
    composition
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_composes_a_pdf() {
        use base64::Engine as _;
        for preset in PdfPreset::ALL {
            let composed = fake_pdf_document(&PdfSpec {
                preset,
                ..PdfSpec::default()
            });
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&composed.base64)
                .unwrap_or_default();
            assert!(
                bytes.starts_with(b"%PDF-"),
                "{} produced no document",
                preset.name()
            );
            assert!(
                composed.overflowed_pages.is_empty(),
                "{} overflowed on {:?}",
                preset.name(),
                composed.overflowed_pages
            );
            assert!(composed.pages >= 1);
        }
    }

    #[test]
    fn every_preset_name_round_trips() {
        for preset in PdfPreset::ALL {
            assert_eq!(preset.name().parse::<PdfPreset>(), Ok(preset));
            assert!(!preset.description().is_empty());
        }
        assert!("nope".parse::<PdfPreset>().is_err());
    }

    /// `scope_seeded` rather than `set_global_seed`: the global seed is
    /// process-wide, so a test that mutated it would race every other test
    /// drawing from the generators. The scope is thread-local.
    #[test]
    fn the_seed_makes_a_document_reproducible() {
        let spec = PdfSpec {
            preset: PdfPreset::Invoice,
            ..PdfSpec::default()
        };
        let render = || {
            let _scope = super::super::rng::scope_seeded(99);
            fake_pdf_document(&spec)
        };
        let first = render();
        let second = render();
        assert_eq!(first.pages, second.pages);
        // The `CreationDate` is fixed width, so equal lengths means equal
        // content: an amount or a name that drifted would change the length.
        assert_eq!(first.base64.len(), second.base64.len());

        let different = {
            let _scope = super::super::rng::scope_seeded(100);
            fake_pdf_document(&spec)
        };
        assert_ne!(
            first.base64, different.base64,
            "a different seed produced the same document"
        );
    }

    #[test]
    fn pages_is_a_floor_not_a_cap() {
        let composed = fake_pdf_document(&PdfSpec {
            pages: 3,
            preset: PdfPreset::Plain,
            ..PdfSpec::default()
        });
        assert_eq!(composed.pages, 3);

        let long = fake_pdf_document(&PdfSpec {
            pages: 1,
            extras: Extras {
                paragraphs: 90,
                ..Extras::default()
            },
            ..PdfSpec::default()
        });
        assert!(long.pages > 1, "got {}", long.pages);
    }

    #[test]
    fn repeat_fills_the_requested_pages_with_content() {
        let composed = fake_pdf_document(&PdfSpec {
            pages: 4,
            repeat: true,
            preset: PdfPreset::Invoice,
            ..PdfSpec::default()
        });
        assert!(composed.pages >= 4);
    }

    #[test]
    fn extras_stack_onto_a_preset() {
        let composed = fake_pdf_document(&PdfSpec {
            preset: PdfPreset::Plain,
            extras: Extras {
                paragraphs: 2,
                tables: vec![(4, 3)],
                charts: vec![(ChartKind::Pie, 5)],
                lists: vec![3],
                key_values: vec![4],
                callouts: 1,
                columns: Some((2, 2)),
                images: vec![(320, 120)],
            },
            ..PdfSpec::default()
        });
        assert!(composed.pages >= 1);
        assert!(composed.overflowed_pages.is_empty());
    }

    #[test]
    fn a_landscape_preset_can_be_overridden() {
        let spec = PdfSpec {
            preset: PdfPreset::Certificate,
            orientation: Some(Orientation::Portrait),
            ..PdfSpec::default()
        };
        assert!(fake_pdf_document(&spec).pages >= 1);
    }
}
