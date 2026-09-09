//! File generation (PDF, PNG, JPEG)

use super::rng::rng;
use image::Rgba;
use rand::RngExt;
use rand::seq::IndexedRandom;
use rust_embed::Embed;

/// Embedded font assets
#[derive(Embed)]
#[folder = "assets/"]
struct EmbeddedAssets;

/// The PDF document model lives in [`super::document`]; these aliases keep the
/// paths callers already import.
pub use super::document::{Extras as PdfExtras, PdfPreset, PdfSpec, fake_pdf_document};

/// Generate a fake PDF document with optional text content and page count
/// Returns base64-encoded PDF data
///
/// Long lines wrap to the text column rather than running off the page.
#[must_use]
pub fn fake_pdf(text: Option<&str>, pages: Option<u32>) -> String {
    fake_pdf_document(&PdfSpec {
        pages: pages.unwrap_or(1).max(1),
        text: text.map(ToString::to_string),
        ..PdfSpec::default()
    })
    .base64
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
    let mut rng = rng();

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
    let mut rng = rng();

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
    let mut rng = rng();
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

// ---------------------------------------------------------------------------
// Structured imagery
//
// The generators above draw flat patterns. These draw the things a fixture
// usually stands in for: a photograph, a scanned page, a chart, a screenshot.
// All of them are deterministic under a seed, and none of them reaches the
// network or the disk.
// ---------------------------------------------------------------------------

/// Encode an RGBA buffer as base64 PNG, the shape every generator here returns.
fn encode_png(img: &image::RgbaImage) -> String {
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

fn lerp(from: f64, to: f64, at: f64) -> f64 {
    (to - from).mul_add(at.clamp(0.0, 1.0), from)
}

fn channel(value: f64) -> u8 {
    value.clamp(0.0, 255.0) as u8
}

fn mix(from: [u8; 3], to: [u8; 3], at: f64) -> Rgba<u8> {
    Rgba([
        channel(lerp(f64::from(from[0]), f64::from(to[0]), at)),
        channel(lerp(f64::from(from[1]), f64::from(to[1]), at)),
        channel(lerp(f64::from(from[2]), f64::from(to[2]), at)),
        255,
    ])
}

/// A smooth multi-frequency colour field: the closest thing to a photograph
/// that no asset on disk can produce.
///
/// `octaves` sets how much fine detail rides on the broad shapes. Each channel
/// gets its own phase, which is what keeps the result coloured rather than a
/// grey wash: summing octaves into one value and ramping it averages to mud.
#[must_use]
pub fn fake_image_plasma(
    width: Option<u32>,
    height: Option<u32>,
    octaves: Option<u32>,
    palette: Option<&str>,
) -> String {
    let width = width.unwrap_or(640).max(1);
    let height = height.unwrap_or(480).max(1);
    let octaves = octaves.unwrap_or(4).clamp(1, 8);
    let tint = palette.map(|hex| parse_color_or_random(Some(hex)));
    let mut rng = rng();

    // Three independent phase sets, one per channel, plus a per-channel
    // rotation of the (u, v) plane so the channels do not line up into grey.
    let channels: Vec<Vec<(f64, f64, f64, f64)>> = (0..3)
        .map(|_| {
            (0..octaves)
                .map(|_| {
                    (
                        rng.random_range(0.0..std::f64::consts::TAU),
                        rng.random_range(0.0..std::f64::consts::TAU),
                        rng.random_range(0.0..std::f64::consts::TAU),
                        rng.random_range(0.0..std::f64::consts::TAU),
                    )
                })
                .collect()
        })
        .collect();

    let sample = |phases: &[(f64, f64, f64, f64)], u: f64, v: f64| -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut total = 0.0;
        for (octave, (px, py, pd, rotation)) in phases.iter().enumerate() {
            let frequency = f64::from(1_u32 << octave as u32) * std::f64::consts::PI;
            let (ru, rv) = (
                u.mul_add(rotation.cos(), -(v * rotation.sin())),
                u.mul_add(rotation.sin(), v * rotation.cos()),
            );
            value = f64::mul_add(
                amplitude,
                ru.mul_add(frequency, *px).sin()
                    + rv.mul_add(frequency, *py).sin()
                    + (ru + rv).mul_add(frequency * 0.7, *pd).sin(),
                value,
            );
            total = f64::mul_add(amplitude, 3.0, total);
            amplitude *= 0.55;
        }
        f64::midpoint(value / total, 1.0)
    };

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let u = f64::from(x) / f64::from(width);
        let v = f64::from(y) / f64::from(height);
        let channel_at = |index: usize| {
            channels
                .get(index)
                .map_or(0.5, |phases| sample(phases, u, v))
        };
        // Widen around the midpoint: the sum of sines clusters near 0.5, which
        // is what made the first version a flat wash.
        let stretch = |value: f64| (value - 0.5).mul_add(1.9, 0.5).clamp(0.0, 1.0);
        let mut rgb = [
            stretch(channel_at(0)),
            stretch(channel_at(1)),
            stretch(channel_at(2)),
        ];
        if let Some(tint) = tint {
            for (channel, level) in rgb.iter_mut().enumerate() {
                let towards = f64::from(tint.get(channel).copied().unwrap_or(128)) / 255.0;
                *level = level.mul_add(0.55, towards * 0.45);
            }
        }
        Rgba([
            channel(rgb.first().copied().unwrap_or(0.5) * 255.0),
            channel(rgb.get(1).copied().unwrap_or(0.5) * 255.0),
            channel(rgb.get(2).copied().unwrap_or(0.5) * 255.0),
            255,
        ])
    });

    encode_png(&img)
}

/// A four-corner gradient mesh, bilinearly interpolated.
#[must_use]
pub fn fake_image_mesh(width: Option<u32>, height: Option<u32>) -> String {
    let width = width.unwrap_or(640).max(1);
    let height = height.unwrap_or(480).max(1);
    let corners = [
        random_color_rgb(),
        random_color_rgb(),
        random_color_rgb(),
        random_color_rgb(),
    ];

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let u = f64::from(x) / f64::from(width.saturating_sub(1).max(1));
        let v = f64::from(y) / f64::from(height.saturating_sub(1).max(1));
        let corner = |index: usize| corners.get(index).copied().unwrap_or([128, 128, 128]);
        let top = mix(corner(0), corner(1), u);
        let bottom = mix(corner(2), corner(3), u);
        mix(
            [top[0], top[1], top[2]],
            [bottom[0], bottom[1], bottom[2]],
            v,
        )
    });

    encode_png(&img)
}

/// A landscape photograph: graded sky, sun, layered hills and a haze band.
#[must_use]
pub fn fake_image_photo(width: Option<u32>, height: Option<u32>) -> String {
    let width = width.unwrap_or(800).max(8);
    let height = height.unwrap_or(500).max(8);
    let mut rng = rng();

    let horizon = f64::from(height) * rng.random_range(0.48..0.68);
    let sun = (
        f64::from(width) * rng.random_range(0.15..0.85),
        horizon * rng.random_range(0.25..0.7),
    );
    let sun_radius = f64::from(width) * 0.035;

    // Each ridge is a sum of two sines, so the skyline is not a clean curve.
    let ridges: Vec<(f64, f64, f64, f64, [u8; 3])> = (0..4)
        .map(|layer| {
            let depth = f64::from(layer) / 4.0;
            let shade = channel(lerp(40.0, 120.0, 1.0 - depth));
            (
                f64::mul_add(
                    f64::from(height),
                    0.09f64.mul_add(f64::from(layer), 0.02),
                    horizon,
                ),
                f64::from(height) * rng.random_range(0.02..0.06),
                rng.random_range(1.5..4.5),
                rng.random_range(0.0..std::f64::consts::TAU),
                [
                    channel(f64::from(shade) * 0.8),
                    shade,
                    channel(f64::from(shade) * 0.7),
                ],
            )
        })
        .collect();

    let sky_top = [
        rng.random_range(20..70u8),
        rng.random_range(70..130u8),
        rng.random_range(150..210u8),
    ];
    let sky_bottom = [
        rng.random_range(200..250u8),
        rng.random_range(180..225u8),
        rng.random_range(150..200u8),
    ];

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);

        for (base, amplitude, frequency, phase, colour) in &ridges {
            let ridge = f64::mul_add(
                amplitude * 0.4,
                -(fx / f64::from(width) * frequency * 3.1 + phase).cos(),
                amplitude.mul_add(
                    (fx / f64::from(width) * frequency * std::f64::consts::TAU + phase).sin(),
                    *base,
                ),
            );
            if fy >= ridge {
                // Nearer ground is darker, so the layers read as depth.
                let fall = ((fy - ridge) / f64::from(height)).min(1.0);
                let shade = [
                    channel(f64::from(colour[0]) * f64::mul_add(fall, -0.5, 1.0)),
                    channel(f64::from(colour[1]) * f64::mul_add(fall, -0.5, 1.0)),
                    channel(f64::from(colour[2]) * f64::mul_add(fall, -0.5, 1.0)),
                ];
                return Rgba([shade[0], shade[1], shade[2], 255]);
            }
        }

        let to_sun = (fx - sun.0).hypot(fy - sun.1);
        if to_sun <= sun_radius {
            return Rgba([255, 246, 214, 255]);
        }

        let mut pixel = mix(sky_top, sky_bottom, fy / horizon.max(1.0));
        // Glow around the sun, falling off over four radii.
        let glow = (1.0 - (to_sun / (sun_radius * 4.0)).min(1.0)).powi(2);
        if glow > 0.0 {
            pixel = Rgba([
                channel(lerp(f64::from(pixel[0]), 255.0, glow * 0.7)),
                channel(lerp(f64::from(pixel[1]), 244.0, glow * 0.7)),
                channel(lerp(f64::from(pixel[2]), 200.0, glow * 0.7)),
                255,
            ]);
        }
        pixel
    });

    encode_png(&img)
}

/// A page that has been through a scanner: off-white stock, ruled text lines
/// that look like prose at a distance, speckle and a corner shadow.
#[must_use]
pub fn fake_image_scan(width: Option<u32>, height: Option<u32>, lines: Option<u32>) -> String {
    let width = width.unwrap_or(620).max(40);
    let height = height.unwrap_or(877).max(40);
    let mut rng = rng();
    let line_count = lines.unwrap_or_else(|| rng.random_range(28..44)).max(1);

    let margin = f64::from(width) * 0.11;
    let column = f64::mul_add(margin, -2.0, f64::from(width));
    let top = f64::from(height) * 0.09;
    let spacing = (f64::from(height) * 0.82) / f64::from(line_count);
    let stroke = (spacing * 0.28).max(1.0);

    // A line is a run of word-length segments, so the page reads as text.
    let rows: Vec<Vec<(f64, f64)>> = (0..line_count)
        .map(|_| {
            let mut words = Vec::new();
            let mut cursor = 0.0;
            let fill = rng.random_range(0.55..1.0) * column;
            // The narrowest word plus its gap is 5.2% of the column, so 32
            // covers any line; the bound is what stops a run of narrow draws
            // from filling the vector instead of the line.
            for _ in 0..32 {
                if cursor >= fill {
                    break;
                }
                let word = rng.random_range(column * 0.03..column * 0.16);
                words.push((cursor, word.min(fill - cursor).max(0.0)));
                cursor += f64::mul_add(column, 0.022, word);
            }
            words
        })
        .collect();

    let speckle: Vec<(u32, u32, u8)> = (0..(width * height / 900))
        .map(|_| {
            (
                rng.random_range(0..width),
                rng.random_range(0..height),
                rng.random_range(150..205u8),
            )
        })
        .collect();
    let speckles: std::collections::HashMap<(u32, u32), u8> =
        speckle.into_iter().map(|(x, y, v)| ((x, y), v)).collect();

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);

        for (index, words) in rows.iter().enumerate() {
            let baseline = (f64::from(index as u32)).mul_add(spacing, top);
            if fy < baseline || fy > baseline + stroke {
                continue;
            }
            for (start, length) in words {
                let from = margin + start;
                if fx >= from && fx <= from + length {
                    return Rgba([46, 46, 52, 255]);
                }
            }
        }

        if let Some(value) = speckles.get(&(x, y)) {
            return Rgba([*value, *value, channel(f64::from(*value) * 0.97), 255]);
        }

        // Paper: warm white, darkening towards the edges the way a platen does.
        let edge = (fx / f64::from(width) - 0.5)
            .abs()
            .max((fy / f64::from(height) - 0.5).abs());
        let vignette = f64::mul_add(((edge - 0.34).max(0.0) / 0.16).min(1.0), -0.14, 1.0);
        Rgba([
            channel(250.0 * vignette),
            channel(248.0 * vignette),
            channel(240.0 * vignette),
            255,
        ])
    });

    encode_png(&img)
}

/// A bar or line chart as a raster image, for a fixture that wants a picture of
/// a chart rather than the vector one a PDF carries.
#[must_use]
pub fn fake_image_chart(
    width: Option<u32>,
    height: Option<u32>,
    kind: Option<&str>,
    points: Option<u32>,
    color: Option<&str>,
) -> String {
    let width = width.unwrap_or(720).max(40);
    let height = height.unwrap_or(360).max(40);
    let kind = kind.unwrap_or("bar").to_ascii_lowercase();
    let mut rng = rng();
    let count = points
        .unwrap_or_else(|| rng.random_range(5..12))
        .clamp(1, 64);
    let accent = parse_color_or_random(color);

    let values: Vec<f64> = (0..count).map(|_| rng.random_range(0.12..1.0)).collect();
    let pad = f64::from(width) * 0.06;
    let plot_bottom = f64::from(height) - pad;
    let plot_height = plot_bottom - pad;
    let plot_width = f64::mul_add(pad, -2.0, f64::from(width));

    let value_at = |index: usize| values.get(index).copied().unwrap_or(0.0);
    let slot = plot_width / f64::from(count);

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);

        // Gridlines every fifth of the plot.
        let on_grid = (0..=5).any(|step| {
            let line = plot_bottom - f64::from(step) * plot_height / 5.0;
            (fy - line).abs() < 0.6 && fx >= pad && fx <= pad + plot_width
        });

        let inside = fx >= pad && fx <= pad + plot_width && fy >= pad && fy <= plot_bottom;
        if inside {
            match kind.as_str() {
                "line" | "area" => {
                    let at = ((fx - pad) / plot_width * f64::from(count.saturating_sub(1).max(1)))
                        .clamp(0.0, f64::from(count.saturating_sub(1)));
                    let left = at.floor() as usize;
                    let value = lerp(value_at(left), value_at(left + 1), at.fract());
                    let line_y = value.mul_add(-plot_height, plot_bottom);
                    if (fy - line_y).abs() <= 1.6 {
                        return Rgba([accent[0], accent[1], accent[2], 255]);
                    }
                    if kind == "area" && fy > line_y {
                        return mix([255, 255, 255], accent, 0.28);
                    }
                }
                _ => {
                    let index = ((fx - pad) / slot).floor().max(0.0) as usize;
                    let bar_left =
                        f64::mul_add(slot, 0.18, f64::mul_add(f64::from(index as u32), slot, pad));
                    let bar_right = f64::mul_add(slot, 0.64, bar_left);
                    let bar_top = f64::mul_add(value_at(index), -plot_height, plot_bottom);
                    if fx >= bar_left && fx <= bar_right && fy >= bar_top {
                        return Rgba([accent[0], accent[1], accent[2], 255]);
                    }
                }
            }
        }

        if (fy - plot_bottom).abs() < 1.2 && fx >= pad && fx <= pad + plot_width {
            return Rgba([90, 90, 96, 255]);
        }
        if on_grid {
            return Rgba([228, 228, 232, 255]);
        }
        Rgba([255, 255, 255, 255])
    });

    encode_png(&img)
}

/// A square matrix of modules with the three finder squares a QR code carries.
///
/// It is not a decodable QR code and encodes nothing; it is what a fixture
/// needs when the test only looks at it.
#[must_use]
pub fn fake_image_qr(size: Option<u32>, modules: Option<u32>) -> String {
    let size = size.unwrap_or(320).max(21);
    let modules = modules.unwrap_or(25).clamp(21, 81);
    let mut rng = rng();

    let cells: Vec<bool> = (0..modules * modules)
        .map(|_| rng.random_range(0..2u8) == 0)
        .collect();
    let scale = f64::from(size) / f64::from(modules);

    let finder = |mx: u32, my: u32| -> Option<bool> {
        for (ox, oy) in [(0, 0), (modules - 7, 0), (0, modules - 7)] {
            if mx >= ox && mx < ox + 7 && my >= oy && my < oy + 7 {
                let rx = mx - ox;
                let ry = my - oy;
                let ring = rx == 0 || rx == 6 || ry == 0 || ry == 6;
                let core = (2..=4).contains(&rx) && (2..=4).contains(&ry);
                return Some(ring || core);
            }
        }
        None
    };

    let img = image::RgbaImage::from_fn(size, size, |x, y| {
        let mx = ((f64::from(x) / scale).floor() as u32).min(modules - 1);
        let my = ((f64::from(y) / scale).floor() as u32).min(modules - 1);
        let dark = finder(mx, my).unwrap_or_else(|| {
            cells
                .get((my * modules + mx) as usize)
                .copied()
                .unwrap_or(false)
        });
        if dark {
            Rgba([17, 17, 17, 255])
        } else {
            Rgba([255, 255, 255, 255])
        }
    });

    encode_png(&img)
}

/// Variable-width bars with a quiet zone and a human-readable number, the shape
/// a Code 128 label has.
#[must_use]
pub fn fake_image_barcode(width: Option<u32>, height: Option<u32>, digits: Option<u32>) -> String {
    let width = width.unwrap_or(480).max(40);
    let height = height.unwrap_or(160).max(24);
    let mut rng = rng();
    let digit_count = digits.unwrap_or(12).clamp(4, 24);

    let number: String = (0..digit_count)
        .map(|_| char::from_digit(rng.random_range(0..10u32), 10).unwrap_or('0'))
        .collect();

    let quiet = f64::from(width) * 0.06;
    let bar_area = f64::mul_add(quiet, -2.0, f64::from(width));
    let bars: Vec<(f64, f64)> = {
        let mut placed = Vec::new();
        let mut cursor = 0.0;
        let unit = bar_area / f64::from(digit_count * 11);
        // A bar and its gap are at least two units, so the symbol budget is
        // half the units available.
        for _ in 0..(digit_count * 11 / 2) {
            if cursor >= bar_area {
                break;
            }
            let bar = unit * f64::from(rng.random_range(1..4u32));
            let gap = unit * f64::from(rng.random_range(1..4u32));
            placed.push((cursor, bar.min(bar_area - cursor).max(0.0)));
            cursor += bar + gap;
        }
        placed
    };

    let bar_bottom = f64::from(height) * 0.76;
    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);
        if fy <= bar_bottom && fx >= quiet && fx <= quiet + bar_area {
            let at = fx - quiet;
            if bars
                .iter()
                .any(|(start, run)| at >= *start && at <= start + run)
            {
                return Rgba([12, 12, 12, 255]);
            }
        }
        Rgba([255, 255, 255, 255])
    });

    let mut img = img;
    let label_size = (f64::from(height) * 0.16) as f32;
    draw_centred_text(
        &mut img,
        &number,
        f64::mul_add(f64::from(height), 0.03, bar_bottom),
        label_size,
    );
    encode_png(&img)
}

/// A symmetric block identicon derived from `seed`, the avatar a service shows
/// before anyone uploads a picture.
#[must_use]
pub fn fake_image_identicon(seed: Option<&str>, size: Option<u32>, cells: Option<u32>) -> String {
    let size = size.unwrap_or(256).max(8);
    let cells = cells.unwrap_or(5).clamp(3, 12);
    let half = cells.div_ceil(2);

    // Hash the seed rather than drawing, so the same name always gives the same
    // identicon whether or not a seed is installed.
    let hash = seed.map_or_else(
        || {
            let mut rng = rng();
            rng.random_range(0..u64::MAX)
        },
        |value| {
            let mut hash = 0xcbf2_9ce4_8422_2325_u64;
            for byte in value.as_bytes() {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash
        },
    );

    let foreground = [
        channel(f64::mul_add(f64::from((hash >> 16) as u8), 0.6, 40.0)),
        channel(f64::mul_add(f64::from((hash >> 24) as u8), 0.6, 40.0)),
        channel(f64::mul_add(f64::from((hash >> 32) as u8), 0.6, 40.0)),
    ];
    let scale = f64::from(size) / f64::from(cells);

    let img = image::RgbaImage::from_fn(size, size, |x, y| {
        let cx = ((f64::from(x) / scale).floor() as u32).min(cells - 1);
        let cy = ((f64::from(y) / scale).floor() as u32).min(cells - 1);
        // Mirror the left half onto the right, which is what makes it read as
        // an identicon rather than noise.
        let mirrored = if cx >= half { cells - 1 - cx } else { cx };
        let bit = (mirrored * cells + cy) % 64;
        if (hash >> bit) & 1 == 1 {
            Rgba([foreground[0], foreground[1], foreground[2], 255])
        } else {
            Rgba([244, 244, 246, 255])
        }
    });

    encode_png(&img)
}

/// A window: title bar with traffic lights, a sidebar and content cards. What a
/// fixture standing in for a product screenshot needs to look like.
#[must_use]
pub fn fake_image_screenshot(
    width: Option<u32>,
    height: Option<u32>,
    dark: Option<bool>,
) -> String {
    let width = width.unwrap_or(1000).max(120);
    let height = height.unwrap_or(640).max(120);
    let dark = dark.unwrap_or(false);
    let mut rng = rng();
    let accent = random_color_rgb();

    let chrome = f64::from(height) * 0.055;
    let sidebar = f64::from(width) * 0.19;
    let (surface, panel, card, text) = if dark {
        ([24, 25, 30], [32, 34, 41], [41, 44, 53], [176, 180, 192])
    } else {
        (
            [246, 247, 250],
            [255, 255, 255],
            [239, 241, 246],
            [140, 146, 160],
        )
    };

    // Rows of list items in the sidebar and cards in the body.
    let items: Vec<f64> = (0..9).map(|_| rng.random_range(0.35..0.92)).collect();
    let cards: Vec<(f64, f64)> = (0..6)
        .map(|_| (rng.random_range(0.3..0.95), rng.random_range(0.4..1.0)))
        .collect();

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);

        if fy < chrome {
            for (index, light) in [[237, 106, 94], [244, 191, 79], [97, 197, 84]]
                .iter()
                .enumerate()
            {
                let cx = chrome * 0.7f64.mul_add(f64::from(index as u32), 0.55);
                if (fx - cx).hypot(fy - chrome / 2.0) <= chrome * 0.19 {
                    return Rgba([light[0], light[1], light[2], 255]);
                }
            }
            return Rgba([
                channel(f64::from(panel[0]) * 0.94),
                channel(f64::from(panel[1]) * 0.94),
                channel(f64::from(panel[2]) * 0.94),
                255,
            ]);
        }

        if fx < sidebar {
            let row_height = (f64::from(height) - chrome) / 12.0;
            let row = ((fy - chrome) / row_height).floor() as usize;
            let inset = sidebar * 0.12;
            if let Some(fill) = items.get(row) {
                let top = f64::mul_add(
                    row_height,
                    0.3,
                    f64::mul_add(f64::from(row as u32), row_height, chrome),
                );
                if fy >= top
                    && fy <= f64::mul_add(row_height, 0.34, top)
                    && fx >= inset
                    && fx <= inset + f64::mul_add(inset, -2.0, sidebar) * fill
                {
                    // The first row is the selection, in the accent.
                    let colour = if row == 1 { accent } else { text };
                    return Rgba([colour[0], colour[1], colour[2], 255]);
                }
            }
            return Rgba([panel[0], panel[1], panel[2], 255]);
        }

        let body_left = f64::mul_add(f64::from(width), 0.03, sidebar);
        let body_width = f64::mul_add(f64::from(width), -0.03, f64::from(width) - body_left);
        let card_height = (f64::from(height) - chrome) / 3.6;
        let column_width = body_width / 2.0;
        let column = ((fx - body_left) / column_width).floor().max(0.0) as usize;
        let row = (f64::mul_add(card_height, -0.2, fy - chrome) / card_height)
            .floor()
            .max(0.0) as usize;
        let index = row * 2 + column;

        if let Some((title_fill, body_fill)) = cards.get(index) {
            let left = f64::mul_add(
                column_width,
                0.04,
                f64::mul_add(f64::from(column as u32), column_width, body_left),
            );
            let right = f64::mul_add(
                column_width,
                -0.04,
                f64::mul_add(f64::from(column as u32 + 1), column_width, body_left),
            );
            let top = f64::mul_add(card_height, 0.2 + f64::from(row as u32), chrome);
            let bottom = f64::mul_add(card_height, 0.74, top);
            if fx >= left && fx <= right && fy >= top && fy <= bottom {
                let title_top = f64::mul_add(card_height, 0.12, top);
                if fy >= title_top && fy <= f64::mul_add(card_height, 0.1, title_top) {
                    let run = (right - left) * 0.82 * title_fill;
                    if fx <= f64::mul_add(right - left, 0.09, left) + run {
                        return Rgba([accent[0], accent[1], accent[2], 255]);
                    }
                }
                let line_top = f64::mul_add(card_height, 0.34, top);
                for line in 0..3 {
                    let ly = f64::mul_add(f64::from(line) * card_height, 0.13, line_top);
                    if fy >= ly && fy <= f64::mul_add(card_height, 0.06, ly) {
                        let run = (right - left)
                            * 0.8
                            * body_fill
                            * 0.16f64.mul_add(-f64::from(line), 1.0);
                        if fx <= f64::mul_add(right - left, 0.09, left) + run {
                            return Rgba([text[0], text[1], text[2], 255]);
                        }
                    }
                }
                return Rgba([card[0], card[1], card[2], 255]);
            }
        }

        Rgba([surface[0], surface[1], surface[2], 255])
    });

    encode_png(&img)
}

/// A cell grid over a blue-to-red ramp, the shape a matrix heatmap has.
#[must_use]
pub fn fake_image_heatmap(
    width: Option<u32>,
    height: Option<u32>,
    columns: Option<u32>,
    rows: Option<u32>,
) -> String {
    let width = width.unwrap_or(640).max(16);
    let height = height.unwrap_or(400).max(16);
    let columns = columns.unwrap_or(16).clamp(1, 128);
    let rows = rows.unwrap_or(10).clamp(1, 128);
    let mut rng = rng();

    let values: Vec<f64> = (0..columns * rows)
        .map(|_| rng.random_range(0.0..1.0f64))
        .collect();
    let cell_w = f64::from(width) / f64::from(columns);
    let cell_h = f64::from(height) / f64::from(rows);

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let cx = ((f64::from(x) / cell_w).floor() as u32).min(columns - 1);
        let cy = ((f64::from(y) / cell_h).floor() as u32).min(rows - 1);
        // A one-pixel gutter, so the cells read as a grid.
        if f64::from(x) % cell_w < 1.0 || f64::from(y) % cell_h < 1.0 {
            return Rgba([255, 255, 255, 255]);
        }
        let value = values
            .get((cy * columns + cx) as usize)
            .copied()
            .unwrap_or(0.0);
        if value < 0.5 {
            mix([32, 68, 148], [242, 240, 226], value * 2.0)
        } else {
            mix([242, 240, 226], [176, 34, 42], (value - 0.5) * 2.0)
        }
    });

    encode_png(&img)
}

/// A centred audio waveform: a mirrored envelope of per-column peaks.
#[must_use]
pub fn fake_image_waveform(width: Option<u32>, height: Option<u32>, color: Option<&str>) -> String {
    let width = width.unwrap_or(800).max(8);
    let height = height.unwrap_or(200).max(8);
    let accent = parse_color_or_random(color);
    let mut rng = rng();

    // A slow envelope times fast per-column noise, which is what a waveform of
    // speech or music looks like at this zoom.
    let envelope: Vec<f64> = (0..width)
        .map(|x| {
            let at = f64::from(x) / f64::from(width);
            let slow = (at * std::f64::consts::TAU * 2.3)
                .sin()
                .abs()
                .mul_add(0.6, 0.25);
            slow * rng.random_range(0.35..1.0)
        })
        .collect();

    let centre = f64::from(height) / 2.0;
    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let peak = envelope.get(x as usize).copied().unwrap_or(0.0) * centre * 0.94;
        let offset = (f64::from(y) - centre).abs();
        if offset <= peak {
            Rgba([accent[0], accent[1], accent[2], 255])
        } else if offset < 1.0 {
            mix([255, 255, 255], accent, 0.4)
        } else {
            Rgba([250, 250, 252, 255])
        }
    });

    encode_png(&img)
}

/// An abstract street map: blocks, a road grid and a river.
#[must_use]
pub fn fake_image_map(width: Option<u32>, height: Option<u32>) -> String {
    let width = width.unwrap_or(640).max(32);
    let height = height.unwrap_or(640).max(32);
    let mut rng = rng();

    let verticals: Vec<(f64, f64)> = (0..rng.random_range(5..10u32))
        .map(|_| {
            (
                f64::from(width) * rng.random_range(0.05..0.95),
                rng.random_range(2.0..7.0),
            )
        })
        .collect();
    let horizontals: Vec<(f64, f64)> = (0..rng.random_range(5..10u32))
        .map(|_| {
            (
                f64::from(height) * rng.random_range(0.05..0.95),
                rng.random_range(2.0..7.0),
            )
        })
        .collect();
    let river = (
        f64::from(width) * rng.random_range(0.25..0.75),
        f64::from(width) * rng.random_range(0.03..0.06),
        rng.random_range(0.0..std::f64::consts::TAU),
    );

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);

        let bank = river.1.mul_add(
            (fy / f64::from(height) * std::f64::consts::TAU)
                .mul_add(1.4, river.2)
                .sin()
                * 2.0,
            river.0,
        );
        if (fx - bank).abs() <= river.1 {
            return Rgba([146, 190, 214, 255]);
        }

        if verticals
            .iter()
            .any(|(at, wide)| (fx - at).abs() <= wide / 2.0)
            || horizontals
                .iter()
                .any(|(at, wide)| (fy - at).abs() <= wide / 2.0)
        {
            return Rgba([252, 252, 250, 255]);
        }

        // Parks on a coarse checker, so the blocks are not all the same green.
        let block_x = (fx / (f64::from(width) / 9.0)).floor() as u32;
        let block_y = (fy / (f64::from(height) / 9.0)).floor() as u32;
        if (block_x + block_y).is_multiple_of(7) {
            return Rgba([203, 224, 194, 255]);
        }
        Rgba([233, 231, 226, 255])
    });

    encode_png(&img)
}

/// A technical drawing: cyan ground, a fine grid, and a dimensioned rectangle.
#[must_use]
pub fn fake_image_blueprint(width: Option<u32>, height: Option<u32>) -> String {
    let width = width.unwrap_or(720).max(40);
    let height = height.unwrap_or(520).max(40);
    let minor = f64::from(width) / 40.0;
    let mut rng = rng();

    let shape = (
        f64::from(width) * rng.random_range(0.14..0.26),
        f64::from(height) * rng.random_range(0.16..0.28),
        f64::from(width) * rng.random_range(0.42..0.62),
        f64::from(height) * rng.random_range(0.36..0.55),
    );

    let img = image::RgbaImage::from_fn(width, height, |x, y| {
        let fx = f64::from(x);
        let fy = f64::from(y);
        let ground = Rgba([22, 62, 116, 255]);

        let on_edge =
            |from: f64, to: f64, at: f64| (at - from).abs() < 1.4 || (at - to).abs() < 1.4;
        let inside_x = fx >= shape.0 && fx <= shape.0 + shape.2;
        let inside_y = fy >= shape.1 && fy <= shape.1 + shape.3;
        if (inside_x && inside_y)
            && (on_edge(shape.0, shape.0 + shape.2, fx) || on_edge(shape.1, shape.1 + shape.3, fy))
        {
            return Rgba([245, 250, 255, 255]);
        }
        // Dimension lines, offset outside the shape.
        if inside_x && ((fy - f64::mul_add(minor, -1.4, shape.1)).abs() < 0.8) {
            return Rgba([190, 214, 240, 255]);
        }
        if inside_y && ((fx - f64::mul_add(minor, -1.4, shape.0)).abs() < 0.8) {
            return Rgba([190, 214, 240, 255]);
        }

        let major = minor * 5.0;
        if fx % major < 0.9 || fy % major < 0.9 {
            return Rgba([58, 104, 166, 255]);
        }
        if fx % minor < 0.7 || fy % minor < 0.7 {
            return Rgba([38, 80, 138, 255]);
        }
        ground
    });

    encode_png(&img)
}

/// Draw `text` centred horizontally with its top at `top`, in the embedded font.
fn draw_centred_text(img: &mut image::RgbaImage, text: &str, top: f64, size: f32) {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont as _};

    let Some(asset) = EmbeddedAssets::get("NotoSans-Regular.ttf") else {
        return;
    };
    let Ok(font) = FontRef::try_from_slice(asset.data.as_ref()) else {
        return;
    };
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let width: f32 = text
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let x = (img.width() as f32 - width) / 2.0;
    imageproc::drawing::draw_text_mut(
        img,
        Rgba([20, 20, 20, 255]),
        x as i32,
        top as i32,
        scale,
        &font,
        text,
    );
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
