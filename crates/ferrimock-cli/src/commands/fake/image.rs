//! Fake image generation

use crate::commands::ui;
use crate::ops::fake::Image;
use base64::Engine;

/// Every `fake image TYPE`, for the error message and the listing.
pub const IMAGE_TYPES: [(&str, &str); 21] = [
    ("placeholder", "size label on a flat panel"),
    ("avatar", "initials on a coloured disc"),
    ("gradient", "two-stop linear gradient"),
    ("mesh", "four-corner gradient mesh"),
    ("checkerboard", "alternating squares"),
    ("noise", "per-pixel noise, grey or coloured"),
    ("stripes", "banded stripes"),
    ("text", "centred text on a panel"),
    ("solid", "one flat colour"),
    ("plasma", "smooth multi-frequency colour field"),
    ("photo", "landscape: sky, sun and layered hills"),
    ("scan", "a scanned page: ruled text, speckle, vignette"),
    ("chart", "bar, line or area chart"),
    ("qr", "QR-shaped module matrix with finders"),
    ("barcode", "Code 128-shaped bars and a number"),
    ("identicon", "symmetric block avatar from a seed"),
    ("screenshot", "window chrome, sidebar and cards"),
    ("heatmap", "cell grid over a blue-to-red ramp"),
    ("waveform", "mirrored audio envelope"),
    ("map", "abstract streets, blocks and a river"),
    ("blueprint", "technical drawing on grid paper"),
];

/// Where the nth image of a batch goes. `%n` in the path is the index.
fn output_path(template: &str, index: usize, count: usize) -> String {
    let width = count.to_string().len();
    if template.contains("%n") {
        return template.replace("%n", &format!("{:0width$}", index + 1));
    }
    if count == 1 {
        return template.to_string();
    }
    match template.rsplit_once('.') {
        Some((stem, extension)) => format!("{stem}-{:0width$}.{extension}", index + 1),
        None => format!("{template}-{:0width$}", index + 1),
    }
}

/// Render one image of the requested type as base64 PNG.
pub fn render(opts: &Image) -> anyhow::Result<String> {
    use ferrimock::fake_data::*;

    let width = opts.width;
    let height = opts.height;
    let bg_color = opts.bg_color.as_deref();
    let text_color = opts.text_color.as_deref();
    let text = opts.text.as_deref();
    let direction = opts.direction.as_str();
    let colored = opts.colored;

    let base64_data = match opts.image_type.to_lowercase().as_str() {
        "placeholder" => {
            let display_text = text.map_or_else(|| format!("{width}x{height}"), String::from);
            fake_placeholder(
                Some(width),
                Some(height),
                Some(&display_text),
                bg_color,
                text_color,
            )
        }
        "avatar" => {
            let init = opts.initials.as_deref().unwrap_or("??");
            fake_avatar(Some(init), Some(width), bg_color, text_color)
        }
        "gradient" => {
            let start = opts.start.as_deref().unwrap_or("#FF0000");
            let end = opts.end.as_deref().unwrap_or("#0000FF");
            fake_image_gradient(
                Some(width),
                Some(height),
                Some(start),
                Some(end),
                Some(direction),
            )
        }
        "checkerboard" | "checker" => {
            let c1 = bg_color.unwrap_or("#FFFFFF");
            let c2 = text_color.unwrap_or("#000000");
            fake_image_checkerboard(Some(width), Some(height), Some(c1), Some(c2), Some(20))
        }
        "noise" => fake_image_noise(Some(width), Some(height), Some(colored)),
        "stripes" => {
            let c1 = bg_color.unwrap_or("#FFFFFF");
            let c2 = text_color.unwrap_or("#000000");
            fake_image_stripes(
                Some(width),
                Some(height),
                Some(c1),
                Some(c2),
                Some(20),
                Some(direction),
            )
        }
        "text" => {
            let display_text = text.unwrap_or("Sample Text");
            fake_image_with_text(
                Some(display_text),
                Some(width),
                Some(height),
                bg_color,
                text_color,
                Some(24.0),
            )
        }
        "solid" | "color" => fake_png(Some(width), Some(height), bg_color),
        "mesh" => fake_image_mesh(Some(width), Some(height)),
        "plasma" => fake_image_plasma(Some(width), Some(height), opts.octaves, bg_color),
        "photo" => fake_image_photo(Some(width), Some(height)),
        "scan" => fake_image_scan(Some(width), Some(height), opts.cells),
        "chart" => fake_image_chart(
            Some(width),
            Some(height),
            opts.kind.as_deref(),
            opts.points,
            bg_color,
        ),
        "qr" => fake_image_qr(Some(width.min(height)), opts.cells),
        "barcode" => fake_image_barcode(Some(width), Some(height), opts.points),
        "identicon" => {
            fake_image_identicon(opts.seed.as_deref(), Some(width.min(height)), opts.cells)
        }
        "screenshot" => fake_image_screenshot(Some(width), Some(height), Some(opts.dark)),
        "heatmap" => fake_image_heatmap(Some(width), Some(height), opts.cells, opts.rows),
        "waveform" => fake_image_waveform(Some(width), Some(height), bg_color),
        "map" => fake_image_map(Some(width), Some(height)),
        "blueprint" => fake_image_blueprint(Some(width), Some(height)),
        other => {
            let names: Vec<&str> = IMAGE_TYPES.iter().map(|(name, _)| *name).collect();
            anyhow::bail!(
                "Unknown image type {other:?}. Available: {}",
                names.join(", ")
            );
        }
    };

    Ok(base64_data)
}

/// Generate a fake image, or a batch of them.
pub fn generate_fake_image(opts: &Image) -> anyhow::Result<()> {
    let count = opts.count.max(1);
    if count > 1 && opts.output.is_none() && !opts.base64 && !opts.data_uri {
        anyhow::bail!("--count {count} needs --output, or every image goes to its own temp file");
    }
    for index in 0..count {
        write_one(opts, index, count)?;
    }
    Ok(())
}

fn write_one(opts: &Image, index: usize, count: usize) -> anyhow::Result<()> {
    use ferrimock::fake_data::png_to_jpeg;

    let base64_data = render(opts)?;
    let image_format = opts.format.as_str();
    let quality = opts.quality;
    let output = opts.output.as_deref();
    let as_base64 = opts.base64;
    let as_data_uri = opts.data_uri;
    let open_file = opts.open;

    // Convert to JPEG if requested
    let final_data =
        if image_format.to_lowercase() == "jpeg" || image_format.to_lowercase() == "jpg" {
            png_to_jpeg(&base64_data, Some(quality))
        } else {
            base64_data
        };

    // Determine MIME type
    let mime = if image_format.to_lowercase() == "jpeg" || image_format.to_lowercase() == "jpg" {
        "image/jpeg"
    } else {
        "image/png"
    };

    // Output handling
    if let Some(path) = output {
        let path = output_path(path, index, count);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&final_data)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
        if let Some(parent) = std::path::Path::new(&path).parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &bytes)?;
        crate::say!("{}", ui::success(&format!("Saved to {}", ui::path(&path))));

        if open_file {
            let _ = open::that(&path);
        }
    } else if as_data_uri {
        println!("data:{mime};base64,{final_data}");
    } else if as_base64 {
        println!("{final_data}");
    } else {
        // Default: save to temp file and show path
        let ext = if image_format.to_lowercase() == "jpeg" || image_format.to_lowercase() == "jpg" {
            "jpg"
        } else {
            "png"
        };
        let temp_path = std::env::temp_dir().join(format!(
            "fake-image-{}.{}",
            ferrimock::fake_data::fake_short_hash(),
            ext
        ));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&final_data)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
        std::fs::write(&temp_path, &bytes)?;
        println!(
            "{}",
            ui::success(&format!(
                "Generated: {}",
                ui::path(&temp_path.to_string_lossy())
            ))
        );

        if open_file {
            let _ = open::that(&temp_path);
        }
    }

    Ok(())
}
