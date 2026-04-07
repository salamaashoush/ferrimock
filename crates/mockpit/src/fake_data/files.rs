//! File generation (PDF, PNG, JPEG)

use image::Rgba;
use rand::RngExt;
use rand::seq::IndexedRandom;
use rust_embed::Embed;

/// Embedded font assets
#[derive(Embed)]
#[folder = "assets/"]
struct EmbeddedAssets;

/// Generate a fake PDF document with optional text content and page count
/// Returns base64-encoded PDF data
///
/// # Panics
/// Panics if PDF encoding fails (very unlikely with valid inputs)
pub fn fake_pdf(text: Option<&str>, pages: Option<u32>) -> String {
    use lopdf::{
        Document, Object, Stream,
        content::{Content, Operation},
        dictionary,
    };

    // Create a new PDF document
    let mut doc = Document::with_version("1.5");

    // Add text content if provided, otherwise use placeholder
    let content_text =
        text.unwrap_or("This is a fake PDF document generated for testing purposes.");

    // Add footer with generation timestamp
    let footer = format!(
        "Generated: {}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    );

    // Number of pages to generate (default: 1)
    let page_count = pages.unwrap_or(1).max(1);

    // Add pages catalog
    let pages_id = doc.new_object_id();

    // Create font object (Helvetica)
    let font_id = doc.add_object(dictionary! {
      "Type" => "Font",
      "Subtype" => "Type1",
      "BaseFont" => "Helvetica",
    });

    let mut page_ids = Vec::new();

    // Generate pages
    for page_num in 1..=page_count {
        // Create page content stream
        let mut operations = Vec::new();

        // Begin text
        operations.push(Operation::new("BT", vec![]));

        // Set font (Helvetica, size 12)
        operations.push(Operation::new("Tf", vec!["F1".into(), 12.into()]));

        // Set text position and write content lines
        let lines: Vec<&str> = content_text.lines().collect();
        let start_y = 750.0; // Start from top
        let line_spacing = 15.0;

        // Set initial position for first line
        operations.push(Operation::new("Td", vec![30.into(), start_y.into()]));

        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                // Move down by line_spacing for subsequent lines (relative positioning)
                operations.push(Operation::new("Td", vec![0.into(), (-line_spacing).into()]));
            }
            operations.push(Operation::new(
                "Tj",
                vec![Object::string_literal((*line).to_string())],
            ));
        }

        // Calculate how far down we are after content
        let content_end_y = (lines.len() as f64).mul_add(-line_spacing, start_y);
        let footer_y = 50.0;
        let move_to_footer = footer_y - content_end_y;

        // Page number (smaller font)
        operations.push(Operation::new("Tf", vec!["F1".into(), 10.into()]));
        operations.push(Operation::new("Td", vec![0.into(), move_to_footer.into()]));
        operations.push(Operation::new(
            "Tj",
            vec![Object::string_literal(format!(
                "Page {page_num} of {page_count}"
            ))],
        ));

        // Footer (even smaller font) - move down 20 points
        operations.push(Operation::new("Tf", vec!["F1".into(), 8.into()]));
        operations.push(Operation::new("Td", vec![0.into(), (-20.0).into()]));
        operations.push(Operation::new(
            "Tj",
            vec![Object::string_literal(footer.clone())],
        ));

        // End text
        operations.push(Operation::new("ET", vec![]));

        // Create content stream
        let content = Content { operations };
        let Ok(content_data) = content.encode() else {
            return String::new();
        };

        let content_id = doc.add_object(Stream::new(dictionary! {}, content_data));

        // Create page object
        let page_id = doc.add_object(dictionary! {
          "Type" => "Page",
          "Parent" => pages_id,
          "Contents" => content_id,
          "Resources" => dictionary! {
            "Font" => dictionary! {
              "F1" => font_id,
            },
          },
          "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()], // A4 size
        });

        page_ids.push(page_id);
    }

    // Create pages dictionary
    doc.objects.insert(
        pages_id,
        dictionary! {
          "Type" => "Pages",
          "Kids" => page_ids.iter().map(|&id| Object::Reference(id)).collect::<Vec<_>>(),
          "Count" => (page_count as i64),
        }
        .into(),
    );

    // Create catalog
    let catalog_id = doc.add_object(dictionary! {
      "Type" => "Catalog",
      "Pages" => pages_id,
    });

    doc.trailer.set("Root", catalog_id);

    // Save to bytes using modern PDF 1.5+ format (object streams + xref streams)
    // for 11-61% smaller output -- significant when base64-encoded for mock responses.
    let mut pdf_bytes = Vec::new();
    if doc.save_modern(&mut pdf_bytes).is_err() {
        return String::new();
    }

    // Return base64-encoded PDF
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &pdf_bytes)
}

/// Generate a fake PNG image with specified dimensions and optional color
/// Returns base64-encoded PNG data
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_png(width: Option<u32>, height: Option<u32>, color: Option<&str>) -> String {
    use image::{ImageBuffer, Rgb};

    let w = width.unwrap_or(800);
    let h = height.unwrap_or(600);

    // Parse color or use random
    let rgb = parse_color_or_random(color);

    // Create image buffer with solid color
    let img = ImageBuffer::from_fn(w, h, |_, _| Rgb(rgb));

    // Encode to PNG and then base64
    let mut png_bytes = Vec::new();
    if img
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png_bytes)
}

/// Generate a fake JPEG image with specified dimensions and optional color
/// Returns base64-encoded JPEG data
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_jpeg(
    width: Option<u32>,
    height: Option<u32>,
    color: Option<&str>,
    quality: Option<u8>,
) -> String {
    use image::{ImageBuffer, Rgb};

    let w = width.unwrap_or(800);
    let h = height.unwrap_or(600);
    let q = quality.unwrap_or(85);

    // Parse color or use random
    let rgb = parse_color_or_random(color);

    // Create image buffer with solid color
    let img = ImageBuffer::from_fn(w, h, |_, _| Rgb(rgb));

    // Encode to JPEG with quality setting
    let mut jpeg_bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, q);
    if encoder
        .encode(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes)
}

/// Generate a fake PDF as a data URI
pub fn fake_pdf_data_uri(text: Option<&str>, pages: Option<u32>) -> String {
    let base64_data = fake_pdf(text, pages);
    format!("data:application/pdf;base64,{base64_data}")
}

/// Generate a fake PNG as a data URI
pub fn fake_png_data_uri(width: Option<u32>, height: Option<u32>, color: Option<&str>) -> String {
    let base64_data = fake_png(width, height, color);
    format!("data:image/png;base64,{base64_data}")
}

/// Generate a fake JPEG as a data URI
pub fn fake_jpeg_data_uri(
    width: Option<u32>,
    height: Option<u32>,
    color: Option<&str>,
    quality: Option<u8>,
) -> String {
    let base64_data = fake_jpeg(width, height, color, quality);
    format!("data:image/jpeg;base64,{base64_data}")
}

/// Convert a base64-encoded PNG to base64-encoded JPEG
///
/// Takes a PNG image (as base64 string) and converts it to JPEG format.
/// Useful for converting any PNG-generating function output to JPEG.
///
/// # Arguments
/// * `png_base64` - Base64-encoded PNG image data
/// * `quality` - JPEG quality (1-100), defaults to 85
///
/// # Returns
/// Base64-encoded JPEG image data, or the original PNG if conversion fails
pub fn png_to_jpeg(png_base64: &str, quality: Option<u8>) -> String {
    let q = quality.unwrap_or(85).clamp(1, 100);

    // Decode base64 PNG
    let Ok(png_bytes) =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, png_base64)
    else {
        return png_base64.to_string();
    };

    // Load image from PNG bytes
    let Ok(img) = image::load_from_memory(&png_bytes) else {
        return png_base64.to_string();
    };

    // Encode to JPEG
    let mut jpeg_bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, q);
    if encoder.encode_image(&img).is_err() {
        return png_base64.to_string();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes)
}

/// Generate a PNG image with text overlay
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_image_with_text(
    text: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
    bg_color: Option<&str>,
    text_color: Option<&str>,
    font_size: Option<f32>,
) -> String {
    use ab_glyph::{FontRef, PxScale};
    use image::ImageBuffer;
    use imageproc::drawing::draw_text_mut;

    let width = width.unwrap_or(400);
    let height = height.unwrap_or(300);
    let text = text.unwrap_or("Sample Image");
    let bg_color = parse_color(bg_color.unwrap_or("#CCCCCC"));
    let text_color = parse_color(text_color.unwrap_or("#333333"));
    let font_size = font_size.unwrap_or(24.0);

    // Create image with background color
    let mut img = ImageBuffer::from_pixel(width, height, bg_color);

    // Load embedded font
    let Some(asset) = EmbeddedAssets::get("NotoSans-Regular.ttf") else {
        return String::new();
    };
    let font_data = asset.data.into_owned();
    let Ok(font) = FontRef::try_from_slice(&font_data) else {
        return String::new();
    };

    // Calculate text position (centered)
    let scale = PxScale::from(font_size);
    let text_width = text.len() as f32 * font_size * 0.6;
    let x = ((width as f32 - text_width) / 2.0).max(10.0) as i32;
    let y = ((height as f32 - font_size) / 2.0).max(10.0) as i32;

    // Draw text
    draw_text_mut(&mut img, text_color, x, y, scale, &font, text);

    // Add image dimensions label at bottom
    let dim_text = format!("{width}x{height}");
    let dim_scale = PxScale::from(14.0);
    let dim_x = 10;
    let dim_y = height as i32 - 25;
    draw_text_mut(
        &mut img, text_color, dim_x, dim_y, dim_scale, &font, &dim_text,
    );

    // Encode to PNG
    let mut buffer = Vec::new();
    if img
        .write_to(
            &mut std::io::Cursor::new(&mut buffer),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, buffer)
}

/// Generate a PNG image with a gradient pattern
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_image_gradient(
    width: Option<u32>,
    height: Option<u32>,
    start_color: Option<&str>,
    end_color: Option<&str>,
    direction: Option<&str>,
) -> String {
    use image::ImageBuffer;

    let width = width.unwrap_or(400);
    let height = height.unwrap_or(300);
    let start = parse_color(start_color.unwrap_or("#FF0000"));
    let end = parse_color(end_color.unwrap_or("#0000FF"));
    let direction = direction.unwrap_or("horizontal");

    // Helper to interpolate between two u8 colors
    let interpolate = |start: u8, end: u8, factor: f32| -> u8 {
        let start_f = f32::from(start);
        let end_f = f32::from(end);
        let result = (end_f - start_f).mul_add(factor, start_f).round();
        // Safe: clamped to [0.0, 255.0], so sign is always positive and value fits in u8
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        {
            result.clamp(0.0, 255.0) as u8
        }
    };

    let img = ImageBuffer::from_fn(width, height, |x, y| {
        let factor = match direction {
            "vertical" => y as f32 / height as f32,
            "diagonal" => ((x + y) as f32) / ((width + height) as f32),
            _ => x as f32 / width as f32,
        };

        Rgba([
            interpolate(start[0], end[0], factor),
            interpolate(start[1], end[1], factor),
            interpolate(start[2], end[2], factor),
            255,
        ])
    });

    let mut buffer = Vec::new();
    if img
        .write_to(
            &mut std::io::Cursor::new(&mut buffer),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, buffer)
}

/// Generate an image with a checkerboard pattern
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_image_checkerboard(
    width: Option<u32>,
    height: Option<u32>,
    color1: Option<&str>,
    color2: Option<&str>,
    square_size: Option<u32>,
) -> String {
    use image::ImageBuffer;

    let width = width.unwrap_or(400);
    let height = height.unwrap_or(300);
    let color1 = parse_color(color1.unwrap_or("#000000"));
    let color2 = parse_color(color2.unwrap_or("#FFFFFF"));
    let square_size = square_size.unwrap_or(20);

    let img = ImageBuffer::from_fn(width, height, |x, y| {
        let checker_x = (x / square_size) % 2;
        let checker_y = (y / square_size) % 2;
        if (checker_x + checker_y).is_multiple_of(2) {
            color1
        } else {
            color2
        }
    });

    let mut buffer = Vec::new();
    if img
        .write_to(
            &mut std::io::Cursor::new(&mut buffer),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, buffer)
}

/// Generate an image with random noise pattern
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_image_noise(width: Option<u32>, height: Option<u32>, colored: Option<bool>) -> String {
    use image::ImageBuffer;

    let width = width.unwrap_or(400);
    let height = height.unwrap_or(300);
    let colored = colored.unwrap_or(false);
    let mut rng = rand::rng();

    let img = ImageBuffer::from_fn(width, height, |_x, _y| {
        use rand::RngExt;
        if colored {
            Rgba([
                rng.random::<u8>(),
                rng.random::<u8>(),
                rng.random::<u8>(),
                255,
            ])
        } else {
            let gray = rng.random::<u8>();
            Rgba([gray, gray, gray, 255])
        }
    });

    let mut buffer = Vec::new();
    if img
        .write_to(
            &mut std::io::Cursor::new(&mut buffer),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, buffer)
}

/// Generate an image with striped pattern
///
/// # Panics
/// Panics if image encoding fails or if dimensions are invalid
pub fn fake_image_stripes(
    width: Option<u32>,
    height: Option<u32>,
    color1: Option<&str>,
    color2: Option<&str>,
    stripe_width: Option<u32>,
    direction: Option<&str>,
) -> String {
    use image::ImageBuffer;

    let width = width.unwrap_or(400);
    let height = height.unwrap_or(300);
    let color1 = parse_color(color1.unwrap_or("#FF0000"));
    let color2 = parse_color(color2.unwrap_or("#0000FF"));
    let stripe_width = stripe_width.unwrap_or(20);
    let direction = direction.unwrap_or("horizontal");

    let img = ImageBuffer::from_fn(width, height, |x, y| {
        let pos = match direction {
            "vertical" => x,
            _ => y,
        };
        if (pos / stripe_width).is_multiple_of(2) {
            color1
        } else {
            color2
        }
    });

    let mut buffer = Vec::new();
    if img
        .write_to(
            &mut std::io::Cursor::new(&mut buffer),
            image::ImageFormat::Png,
        )
        .is_err()
    {
        return String::new();
    }

    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, buffer)
}

/// Generate a placeholder image with centered text
pub fn fake_placeholder(
    width: Option<u32>,
    height: Option<u32>,
    text: Option<&str>,
    bg_color: Option<&str>,
    text_color: Option<&str>,
) -> String {
    let width = width.unwrap_or(400);
    let height = height.unwrap_or(300);
    let text = text.map_or_else(
        || format!("{width}x{height}"),
        std::string::ToString::to_string,
    );

    fake_image_with_text(
        Some(&text),
        Some(width),
        Some(height),
        bg_color,
        text_color,
        Some(32.0),
    )
}

/// Generate an avatar placeholder with initials
pub fn fake_avatar(
    initials: Option<&str>,
    size: Option<u32>,
    bg_color: Option<&str>,
    text_color: Option<&str>,
) -> String {
    let mut rng = rand::rng();

    let size = size.unwrap_or(200);
    let default_initials = "AB";
    let initials = initials.unwrap_or(default_initials);

    // Generate random pastel background color if not provided
    let bg_color = bg_color.unwrap_or_else(|| {
        let colors = [
            "#FF6B6B", "#4ECDC4", "#45B7D1", "#FFA07A", "#98D8C8", "#F7DC6F", "#BB8FCE",
        ];
        colors.choose(&mut rng).copied().unwrap_or("#4ECDC4")
    });

    fake_image_with_text(
        Some(initials),
        Some(size),
        Some(size),
        Some(bg_color),
        text_color,
        Some(size as f32 * 0.4),
    )
}

// Helper functions

/// Helper to parse color string or return random color
fn parse_color_or_random(color: Option<&str>) -> [u8; 3] {
    match color {
        Some(c) => {
            if let Some(hex) = c.strip_prefix('#') {
                if hex.len() == 6 {
                    let r =
                        u8::from_str_radix(hex.get(0..2).unwrap_or_default(), 16).unwrap_or(128);
                    let g =
                        u8::from_str_radix(hex.get(2..4).unwrap_or_default(), 16).unwrap_or(128);
                    let b =
                        u8::from_str_radix(hex.get(4..6).unwrap_or_default(), 16).unwrap_or(128);
                    return [r, g, b];
                } else if hex.len() == 3 {
                    let r = u8::from_str_radix(&hex.get(0..1).unwrap_or_default().repeat(2), 16)
                        .unwrap_or(128);
                    let g = u8::from_str_radix(&hex.get(1..2).unwrap_or_default().repeat(2), 16)
                        .unwrap_or(128);
                    let b = u8::from_str_radix(&hex.get(2..3).unwrap_or_default().repeat(2), 16)
                        .unwrap_or(128);
                    return [r, g, b];
                }
            }
            random_color_rgb()
        }
        None => random_color_rgb(),
    }
}

/// Generate random RGB color
fn random_color_rgb() -> [u8; 3] {
    let mut rng = rand::rng();
    [
        rng.random_range(0..=255),
        rng.random_range(0..=255),
        rng.random_range(0..=255),
    ]
}

/// Helper to parse hex color to RGBA
fn parse_color(hex: &str) -> image::Rgba<u8> {
    use image::Rgba;

    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(hex.get(0..2).unwrap_or_default(), 16).unwrap_or(0);
    let g = u8::from_str_radix(hex.get(2..4).unwrap_or_default(), 16).unwrap_or(0);
    let b = u8::from_str_radix(hex.get(4..6).unwrap_or_default(), 16).unwrap_or(0);
    Rgba([r, g, b, 255])
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_pdf() {
        let pdf = fake_pdf(None, None);
        assert!(!pdf.is_empty());
        assert!(
            pdf.chars()
                .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
        );

        let custom_pdf = fake_pdf(Some("Test content"), None);
        assert!(!custom_pdf.is_empty());
        assert_ne!(pdf, custom_pdf);
    }

    #[test]
    fn test_fake_pdf_multipage() {
        let single_page = fake_pdf(None, Some(1));
        assert!(!single_page.is_empty());

        let multi_page = fake_pdf(None, Some(5));
        assert!(!multi_page.is_empty());
        assert!(multi_page.len() > single_page.len());

        let custom_multi = fake_pdf(Some("Page content"), Some(3));
        assert!(!custom_multi.is_empty());

        let zero_pages = fake_pdf(None, Some(0));
        assert!(!zero_pages.is_empty());
    }

    #[test]
    fn test_fake_png() {
        let png = fake_png(None, None, None);
        assert!(!png.is_empty());
        assert!(
            png.chars()
                .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
        );

        let custom_png = fake_png(Some(100), Some(100), Some("#FF0000"));
        assert!(!custom_png.is_empty());
    }

    #[test]
    fn test_fake_jpeg() {
        let jpeg = fake_jpeg(None, None, None, None);
        assert!(!jpeg.is_empty());
        assert!(
            jpeg.chars()
                .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
        );

        let custom_jpeg = fake_jpeg(Some(200), Some(150), Some("#00FF00"), Some(90));
        assert!(!custom_jpeg.is_empty());
    }

    #[test]
    fn test_fake_pdf_data_uri() {
        let uri = fake_pdf_data_uri(None, None);
        assert!(uri.starts_with("data:application/pdf;base64,"));
        assert!(uri.len() > 30);

        let custom_uri = fake_pdf_data_uri(Some("Custom PDF text"), None);
        assert!(custom_uri.starts_with("data:application/pdf;base64,"));

        let multi_uri = fake_pdf_data_uri(None, Some(3));
        assert!(multi_uri.starts_with("data:application/pdf;base64,"));
        assert!(multi_uri.len() > uri.len());
    }

    #[test]
    fn test_fake_png_data_uri() {
        let uri = fake_png_data_uri(None, None, None);
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(uri.len() > 30);

        let custom_uri = fake_png_data_uri(Some(50), Some(50), Some("#0000FF"));
        assert!(custom_uri.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_fake_jpeg_data_uri() {
        let uri = fake_jpeg_data_uri(None, None, None, None);
        assert!(uri.starts_with("data:image/jpeg;base64,"));
        assert!(uri.len() > 30);

        let custom_uri = fake_jpeg_data_uri(Some(100), Some(100), Some("#FFFF00"), Some(80));
        assert!(custom_uri.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn test_parse_color_hex_6digit() {
        let rgb = parse_color_or_random(Some("#FF0000"));
        assert_eq!(rgb, [255, 0, 0]);

        let rgb = parse_color_or_random(Some("#00FF00"));
        assert_eq!(rgb, [0, 255, 0]);

        let rgb = parse_color_or_random(Some("#0000FF"));
        assert_eq!(rgb, [0, 0, 255]);
    }

    #[test]
    fn test_parse_color_hex_3digit() {
        let rgb = parse_color_or_random(Some("#F00"));
        assert_eq!(rgb, [255, 0, 0]);

        let rgb = parse_color_or_random(Some("#0F0"));
        assert_eq!(rgb, [0, 255, 0]);

        let rgb = parse_color_or_random(Some("#00F"));
        assert_eq!(rgb, [0, 0, 255]);
    }

    #[test]
    fn test_parse_color_invalid() {
        // Invalid color should fall back to random RGB values
        let rgb = parse_color_or_random(Some("invalid"));
        // Just verify it returns a valid array (u8 values are always 0-255)
        assert_eq!(rgb.len(), 3);

        let rgb = parse_color_or_random(Some("#GG00FF"));
        assert_eq!(rgb.len(), 3);
    }

    #[test]
    fn test_random_color_rgb() {
        // Verify random_color_rgb returns a valid RGB array
        let rgb = random_color_rgb();
        // Just verify it returns a valid array (u8 values are always 0-255)
        assert_eq!(rgb.len(), 3);
    }

    #[test]
    fn test_embedded_font_loads() {
        use ab_glyph::FontRef;

        let font_data = EmbeddedAssets::get("NotoSans-Regular.ttf")
            .expect("NotoSans-Regular.ttf not found in embedded assets");
        let font_bytes = font_data.data.into_owned();
        assert!(
            font_bytes.len() > 100_000,
            "Font file should be at least 100KB, got {} bytes",
            font_bytes.len()
        );

        let font = FontRef::try_from_slice(&font_bytes);
        assert!(
            font.is_ok(),
            "FontRef::try_from_slice should succeed on embedded font, got: {:?}",
            font.err()
        );
    }

    #[test]
    fn test_fake_image_with_text() {
        let result = fake_image_with_text(Some("Test"), Some(200), Some(100), None, None, None);
        assert!(
            !result.is_empty(),
            "fake_image_with_text should return non-empty base64"
        );

        // Decode and verify PNG magic bytes
        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &result)
            .expect("Should be valid base64");
        assert_eq!(
            &bytes[0..8],
            &[137, 80, 78, 71, 13, 10, 26, 10],
            "Should be a valid PNG"
        );
    }

    #[test]
    fn test_fake_avatar() {
        let result = fake_avatar(Some("SA"), Some(100), Some("#FF0000"), Some("#FFFFFF"));
        assert!(
            !result.is_empty(),
            "fake_avatar should return non-empty base64"
        );

        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &result)
            .expect("Should be valid base64");
        assert_eq!(
            &bytes[0..8],
            &[137, 80, 78, 71, 13, 10, 26, 10],
            "Should be a valid PNG"
        );
    }

    #[test]
    fn test_fake_placeholder() {
        let result = fake_placeholder(Some(300), Some(200), None, None, None);
        assert!(
            !result.is_empty(),
            "fake_placeholder should return non-empty base64"
        );

        let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &result)
            .expect("Should be valid base64");
        assert_eq!(
            &bytes[0..8],
            &[137, 80, 78, 71, 13, 10, 26, 10],
            "Should be a valid PNG"
        );
    }
}
