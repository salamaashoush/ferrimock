//! PDF composition: a flowing document model measured against the page.
//!
//! Text is measured with the Adobe AFM advance widths for the base-14 faces,
//! which is what a viewer uses, so a line this module reports as fitting is a
//! line that renders inside the page.
//!
//! Content is flattened into atoms that each know their own height and then
//! greedily filled onto pages. A document longer than one page therefore
//! continues onto the next rather than being cut at the bottom margin, and a
//! table broken across a page boundary repeats its header row.

use lopdf::{
    Document, Object, ObjectId, Stream,
    content::{Content, Operation},
    dictionary,
};

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A4, in points. The default page, kept as a constant because callers size
/// images and columns against it.
pub const PAGE_WIDTH: f64 = 595.0;
/// A4, in points.
pub const PAGE_HEIGHT: f64 = 842.0;
/// Left, right and top margin.
pub const MARGIN: f64 = 56.0;

/// Everything below this belongs to the page furniture.
const FOOTER_TOP: f64 = 46.0;

const BODY_SIZE: f64 = 11.0;
const TABLE_SIZE: f64 = 9.5;
const FOOTER_SIZE: f64 = 8.0;
/// Baseline-to-baseline distance as a multiple of the font size.
const LEADING: f64 = 1.32;
const CELL_PADDING: f64 = 5.0;
const BLOCK_GAP: f64 = 8.0;

/// A stock paper size, in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSize {
    #[default]
    A4,
    A3,
    A5,
    Letter,
    Legal,
    Tabloid,
}

impl PageSize {
    /// Width and height in points, portrait.
    #[must_use]
    pub const fn points(self) -> (f64, f64) {
        match self {
            Self::A4 => (595.0, 842.0),
            Self::A3 => (842.0, 1191.0),
            Self::A5 => (420.0, 595.0),
            Self::Letter => (612.0, 792.0),
            Self::Legal => (612.0, 1008.0),
            Self::Tabloid => (792.0, 1224.0),
        }
    }
}

impl std::str::FromStr for PageSize {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "a4" => Ok(Self::A4),
            "a3" => Ok(Self::A3),
            "a5" => Ok(Self::A5),
            "letter" => Ok(Self::Letter),
            "legal" => Ok(Self::Legal),
            "tabloid" | "ledger" => Ok(Self::Tabloid),
            other => Err(format!(
                "unknown page size {other:?}; try a4, a3, a5, letter, legal or tabloid"
            )),
        }
    }
}

/// Which way round the page sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Portrait,
    Landscape,
}

impl std::str::FromStr for Orientation {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "portrait" | "tall" => Ok(Self::Portrait),
            "landscape" | "wide" => Ok(Self::Landscape),
            other => Err(format!(
                "unknown orientation {other:?}; try portrait or landscape"
            )),
        }
    }
}

/// The page box and its margins, in points.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub width: f64,
    pub height: f64,
    pub margin: f64,
}

impl Default for Geometry {
    fn default() -> Self {
        Self {
            width: PAGE_WIDTH,
            height: PAGE_HEIGHT,
            margin: MARGIN,
        }
    }
}

impl Geometry {
    /// The page `size` turned `orientation`, with the default margin.
    #[must_use]
    pub const fn new(size: PageSize, orientation: Orientation) -> Self {
        let (width, height) = size.points();
        let (width, height) = match orientation {
            Orientation::Portrait => (width, height),
            Orientation::Landscape => (height, width),
        };
        Self {
            width,
            height,
            margin: MARGIN,
        }
    }

    /// Width of the text column.
    #[must_use]
    pub fn body_width(&self) -> f64 {
        2.0f64.mul_add(-self.margin, self.width)
    }

    /// The y coordinate content starts at.
    #[must_use]
    pub fn top(&self) -> f64 {
        self.height - self.margin
    }

    /// The y coordinate content must stay above.
    #[must_use]
    pub fn floor(&self) -> f64 {
        2.0f64.mul_add(FOOTER_SIZE, FOOTER_TOP)
    }

    /// How much vertical room one page has for content.
    #[must_use]
    pub fn body_height(&self) -> f64 {
        self.top() - self.floor()
    }
}

/// Width of the default A4 text column.
#[must_use]
pub fn body_width() -> f64 {
    Geometry::default().body_width()
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// A device RGB colour, each channel in 0.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const BLACK: Self = Self::gray(0.0);
    pub const TEXT: Self = Self::gray(0.1);
    pub const MUTED: Self = Self::gray(0.42);
    pub const RULE: Self = Self::gray(0.75);
    pub const ZEBRA: Self = Self::gray(0.955);

    #[must_use]
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }

    #[must_use]
    pub const fn gray(level: f64) -> Self {
        Self {
            r: level,
            g: level,
            b: level,
        }
    }

    /// Parse `#rrggbb`, `rrggbb` or `#rgb`. Returns `None` for anything else.
    #[must_use]
    pub fn parse(hex: &str) -> Option<Self> {
        let hex = hex.trim().trim_start_matches('#');
        let channel = |pair: &str| u8::from_str_radix(pair, 16).ok().map(f64::from);
        let bytes: Vec<char> = hex.chars().collect();
        let (r, g, b) = match bytes.len() {
            3 => {
                let mut expanded = String::with_capacity(6);
                for c in &bytes {
                    expanded.push(*c);
                    expanded.push(*c);
                }
                return Self::parse(&expanded);
            }
            6 => (
                channel(hex.get(0..2)?)?,
                channel(hex.get(2..4)?)?,
                channel(hex.get(4..6)?)?,
            ),
            _ => return None,
        };
        Some(Self::new(r / 255.0, g / 255.0, b / 255.0))
    }

    /// This colour mixed `amount` of the way towards white.
    #[must_use]
    pub fn tint(self, amount: f64) -> Self {
        let mix = |c: f64| amount.mul_add(1.0 - c, c);
        Self::new(mix(self.r), mix(self.g), mix(self.b))
    }

    /// Black or white, whichever stays legible on this colour.
    #[must_use]
    pub fn contrasting(self) -> Self {
        let luma = 0.0722f64.mul_add(self.b, 0.2126f64.mul_add(self.r, 0.7152 * self.g));
        if luma > 0.6 {
            Self::gray(0.08)
        } else {
            Self::gray(1.0)
        }
    }
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

/// Adobe AFM advance widths for Helvetica, ASCII 32..=126, in 1/1000 em.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Adobe AFM advance widths for Helvetica-Bold, ASCII 32..=126, in 1/1000 em.
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Adobe AFM advance widths for Times-Roman, ASCII 32..=126, in 1/1000 em.
const TIMES: [u16; 95] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611,
    556, 722, 722, 333, 389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722,
    722, 611, 333, 278, 333, 469, 500, 333, 444, 500, 444, 500, 444, 333, 500, 500, 278, 278, 500,
    278, 778, 500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480, 200, 480, 541,
];

/// Adobe AFM advance widths for Times-Bold, ASCII 32..=126, in 1/1000 em.
const TIMES_BOLD: [u16; 95] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 930, 722, 667, 722, 722, 667,
    611, 778, 778, 389, 500, 778, 667, 944, 722, 778, 611, 778, 722, 556, 667, 722, 722, 1000, 722,
    722, 667, 333, 278, 333, 581, 500, 333, 500, 556, 444, 556, 444, 333, 500, 556, 278, 333, 556,
    278, 833, 556, 500, 556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394, 220, 394, 520,
];

/// Adobe AFM advance widths for Times-Italic, ASCII 32..=126, in 1/1000 em.
const TIMES_ITALIC: [u16; 95] = [
    250, 333, 420, 500, 500, 833, 778, 214, 333, 333, 500, 675, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 333, 333, 675, 675, 675, 500, 920, 611, 611, 667, 722, 611,
    611, 722, 722, 333, 444, 667, 556, 833, 667, 722, 611, 722, 611, 500, 556, 722, 611, 833, 611,
    556, 556, 389, 278, 389, 422, 500, 333, 500, 500, 444, 500, 444, 278, 500, 500, 278, 278, 444,
    278, 722, 500, 500, 500, 500, 389, 389, 278, 500, 444, 667, 444, 444, 389, 400, 275, 400, 541,
];

/// Adobe AFM advance widths for Times-BoldItalic, ASCII 32..=126, in 1/1000 em.
const TIMES_BOLD_ITALIC: [u16; 95] = [
    250, 389, 555, 500, 500, 833, 778, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500,
    500, 500, 500, 500, 500, 500, 500, 333, 333, 570, 570, 570, 500, 832, 667, 667, 667, 722, 667,
    667, 722, 778, 389, 500, 667, 611, 889, 722, 722, 611, 722, 667, 556, 611, 722, 667, 889, 667,
    611, 611, 333, 278, 333, 570, 500, 333, 500, 500, 444, 500, 444, 333, 500, 556, 278, 278, 500,
    278, 778, 556, 500, 500, 500, 389, 389, 278, 556, 444, 667, 500, 444, 389, 348, 220, 348, 570,
];

/// Courier is monospaced: every glyph advances the same.
const COURIER_ADVANCE: u16 = 600;

/// Which of the three base-14 families a run of text is set in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Family {
    #[default]
    Helvetica,
    Times,
    Courier,
}

impl std::str::FromStr for Family {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "helvetica" | "sans" | "sans-serif" | "arial" => Ok(Self::Helvetica),
            "times" | "serif" | "roman" => Ok(Self::Times),
            "courier" | "mono" | "monospace" => Ok(Self::Courier),
            other => Err(format!(
                "unknown font {other:?}; try helvetica, times or courier"
            )),
        }
    }
}

/// One of the twelve base-14 text faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    pub family: Family,
    pub bold: bool,
    pub italic: bool,
}

impl Default for Face {
    fn default() -> Self {
        Self::REGULAR
    }
}

impl Face {
    pub const REGULAR: Self = Self {
        family: Family::Helvetica,
        bold: false,
        italic: false,
    };
    pub const BOLD: Self = Self {
        family: Family::Helvetica,
        bold: true,
        italic: false,
    };
    pub const ITALIC: Self = Self {
        family: Family::Helvetica,
        bold: false,
        italic: true,
    };

    /// The same face in `family`, keeping weight and slant.
    #[must_use]
    pub const fn in_family(self, family: Family) -> Self {
        Self { family, ..self }
    }

    #[must_use]
    pub const fn bold(self) -> Self {
        Self { bold: true, ..self }
    }

    #[must_use]
    pub const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }

    /// Index into the document's font table, and so into `/F{n}`.
    const fn slot(self) -> usize {
        let family = match self.family {
            Family::Helvetica => 0,
            Family::Times => 4,
            Family::Courier => 8,
        };
        family + (self.bold as usize) + if self.italic { 2 } else { 0 }
    }

    fn resource(self) -> String {
        format!("F{}", self.slot() + 1)
    }

    /// The `BaseFont` name a viewer resolves to a built-in face.
    const fn base_font(self) -> &'static str {
        match (self.family, self.bold, self.italic) {
            (Family::Helvetica, false, false) => "Helvetica",
            (Family::Helvetica, true, false) => "Helvetica-Bold",
            (Family::Helvetica, false, true) => "Helvetica-Oblique",
            (Family::Helvetica, true, true) => "Helvetica-BoldOblique",
            (Family::Times, false, false) => "Times-Roman",
            (Family::Times, true, false) => "Times-Bold",
            (Family::Times, false, true) => "Times-Italic",
            (Family::Times, true, true) => "Times-BoldItalic",
            (Family::Courier, false, false) => "Courier",
            (Family::Courier, true, false) => "Courier-Bold",
            (Family::Courier, false, true) => "Courier-Oblique",
            (Family::Courier, true, true) => "Courier-BoldOblique",
        }
    }

    /// Advance widths, or `None` for the monospaced family.
    const fn widths(self) -> Option<&'static [u16; 95]> {
        match (self.family, self.bold, self.italic) {
            // Oblique is the upright outline sheared, so the metrics are shared.
            (Family::Helvetica, false, _) => Some(&HELVETICA),
            (Family::Helvetica, true, _) => Some(&HELVETICA_BOLD),
            (Family::Times, false, false) => Some(&TIMES),
            (Family::Times, true, false) => Some(&TIMES_BOLD),
            (Family::Times, false, true) => Some(&TIMES_ITALIC),
            (Family::Times, true, true) => Some(&TIMES_BOLD_ITALIC),
            (Family::Courier, _, _) => None,
        }
    }
}

/// Every face the document registers, in slot order.
const FACES: [Face; 12] = [
    Face {
        family: Family::Helvetica,
        bold: false,
        italic: false,
    },
    Face {
        family: Family::Helvetica,
        bold: true,
        italic: false,
    },
    Face {
        family: Family::Helvetica,
        bold: false,
        italic: true,
    },
    Face {
        family: Family::Helvetica,
        bold: true,
        italic: true,
    },
    Face {
        family: Family::Times,
        bold: false,
        italic: false,
    },
    Face {
        family: Family::Times,
        bold: true,
        italic: false,
    },
    Face {
        family: Family::Times,
        bold: false,
        italic: true,
    },
    Face {
        family: Family::Times,
        bold: true,
        italic: true,
    },
    Face {
        family: Family::Courier,
        bold: false,
        italic: false,
    },
    Face {
        family: Family::Courier,
        bold: true,
        italic: false,
    },
    Face {
        family: Family::Courier,
        bold: false,
        italic: true,
    },
    Face {
        family: Family::Courier,
        bold: true,
        italic: true,
    },
];

fn advance(face: Face, c: char) -> f64 {
    let Some(widths) = face.widths() else {
        return f64::from(COURIER_ADVANCE) / 1000.0;
    };
    // Outside the AFM range the WinAnsi glyph is one this table does not
    // carry, so charge an average rather than nothing and keep wrapping honest.
    let thousandths = usize::try_from(u32::from(c))
        .ok()
        .and_then(|code| code.checked_sub(32))
        .and_then(|index| widths.get(index))
        .copied()
        .unwrap_or(556);
    f64::from(thousandths) / 1000.0
}

/// Width of `text` set in `face` at `size`, in points.
#[must_use]
pub fn text_width(text: &str, face: Face, size: f64) -> f64 {
    text.chars().map(|c| advance(face, c)).sum::<f64>() * size
}

/// Break `text` so that no line exceeds `max_width` when set in `face` at
/// `size`. A single word wider than the column is split rather than allowed to
/// run off the page.
#[must_use]
pub fn wrap(text: &str, max_width: f64, face: Face, size: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();

    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };

        if text_width(&candidate, face, size) <= max_width {
            line = candidate;
            continue;
        }

        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }

        // The word alone still has to fit, so break it at the last character
        // that does.
        let mut chunk = String::new();
        for c in word.chars() {
            let mut wider = chunk.clone();
            wider.push(c);
            if !chunk.is_empty() && text_width(&wider, face, size) > max_width {
                lines.push(std::mem::take(&mut chunk));
            }
            chunk.push(c);
        }
        line = chunk;
    }

    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Whether every character in `text` has a WinAnsi glyph.
///
/// A caller that can choose its wording (a currency symbol, say) should ask
/// first rather than emit a run of `?`.
#[must_use]
pub fn win_ansi_encodable(text: &str) -> bool {
    text.chars()
        .all(|c| win_ansi(&c.to_string()) != vec![b'?'] || c == '?')
}

/// Encode `text` as WinAnsi bytes, which is the encoding the font dictionaries
/// declare. A character with no WinAnsi glyph becomes `?` rather than a pair of
/// UTF-8 bytes the viewer would draw as two wrong glyphs.
fn win_ansi(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| match c {
            '\u{20ac}' => 0x80,
            '\u{201a}' => 0x82,
            '\u{192}' => 0x83,
            '\u{201e}' => 0x84,
            '\u{2026}' => 0x85,
            '\u{2020}' => 0x86,
            '\u{2021}' => 0x87,
            '\u{2c6}' => 0x88,
            '\u{2030}' => 0x89,
            '\u{160}' => 0x8a,
            '\u{2039}' => 0x8b,
            '\u{152}' => 0x8c,
            '\u{17d}' => 0x8e,
            '\u{2018}' => 0x91,
            '\u{2019}' => 0x92,
            '\u{201c}' => 0x93,
            '\u{201d}' => 0x94,
            '\u{2022}' => 0x95,
            '\u{2013}' => 0x96,
            '\u{2014}' => 0x97,
            '\u{2dc}' => 0x98,
            '\u{2122}' => 0x99,
            '\u{161}' => 0x9a,
            '\u{203a}' => 0x9b,
            '\u{153}' => 0x9c,
            '\u{17e}' => 0x9e,
            '\u{178}' => 0x9f,
            c if (c as u32) < 0x100 => c as u8,
            _ => b'?',
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Content model
// ---------------------------------------------------------------------------

/// How a cell or line sits in the width available to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl std::str::FromStr for Align {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "left" | "start" => Ok(Self::Left),
            "center" | "centre" | "middle" => Ok(Self::Center),
            "right" | "end" => Ok(Self::Right),
            other => Err(format!(
                "unknown alignment {other:?}; try left, center or right"
            )),
        }
    }
}

/// A table drawn with a ruled grid and a bold header row.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Relative column widths. Empty means every column is equal.
    pub weights: Vec<f64>,
    /// Per-column alignment. Missing entries fall back to left.
    pub align: Vec<Align>,
    /// Shade alternate body rows.
    pub zebra: bool,
    /// Draw the cell grid.
    pub grid: bool,
    /// Fill behind the header row.
    pub header_fill: Option<Rgb>,
}

impl Table {
    /// A ruled table with a shaded header, which is what most documents use.
    #[must_use]
    pub fn new(headers: Vec<String>, rows: Vec<Vec<String>>) -> Self {
        Self {
            headers,
            rows,
            weights: Vec::new(),
            align: Vec::new(),
            zebra: false,
            grid: true,
            header_fill: None,
        }
    }

    fn column_count(&self) -> usize {
        self.headers
            .len()
            .max(self.rows.iter().map(Vec::len).max().unwrap_or(0))
    }

    /// Column widths in points, from the weights or evenly.
    fn column_widths(&self, total: f64) -> Vec<f64> {
        let columns = self.column_count();
        if columns == 0 {
            return Vec::new();
        }
        let weights: Vec<f64> = (0..columns)
            .map(|column| self.weights.get(column).copied().unwrap_or(1.0).max(0.01))
            .collect();
        let sum: f64 = weights.iter().sum();
        weights.iter().map(|w| total * w / sum).collect()
    }

    fn alignment(&self, column: usize) -> Align {
        self.align.get(column).copied().unwrap_or_default()
    }
}

/// Which shape a chart is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
    Area,
}

impl std::str::FromStr for ChartKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "bar" | "column" => Ok(Self::Bar),
            "line" => Ok(Self::Line),
            "pie" | "donut" => Ok(Self::Pie),
            "area" => Ok(Self::Area),
            other => Err(format!(
                "unknown chart {other:?}; try bar, line, pie or area"
            )),
        }
    }
}

/// A chart drawn with vector operators rather than embedded as an image.
#[derive(Debug, Clone)]
pub struct Chart {
    pub kind: ChartKind,
    pub title: Option<String>,
    pub labels: Vec<String>,
    pub values: Vec<f64>,
    pub height: f64,
    pub accent: Rgb,
}

/// One piece of page content. Blocks flow top to bottom and across pages.
#[derive(Debug, Clone)]
pub enum Block {
    /// A bold, larger line. Level 1 is the document title, 3 a run-in heading.
    Heading {
        text: String,
        level: u8,
    },
    /// Prose that reflows to the column width.
    Paragraph(String),
    /// One logical line that wraps but is not merged with its neighbours.
    Line(String),
    /// Prose in a named face, size and colour.
    Styled {
        text: String,
        face: Face,
        size: f64,
        color: Rgb,
        align: Align,
    },
    /// A bulleted or numbered list.
    List {
        items: Vec<String>,
        ordered: bool,
    },
    /// Label and value pairs set in two columns, as a form or a summary uses.
    KeyValues(Vec<(String, String)>),
    Table(Table),
    /// A base64 PNG, scaled to `width` points wide, aspect preserved.
    Image {
        png_base64: String,
        width: f64,
        align: Align,
        caption: Option<String>,
    },
    /// A horizontal rule across the text column.
    Rule,
    /// Tinted panel with an optional bold first line.
    Callout {
        title: Option<String>,
        body: String,
        tint: Rgb,
    },
    /// Ruled signature lines, one per name.
    Signature(Vec<String>),
    Chart(Chart),
    /// Text laid out in `columns` newspaper columns.
    Columns {
        columns: usize,
        text: String,
    },
    /// Vertical space, in points.
    Spacer(f64),
    /// Start the next block on a fresh page.
    PageBreak,
}

/// What goes in the PDF `Info` dictionary.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
}

/// A whole document: the blocks, the page it sits on, and its furniture.
#[derive(Debug, Clone)]
pub struct Doc {
    pub blocks: Vec<Block>,
    pub geometry: Geometry,
    pub meta: Meta,
    /// Running head, drawn above the text column on every page.
    pub header: Option<String>,
    /// Running foot, drawn on the left of the footer line.
    pub footer: Option<String>,
    /// Diagonal stamp across every page.
    pub watermark: Option<String>,
    /// Draw `Page n of m` on the right of the footer line.
    pub page_numbers: bool,
    /// Keep emitting pages until the document has at least this many.
    pub min_pages: u32,
    /// Body family; headings and tables follow it.
    pub family: Family,
    /// Colour for headings, rules under them, and chart bars.
    pub accent: Rgb,
}

impl Default for Doc {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            geometry: Geometry::default(),
            meta: Meta::default(),
            header: None,
            footer: None,
            watermark: None,
            page_numbers: true,
            min_pages: 1,
            family: Family::Helvetica,
            accent: Rgb::new(0.11, 0.33, 0.60),
        }
    }
}

/// What `compose` produced.
#[derive(Debug, Clone)]
pub struct Composed {
    /// The document, base64 encoded.
    pub base64: String,
    /// Pages that could not hold a single indivisible block, so dropped it.
    pub overflowed_pages: Vec<u32>,
    /// How many pages the flow produced.
    pub pages: u32,
}

// ---------------------------------------------------------------------------
// Atoms
// ---------------------------------------------------------------------------

/// One drawable unit with a known height. Pagination works on these, so
/// anything that must not be split across pages is a single atom.
#[derive(Debug, Clone)]
enum Atom {
    Text {
        text: String,
        x: f64,
        width: f64,
        face: Face,
        size: f64,
        color: Rgb,
        align: Align,
        height: f64,
        /// Move to the next page rather than ending one, as a heading does.
        keep: bool,
    },
    Row {
        cells: Vec<String>,
        widths: Vec<f64>,
        align: Vec<Align>,
        face: Face,
        fill: Option<Rgb>,
        grid: bool,
        height: f64,
        /// Which table this row belongs to, so a break can repeat the header.
        table: usize,
    },
    Image {
        jpeg: Vec<u8>,
        pixels: (u32, u32),
        draw: (f64, f64),
        x: f64,
        height: f64,
    },
    Rule {
        color: Rgb,
        thickness: f64,
        width: f64,
        height: f64,
        keep: bool,
    },
    Panel {
        lines: Vec<(String, Face)>,
        tint: Rgb,
        height: f64,
    },
    Chart {
        chart: Chart,
        height: f64,
    },
    Columns {
        columns: Vec<Vec<String>>,
        face: Face,
        size: f64,
        column_width: f64,
        gutter: f64,
        height: f64,
    },
    Space(f64),
    Break,
}

impl Atom {
    /// Whether this atom introduces the one after it and so cannot be the last
    /// thing on a page.
    const fn keeps(&self) -> bool {
        match self {
            Self::Text { keep, .. } | Self::Rule { keep, .. } => *keep,
            _ => false,
        }
    }

    const fn height(&self) -> f64 {
        match self {
            Self::Text { height, .. }
            | Self::Row { height, .. }
            | Self::Image { height, .. }
            | Self::Rule { height, .. }
            | Self::Panel { height, .. }
            | Self::Chart { height, .. }
            | Self::Columns { height, .. }
            | Self::Space(height) => *height,
            Self::Break => 0.0,
        }
    }
}

/// Turns blocks into atoms against a fixed geometry.
struct Flattener<'a> {
    doc: &'a Doc,
    atoms: Vec<Atom>,
    /// Header row per table, replayed when a table breaks across a page.
    table_headers: Vec<Option<Atom>>,
}

impl<'a> Flattener<'a> {
    fn new(doc: &'a Doc) -> Self {
        Self {
            doc,
            atoms: Vec::new(),
            table_headers: Vec::new(),
        }
    }

    fn body_width(&self) -> f64 {
        self.doc.geometry.body_width()
    }

    fn face(&self) -> Face {
        Face::REGULAR.in_family(self.doc.family)
    }

    fn text(
        &mut self,
        text: &str,
        face: Face,
        size: f64,
        color: Rgb,
        align: Align,
        x: f64,
        width: f64,
    ) {
        self.text_keeping(text, face, size, color, align, x, width, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn text_keeping(
        &mut self,
        text: &str,
        face: Face,
        size: f64,
        color: Rgb,
        align: Align,
        x: f64,
        width: f64,
        keep: bool,
    ) {
        let leading = size * LEADING;
        for line in wrap(text, width, face, size) {
            self.atoms.push(Atom::Text {
                text: line,
                x,
                width,
                face,
                size,
                color,
                align,
                height: leading,
                keep,
            });
        }
    }

    fn gap(&mut self, height: f64) {
        self.atoms.push(Atom::Space(height));
    }

    fn push(&mut self, block: &Block) {
        let width = self.body_width();
        let body = self.face();
        match block {
            Block::Spacer(height) => self.gap(*height),
            Block::PageBreak => self.atoms.push(Atom::Break),
            Block::Heading { text, level } => {
                let size = match level {
                    0 | 1 => 19.0,
                    2 => 14.5,
                    _ => 12.0,
                };
                let color = if *level <= 2 {
                    self.doc.accent
                } else {
                    Rgb::TEXT
                };
                self.gap(if *level <= 1 { 2.0 } else { 6.0 });
                self.text_keeping(
                    text,
                    body.bold(),
                    size,
                    color,
                    Align::Left,
                    0.0,
                    width,
                    true,
                );
                if *level <= 1 {
                    self.atoms.push(Atom::Rule {
                        color: self.doc.accent.tint(0.55),
                        thickness: 1.2,
                        width,
                        height: 6.0,
                        keep: true,
                    });
                }
                self.gap(4.0);
            }
            Block::Paragraph(text) | Block::Line(text) => {
                self.text(text, body, BODY_SIZE, Rgb::TEXT, Align::Left, 0.0, width);
                self.gap(4.0);
            }
            Block::Styled {
                text,
                face,
                size,
                color,
                align,
            } => {
                self.text(text, *face, *size, *color, *align, 0.0, width);
                self.gap(4.0);
            }
            Block::List { items, ordered } => {
                let indent = 18.0;
                for (index, item) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}.", index + 1)
                    } else {
                        "\u{2022}".to_string()
                    };
                    self.atoms.push(Atom::Text {
                        text: marker,
                        x: 0.0,
                        width: indent,
                        face: body,
                        size: BODY_SIZE,
                        color: Rgb::TEXT,
                        align: Align::Left,
                        // The marker shares a baseline with the first wrapped
                        // line, so it must not advance the cursor itself.
                        height: 0.0,
                        keep: true,
                    });
                    self.text(
                        item,
                        body,
                        BODY_SIZE,
                        Rgb::TEXT,
                        Align::Left,
                        indent,
                        width - indent,
                    );
                    self.gap(2.0);
                }
                self.gap(4.0);
            }
            Block::KeyValues(pairs) => {
                let label_width = width * 0.32;
                for (key, value) in pairs {
                    self.atoms.push(Atom::Text {
                        text: key.clone(),
                        x: 0.0,
                        width: label_width,
                        face: body.bold(),
                        size: BODY_SIZE,
                        color: Rgb::MUTED,
                        align: Align::Left,
                        height: 0.0,
                        keep: true,
                    });
                    self.text(
                        value,
                        body,
                        BODY_SIZE,
                        Rgb::TEXT,
                        Align::Left,
                        label_width,
                        width - label_width,
                    );
                    self.gap(2.0);
                }
                self.gap(4.0);
            }
            Block::Table(table) => self.push_table(table),
            Block::Rule => self.atoms.push(Atom::Rule {
                color: Rgb::RULE,
                thickness: 0.6,
                width,
                height: BLOCK_GAP * 2.0,
                keep: false,
            }),
            Block::Callout {
                title,
                body: text,
                tint,
            } => {
                let inner = (2.0 * CELL_PADDING).mul_add(-2.0, width);
                let mut lines = Vec::new();
                if let Some(title) = title {
                    for line in wrap(title, inner, body.bold(), BODY_SIZE) {
                        lines.push((line, body.bold()));
                    }
                }
                for line in wrap(text, inner, body, BODY_SIZE) {
                    lines.push((line, body));
                }
                let height = (lines.len() as f64).mul_add(BODY_SIZE * LEADING, 4.0 * CELL_PADDING);
                self.atoms.push(Atom::Panel {
                    lines,
                    tint: *tint,
                    height,
                });
                self.gap(BLOCK_GAP);
            }
            Block::Signature(names) => {
                self.gap(16.0);
                for name in names {
                    self.atoms.push(Atom::Rule {
                        color: Rgb::gray(0.35),
                        thickness: 0.7,
                        width: width * 0.46,
                        height: 4.0,
                        keep: true,
                    });
                    self.text(name, body, 9.0, Rgb::MUTED, Align::Left, 0.0, width * 0.46);
                    self.gap(14.0);
                }
            }
            Block::Chart(chart) => {
                let height = chart.height.max(80.0);
                self.atoms.push(Atom::Chart {
                    chart: chart.clone(),
                    height: height + if chart.title.is_some() { 18.0 } else { 0.0 },
                });
                self.gap(BLOCK_GAP);
            }
            Block::Columns { columns, text } => {
                let columns = (*columns).clamp(1, 4);
                let gutter = 16.0;
                let column_width =
                    f64::mul_add(gutter, -((columns - 1) as f64), width) / columns as f64;
                let lines = wrap(text, column_width, body, BODY_SIZE);
                let per_column = lines.len().div_ceil(columns).max(1);
                let split: Vec<Vec<String>> =
                    lines.chunks(per_column).map(<[String]>::to_vec).collect();
                let height = (per_column as f64) * BODY_SIZE * LEADING;
                self.atoms.push(Atom::Columns {
                    columns: split,
                    face: body,
                    size: BODY_SIZE,
                    column_width,
                    gutter,
                    height,
                });
                self.gap(BLOCK_GAP);
            }
            Block::Image {
                png_base64,
                width: draw_width,
                align,
                caption,
            } => {
                let Some((jpeg, px_w, px_h)) = jpeg_from_png_base64(png_base64) else {
                    return;
                };
                let draw_width = draw_width.min(width);
                let draw_height = draw_width * f64::from(px_h) / f64::from(px_w);
                let x = match align {
                    Align::Left => 0.0,
                    Align::Center => (width - draw_width) / 2.0,
                    Align::Right => width - draw_width,
                };
                self.atoms.push(Atom::Image {
                    jpeg,
                    pixels: (px_w, px_h),
                    draw: (draw_width, draw_height),
                    x,
                    height: draw_height,
                });
                if let Some(caption) = caption {
                    self.gap(4.0);
                    self.text(caption, body.italic(), 9.0, Rgb::MUTED, *align, 0.0, width);
                }
                self.gap(BLOCK_GAP);
            }
        }
    }

    fn push_table(&mut self, table: &Table) {
        let columns = table.column_count();
        if columns == 0 {
            return;
        }
        let width = self.body_width();
        let widths = table.column_widths(width);
        let align: Vec<Align> = (0..columns).map(|c| table.alignment(c)).collect();
        let body = self.face();
        let row_height = 2.0f64.mul_add(CELL_PADDING, TABLE_SIZE * LEADING);
        let table_id = self.table_headers.len();

        let header = if table.headers.is_empty() {
            None
        } else {
            Some(Atom::Row {
                cells: table.headers.clone(),
                widths: widths.clone(),
                align: align.clone(),
                face: body.bold(),
                fill: Some(
                    table
                        .header_fill
                        .unwrap_or_else(|| self.doc.accent.tint(0.82)),
                ),
                grid: table.grid,
                height: row_height,
                table: table_id,
            })
        };
        self.table_headers.push(header.clone());
        if let Some(header) = header {
            self.atoms.push(header);
        }

        for (index, row) in table.rows.iter().enumerate() {
            let fill = if table.zebra && index % 2 == 1 {
                Some(Rgb::ZEBRA)
            } else {
                None
            };
            self.atoms.push(Atom::Row {
                cells: row.clone(),
                widths: widths.clone(),
                align: align.clone(),
                face: body,
                fill,
                grid: table.grid,
                height: row_height,
                table: table_id,
            });
        }
        self.gap(BLOCK_GAP);
    }
}

/// Greedily fill pages with atoms, repeating a broken table's header row.
fn paginate(atoms: Vec<Atom>, doc: &Doc, headers: &[Option<Atom>]) -> (Vec<Vec<Atom>>, Vec<u32>) {
    let usable = doc.geometry.body_height();
    let mut pages: Vec<Vec<Atom>> = Vec::new();
    let mut current: Vec<Atom> = Vec::new();
    let mut used = 0.0f64;
    let mut overflowed = Vec::new();

    for atom in atoms {
        if matches!(atom, Atom::Break) {
            if !current.is_empty() {
                pages.push(std::mem::take(&mut current));
                used = 0.0;
            }
            continue;
        }

        let height = atom.height();
        if used + height <= usable {
            used += height;
            current.push(atom);
            continue;
        }

        // A block taller than a whole page can never be placed; say so rather
        // than looping forever trying to find room for it.
        if height > usable {
            overflowed.push(pages.len() as u32 + 1);
            continue;
        }

        // Leading whitespace on a fresh page is noise, not layout.
        if matches!(atom, Atom::Space(_)) {
            pages.push(std::mem::take(&mut current));
            used = 0.0;
            continue;
        }

        // A heading that would end a page belongs at the top of the next one,
        // along with the gap that sits under it.
        let mut carried = Vec::new();
        loop {
            let mut spaces = Vec::new();
            while current.len() > 1 && matches!(current.last(), Some(Atom::Space(_))) {
                if let Some(space) = current.pop() {
                    spaces.push(space);
                }
            }
            let keeps = current.len() > 1 && current.last().is_some_and(Atom::keeps);
            if !keeps {
                // Not a run to carry, so the gaps belong where they were.
                spaces.reverse();
                current.extend(spaces);
                break;
            }
            carried.extend(spaces);
            if let Some(atom) = current.pop() {
                carried.push(atom);
            }
        }
        carried.reverse();

        pages.push(std::mem::take(&mut current));
        used = carried.iter().map(Atom::height).sum();
        current = carried;

        if let Atom::Row { table, .. } = &atom
            && let Some(Some(header)) = headers.get(*table)
        {
            used += header.height();
            current.push(header.clone());
        }

        used += height;
        current.push(atom);
    }

    if !current.is_empty() {
        pages.push(current);
    }
    while pages.len() < doc.min_pages.max(1) as usize {
        pages.push(Vec::new());
    }

    (pages, overflowed)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

struct ImageResource {
    name: String,
    id: ObjectId,
}

/// Lay `doc` out and encode it.
#[must_use]
pub fn compose_doc(doc: &Doc) -> Composed {
    let mut flattener = Flattener::new(doc);
    for block in &doc.blocks {
        flattener.push(block);
    }
    let Flattener {
        atoms,
        table_headers,
        ..
    } = flattener;
    let (pages, overflowed_pages) = paginate(atoms, doc, &table_headers);

    let mut pdf = Document::with_version("1.5");
    let pages_id = pdf.new_object_id();

    let font_ids: Vec<ObjectId> = FACES
        .iter()
        .map(|face| {
            pdf.add_object(dictionary! {
              "Type" => "Font",
              "Subtype" => "Type1",
              "BaseFont" => face.base_font(),
              "Encoding" => "WinAnsiEncoding",
            })
        })
        .collect();

    let page_count = pages.len() as u32;
    let mut page_ids = Vec::with_capacity(pages.len());

    for (index, atoms) in pages.iter().enumerate() {
        let page_number = index as u32 + 1;
        let mut painter = Painter::new(doc);
        let mut images = Vec::new();

        painter.draw_watermark();
        for atom in atoms {
            painter.draw(atom, &mut pdf, &mut images);
        }
        painter.draw_furniture(page_number, page_count);

        let content = Content {
            operations: painter.operations,
        };
        let Ok(encoded) = content.encode() else {
            return Composed {
                base64: String::new(),
                overflowed_pages,
                pages: page_count,
            };
        };
        let content_id = pdf.add_object(Stream::new(dictionary! {}, encoded));

        let mut fonts = lopdf::Dictionary::new();
        for (slot, id) in font_ids.iter().enumerate() {
            fonts.set(format!("F{}", slot + 1), Object::Reference(*id));
        }
        let mut resources = dictionary! { "Font" => fonts };
        if !images.is_empty() {
            let mut xobjects = lopdf::Dictionary::new();
            for image in &images {
                xobjects.set(image.name.as_bytes().to_vec(), Object::Reference(image.id));
            }
            resources.set("XObject", xobjects);
        }

        page_ids.push(pdf.add_object(dictionary! {
          "Type" => "Page",
          "Parent" => pages_id,
          "Contents" => content_id,
          "Resources" => resources,
          "MediaBox" => vec![
            0.into(),
            0.into(),
            doc.geometry.width.into(),
            doc.geometry.height.into(),
          ],
        }));
    }

    pdf.objects.insert(
        pages_id,
        dictionary! {
          "Type" => "Pages",
          "Kids" => page_ids.iter().map(|&id| Object::Reference(id)).collect::<Vec<_>>(),
          "Count" => i64::from(page_count),
        }
        .into(),
    );

    let catalog_id = pdf.add_object(dictionary! {
      "Type" => "Catalog",
      "Pages" => pages_id,
    });
    pdf.trailer.set("Root", catalog_id);

    let info_id = pdf.add_object(info_dictionary(&doc.meta));
    pdf.trailer.set("Info", info_id);

    let mut bytes = Vec::new();
    if pdf.save_modern(&mut bytes).is_err() {
        return Composed {
            base64: String::new(),
            overflowed_pages,
            pages: page_count,
        };
    }

    Composed {
        base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
        overflowed_pages,
        pages: page_count,
    }
}

fn info_dictionary(meta: &Meta) -> lopdf::Dictionary {
    let mut info = dictionary! {
      "Producer" => Object::string_literal(win_ansi("ferrimock")),
      "CreationDate" => Object::string_literal(win_ansi(
        &chrono::Utc::now().format("D:%Y%m%d%H%M%SZ").to_string(),
      )),
    };
    let mut set = |key: &str, value: &Option<String>| {
        if let Some(value) = value {
            info.set(key, Object::string_literal(win_ansi(value)));
        }
    };
    set("Title", &meta.title);
    set("Author", &meta.author);
    set("Subject", &meta.subject);
    set("Keywords", &meta.keywords);
    set("Creator", &meta.creator);
    info
}

/// Accumulates the operations for one page and tracks the vertical cursor.
struct Painter<'a> {
    doc: &'a Doc,
    operations: Vec<Operation>,
    y: f64,
}

impl<'a> Painter<'a> {
    fn new(doc: &'a Doc) -> Self {
        Self {
            doc,
            operations: Vec::new(),
            y: doc.geometry.top(),
        }
    }

    fn margin(&self) -> f64 {
        self.doc.geometry.margin
    }

    fn op(&mut self, operator: &str, operands: Vec<Object>) {
        self.operations.push(Operation::new(operator, operands));
    }

    fn set_fill(&mut self, color: Rgb) {
        self.op("rg", vec![color.r.into(), color.g.into(), color.b.into()]);
    }

    fn set_stroke(&mut self, color: Rgb) {
        self.op("RG", vec![color.r.into(), color.g.into(), color.b.into()]);
    }

    fn show(&mut self, text: &str, x: f64, baseline: f64, face: Face, size: f64, color: Rgb) {
        if text.is_empty() {
            return;
        }
        self.op("BT", vec![]);
        self.set_fill(color);
        self.op("Tf", vec![face.resource().into(), size.into()]);
        self.op("Td", vec![x.into(), baseline.into()]);
        self.op("Tj", vec![Object::string_literal(win_ansi(text))]);
        self.op("ET", vec![]);
    }

    fn fill_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: Rgb) {
        self.op("q", vec![]);
        self.set_fill(color);
        self.op("re", vec![x.into(), y.into(), width.into(), height.into()]);
        self.op("f", vec![]);
        self.op("Q", vec![]);
    }

    fn stroke_rect(&mut self, x: f64, y: f64, width: f64, height: f64, color: Rgb, thickness: f64) {
        self.op("q", vec![]);
        self.op("w", vec![thickness.into()]);
        self.set_stroke(color);
        self.op("re", vec![x.into(), y.into(), width.into(), height.into()]);
        self.op("S", vec![]);
        self.op("Q", vec![]);
    }

    fn line(&mut self, from: (f64, f64), to: (f64, f64), color: Rgb, thickness: f64) {
        self.op("q", vec![]);
        self.op("w", vec![thickness.into()]);
        self.set_stroke(color);
        self.op("m", vec![from.0.into(), from.1.into()]);
        self.op("l", vec![to.0.into(), to.1.into()]);
        self.op("S", vec![]);
        self.op("Q", vec![]);
    }

    /// The x a run of `width` starts at inside a box of `available` at `x`.
    fn aligned(x: f64, available: f64, width: f64, align: Align) -> f64 {
        match align {
            Align::Left => x,
            Align::Center => x + (available - width) / 2.0,
            Align::Right => x + available - width,
        }
    }

    fn draw(&mut self, atom: &Atom, pdf: &mut Document, images: &mut Vec<ImageResource>) {
        match atom {
            Atom::Space(height) => self.y -= height,
            Atom::Break => {}
            Atom::Text {
                text,
                x,
                width,
                face,
                size,
                color,
                align,
                height,
                ..
            } => {
                // A zero-height run shares the baseline of the line that
                // follows it, which is how a list marker sits beside its item.
                let baseline = if *height > 0.0 {
                    self.y -= height;
                    self.y
                } else {
                    self.y - size * LEADING
                };
                let run = text_width(text, *face, *size);
                let x = Self::aligned(self.margin() + x, *width, run, *align);
                self.show(text, x, baseline, *face, *size, *color);
            }
            Atom::Rule {
                color,
                thickness,
                width,
                height,
                ..
            } => {
                let y = self.y - height / 2.0;
                self.line(
                    (self.margin(), y),
                    (self.margin() + width, y),
                    *color,
                    *thickness,
                );
                self.y -= height;
            }
            Atom::Row {
                cells,
                widths,
                align,
                face,
                fill,
                grid,
                height,
                ..
            } => {
                let top = self.y;
                self.y -= height;
                let mut x = self.margin();
                for (column, width) in widths.iter().enumerate() {
                    if let Some(fill) = fill {
                        self.fill_rect(x, self.y, *width, *height, *fill);
                    }
                    if *grid {
                        self.stroke_rect(x, self.y, *width, *height, Rgb::RULE, 0.5);
                    }
                    let Some(cell) = cells.get(column) else {
                        x += width;
                        continue;
                    };
                    let available = 2.0f64.mul_add(-CELL_PADDING, *width);
                    // One line per cell: a table that needs reflow inside a
                    // cell wants a different layout, not a taller row here.
                    let text = wrap(cell, available, *face, TABLE_SIZE)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let run = text_width(&text, *face, TABLE_SIZE);
                    let alignment = align.get(column).copied().unwrap_or_default();
                    let text_x = Self::aligned(x + CELL_PADDING, available, run, alignment);
                    self.show(
                        &text,
                        text_x,
                        top - CELL_PADDING - TABLE_SIZE,
                        *face,
                        TABLE_SIZE,
                        Rgb::TEXT,
                    );
                    x += width;
                }
            }
            Atom::Panel {
                lines,
                tint,
                height,
            } => {
                self.y -= height;
                let width = self.doc.geometry.body_width();
                self.fill_rect(self.margin(), self.y, width, *height, *tint);
                self.fill_rect(self.margin(), self.y, 3.0, *height, tint.tint(-0.35));
                let mut baseline =
                    BODY_SIZE.mul_add(-0.85, 2.0f64.mul_add(-CELL_PADDING, self.y + height));
                for (line, face) in lines {
                    self.show(
                        line,
                        2.0f64.mul_add(CELL_PADDING, self.margin()),
                        baseline,
                        *face,
                        BODY_SIZE,
                        Rgb::TEXT,
                    );
                    baseline = BODY_SIZE.mul_add(-LEADING, baseline);
                }
            }
            Atom::Columns {
                columns,
                face,
                size,
                column_width,
                gutter,
                height,
            } => {
                let top = self.y;
                self.y -= height;
                for (index, lines) in columns.iter().enumerate() {
                    let x = (index as f64).mul_add(column_width + gutter, self.margin());
                    let mut baseline = top - size * LEADING;
                    for line in lines {
                        self.show(line, x, baseline, *face, *size, Rgb::TEXT);
                        baseline -= size * LEADING;
                    }
                }
            }
            Atom::Image {
                jpeg,
                pixels,
                draw,
                x,
                height,
            } => {
                let id = pdf.add_object(Stream::new(
                    dictionary! {
                      "Type" => "XObject",
                      "Subtype" => "Image",
                      "Width" => i64::from(pixels.0),
                      "Height" => i64::from(pixels.1),
                      "ColorSpace" => "DeviceRGB",
                      "BitsPerComponent" => 8,
                      "Filter" => "DCTDecode",
                    },
                    jpeg.clone(),
                ));
                let name = format!("Im{}", images.len() + 1);
                self.y -= height;
                self.op("q", vec![]);
                self.op(
                    "cm",
                    vec![
                        draw.0.into(),
                        0.into(),
                        0.into(),
                        draw.1.into(),
                        (self.margin() + x).into(),
                        self.y.into(),
                    ],
                );
                self.op("Do", vec![Object::Name(name.clone().into())]);
                self.op("Q", vec![]);
                images.push(ImageResource { name, id });
            }
            Atom::Chart { chart, height } => {
                self.y -= height;
                self.draw_chart(chart, self.y, *height);
            }
        }
    }

    fn draw_chart(&mut self, chart: &Chart, bottom: f64, total_height: f64) {
        let width = self.doc.geometry.body_width();
        let left = self.margin();
        let mut plot_height = total_height;

        if let Some(title) = &chart.title {
            self.show(
                title,
                left,
                bottom + total_height - 12.0,
                Face::REGULAR.in_family(self.doc.family).bold(),
                11.0,
                Rgb::TEXT,
            );
            plot_height -= 18.0;
        }

        let max = chart
            .values
            .iter()
            .copied()
            .fold(f64::MIN_POSITIVE, f64::max);
        let count = chart.values.len();
        if count == 0 || plot_height <= 0.0 {
            return;
        }

        // Room for the category labels under the plot.
        let label_band = 12.0;
        let plot = plot_height - label_band;
        let base = bottom + label_band;

        match chart.kind {
            ChartKind::Pie => {
                self.draw_pie(chart, left + width / 2.0, base + plot / 2.0, plot / 2.0);
            }
            ChartKind::Bar => {
                let slot = width / count as f64;
                let bar = slot * 0.62;
                for (index, value) in chart.values.iter().enumerate() {
                    let h = (value / max) * plot;
                    let x = (index as f64).mul_add(slot, left) + (slot - bar) / 2.0;
                    self.fill_rect(x, base, bar, h, chart.accent);
                    if let Some(label) = chart.labels.get(index) {
                        let run = text_width(label, Face::REGULAR, 7.5);
                        self.show(
                            label,
                            x + (bar - run) / 2.0,
                            bottom + 3.0,
                            Face::REGULAR,
                            7.5,
                            Rgb::MUTED,
                        );
                    }
                }
                self.line((left, base), (left + width, base), Rgb::RULE, 0.7);
            }
            ChartKind::Line | ChartKind::Area => {
                let step = if count > 1 {
                    width / (count - 1) as f64
                } else {
                    0.0
                };
                let point = |index: usize, value: f64| {
                    (
                        (index as f64).mul_add(step, left),
                        (value / max).mul_add(plot, base),
                    )
                };

                if chart.kind == ChartKind::Area {
                    self.op("q", vec![]);
                    self.set_fill(chart.accent.tint(0.72));
                    self.op("m", vec![left.into(), base.into()]);
                    for (index, value) in chart.values.iter().enumerate() {
                        let (x, y) = point(index, *value);
                        self.op("l", vec![x.into(), y.into()]);
                    }
                    self.op("l", vec![(index_x(count, step, left)).into(), base.into()]);
                    self.op("h", vec![]);
                    self.op("f", vec![]);
                    self.op("Q", vec![]);
                }

                self.op("q", vec![]);
                self.op("w", vec![1.6.into()]);
                self.set_stroke(chart.accent);
                for (index, value) in chart.values.iter().enumerate() {
                    let (x, y) = point(index, *value);
                    self.op(if index == 0 { "m" } else { "l" }, vec![x.into(), y.into()]);
                }
                self.op("S", vec![]);
                self.op("Q", vec![]);

                self.line((left, base), (left + width, base), Rgb::RULE, 0.7);
                for (index, label) in chart.labels.iter().enumerate().take(count) {
                    let (x, _) = point(index, 0.0);
                    let run = text_width(label, Face::REGULAR, 7.5);
                    self.show(
                        label,
                        x - run / 2.0,
                        bottom + 3.0,
                        Face::REGULAR,
                        7.5,
                        Rgb::MUTED,
                    );
                }
            }
        }
    }

    fn draw_pie(&mut self, chart: &Chart, cx: f64, cy: f64, radius: f64) {
        let total: f64 = chart.values.iter().sum();
        if total <= 0.0 || radius <= 0.0 {
            return;
        }
        let mut start = 0.0f64;
        for (index, value) in chart.values.iter().enumerate() {
            let sweep = value / total * std::f64::consts::TAU;
            let shade = chart.accent.tint(0.12 * index as f64 % 0.8);
            self.op("q", vec![]);
            self.set_fill(shade);
            self.op("m", vec![cx.into(), cy.into()]);
            // A circular arc has no PDF operator, so walk it as a polyline
            // fine enough that the straight segments do not read as facets.
            let steps = ((sweep / std::f64::consts::TAU) * 120.0).ceil().max(2.0) as usize;
            for step in 0..=steps {
                let angle = (step as f64 / steps as f64).mul_add(sweep, start);
                let x = radius.mul_add(angle.cos(), cx);
                let y = radius.mul_add(angle.sin(), cy);
                self.op("l", vec![x.into(), y.into()]);
            }
            self.op("h", vec![]);
            self.op("f", vec![]);
            self.op("Q", vec![]);
            start += sweep;
        }
    }

    fn draw_watermark(&mut self) {
        let Some(text) = &self.doc.watermark else {
            return;
        };
        let size = 62.0;
        let face = Face::REGULAR.in_family(self.doc.family).bold();
        let run = text_width(text, face, size);
        let cx = self.doc.geometry.width / 2.0;
        let cy = self.doc.geometry.height / 2.0;
        let angle = std::f64::consts::FRAC_PI_4;

        self.op("q", vec![]);
        self.op("BT", vec![]);
        self.set_fill(Rgb::gray(0.88));
        self.op("Tf", vec![face.resource().into(), size.into()]);
        self.op(
            "Tm",
            vec![
                angle.cos().into(),
                angle.sin().into(),
                (-angle.sin()).into(),
                angle.cos().into(),
                (run / 2.0).mul_add(-angle.cos(), cx).into(),
                (run / 2.0).mul_add(-angle.sin(), cy).into(),
            ],
        );
        self.op("Tj", vec![Object::string_literal(win_ansi(text))]);
        self.op("ET", vec![]);
        self.op("Q", vec![]);
    }

    fn draw_furniture(&mut self, page_number: u32, page_count: u32) {
        let margin = self.margin();
        let width = self.doc.geometry.body_width();
        let face = Face::REGULAR.in_family(self.doc.family);

        if let Some(header) = &self.doc.header {
            let y = self.doc.geometry.height - margin + 14.0;
            self.show(header, margin, y, face, FOOTER_SIZE, Rgb::MUTED);
            self.line((margin, y - 5.0), (margin + width, y - 5.0), Rgb::RULE, 0.5);
        }

        if let Some(footer) = &self.doc.footer {
            self.show(footer, margin, FOOTER_TOP, face, FOOTER_SIZE, Rgb::MUTED);
        }

        if self.doc.page_numbers {
            let label = format!("Page {page_number} of {page_count}");
            let run = text_width(&label, face, FOOTER_SIZE);
            self.show(
                &label,
                self.doc.geometry.width - margin - run,
                FOOTER_TOP,
                face,
                FOOTER_SIZE,
                Rgb::MUTED,
            );
        }
    }
}

/// Right edge of the last plotted point, for closing an area fill.
fn index_x(count: usize, step: f64, left: f64) -> f64 {
    (count.saturating_sub(1) as f64).mul_add(step, left)
}

/// Lay `pages` out, one entry per page, and encode the document.
///
/// Kept for callers that page their own content; `compose_doc` flows instead.
#[must_use]
pub fn compose(pages: &[Vec<Block>]) -> Composed {
    let mut blocks = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        if index > 0 {
            blocks.push(Block::PageBreak);
        }
        blocks.extend(page.iter().cloned());
    }
    compose_doc(&Doc {
        blocks,
        min_pages: pages.len() as u32,
        ..Doc::default()
    })
}

/// Re-encode a base64 PNG as JPEG so it can go straight into a `DCTDecode`
/// image XObject. Returns the bytes and the pixel dimensions.
fn jpeg_from_png_base64(png_base64: &str) -> Option<(Vec<u8>, u32, u32)> {
    use base64::Engine as _;

    let png = base64::engine::general_purpose::STANDARD
        .decode(png_base64)
        .ok()?;
    let decoded = image::load_from_memory(&png).ok()?.to_rgb8();
    let (width, height) = decoded.dimensions();

    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode(
            decoded.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgb8,
        )
        .ok()?;

    Some((jpeg, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str, width: f64) -> Vec<String> {
        wrap(text, width, Face::REGULAR, BODY_SIZE)
    }

    #[test]
    fn wrapped_lines_fit_the_column() {
        let text = "The quick brown fox jumps over the lazy dog and keeps on running well past \
                    the right hand edge of any sensible text column.";
        for line in lines(text, body_width()) {
            assert!(text_width(&line, Face::REGULAR, BODY_SIZE) <= body_width());
        }
    }

    #[test]
    fn a_word_wider_than_the_column_is_split() {
        let wide = "x".repeat(400);
        let broken = lines(&wide, 100.0);
        assert!(broken.len() > 1);
        for line in broken {
            assert!(text_width(&line, Face::REGULAR, BODY_SIZE) <= 100.0);
        }
    }

    #[test]
    fn every_face_has_its_own_resource_slot() {
        let mut seen = std::collections::HashSet::new();
        for face in FACES {
            assert!(seen.insert(face.slot()), "{face:?} collides");
            assert_eq!(FACES.get(face.slot()).copied(), Some(face));
        }
    }

    #[test]
    fn courier_is_monospaced_and_times_is_not() {
        let courier = Face::REGULAR.in_family(Family::Courier);
        assert_eq!(
            text_width("iii", courier, 10.0),
            text_width("WWW", courier, 10.0)
        );
        let times = Face::REGULAR.in_family(Family::Times);
        assert!(text_width("iii", times, 10.0) < text_width("WWW", times, 10.0));
    }

    #[test]
    fn long_content_flows_onto_more_pages() {
        let doc = Doc {
            blocks: (0..80)
                .map(|n| Block::Paragraph(format!("Paragraph {n} of a document that keeps going.")))
                .collect(),
            ..Doc::default()
        };
        let composed = compose_doc(&doc);
        assert!(composed.pages > 1, "got {} pages", composed.pages);
        assert!(composed.overflowed_pages.is_empty());
    }

    #[test]
    fn a_broken_table_repeats_its_header() {
        let table = Table::new(
            vec!["Item".into(), "Value".into()],
            (0..90)
                .map(|n| vec![format!("row {n}"), n.to_string()])
                .collect(),
        );
        let doc = Doc {
            blocks: vec![Block::Table(table)],
            ..Doc::default()
        };
        let mut flattener = Flattener::new(&doc);
        for block in &doc.blocks {
            flattener.push(block);
        }
        let headers = flattener.table_headers.clone();
        let (pages, _) = paginate(flattener.atoms, &doc, &headers);
        assert!(pages.len() > 1);
        for page in &pages {
            let Some(Atom::Row { cells, .. }) = page.first() else {
                continue;
            };
            assert_eq!(cells.first().map(String::as_str), Some("Item"));
        }
    }

    #[test]
    fn a_page_break_starts_a_new_page() {
        let doc = Doc {
            blocks: vec![
                Block::Paragraph("first".into()),
                Block::PageBreak,
                Block::Paragraph("second".into()),
            ],
            ..Doc::default()
        };
        assert_eq!(compose_doc(&doc).pages, 2);
    }

    #[test]
    fn hex_colours_parse_in_both_lengths() {
        assert_eq!(Rgb::parse("#000"), Some(Rgb::gray(0.0)));
        assert_eq!(Rgb::parse("ffffff"), Some(Rgb::gray(1.0)));
        assert_eq!(Rgb::parse("nope"), None);
    }

    #[test]
    fn landscape_is_the_portrait_box_turned() {
        let portrait = Geometry::new(PageSize::A4, Orientation::Portrait);
        let landscape = Geometry::new(PageSize::A4, Orientation::Landscape);
        assert_eq!(
            (portrait.width, portrait.height),
            (landscape.height, landscape.width)
        );
    }

    #[test]
    fn win_ansi_maps_smart_punctuation_to_one_byte() {
        assert_eq!(win_ansi("\u{2019}"), vec![0x92]);
        assert_eq!(win_ansi("n\u{e9}e"), vec![b'n', 0xe9, b'e']);
        assert_eq!(win_ansi("\u{4e2d}"), vec![b'?']);
    }

    #[test]
    fn a_composed_document_starts_with_the_pdf_header() {
        use base64::Engine as _;
        let composed = compose_doc(&Doc {
            blocks: vec![Block::Heading {
                text: "Title".into(),
                level: 1,
            }],
            meta: Meta {
                title: Some("Title".into()),
                ..Meta::default()
            },
            ..Doc::default()
        });
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&composed.base64)
            .unwrap_or_default();
        assert!(bytes.starts_with(b"%PDF-"));
    }
}
