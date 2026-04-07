//! Fake image generation

use crate::commands::ui;
use base64::Engine;

/// Generate a fake image
#[allow(clippy::too_many_arguments)]
pub fn generate_fake_image(
    image_type: &str,
    width: u32,
    height: u32,
    bg_color: Option<&str>,
    text_color: Option<&str>,
    text: Option<&str>,
    initials: Option<&str>,
    start_color: Option<&str>,
    end_color: Option<&str>,
    direction: &str,
    image_format: &str,
    quality: u8,
    output: Option<&str>,
    as_base64: bool,
    as_data_uri: bool,
    colored: bool,
    open_file: bool,
) -> anyhow::Result<()> {
    use mockpit::fake_data::*;

    let base64_data = match image_type.to_lowercase().as_str() {
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
            let init = initials.unwrap_or("??");
            fake_avatar(Some(init), Some(width), bg_color, text_color)
        }
        "gradient" => {
            let start = start_color.unwrap_or("#FF0000");
            let end = end_color.unwrap_or("#0000FF");
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
        _ => {
            anyhow::bail!(
                "Unknown image type: '{image_type}'. Available: placeholder, avatar, gradient, checkerboard, noise, stripes, text, solid"
            );
        }
    };

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
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&final_data)
            .map_err(|e| anyhow::anyhow!("Failed to decode base64: {e}"))?;
        std::fs::write(path, &bytes)?;
        println!("{}", ui::success(&format!("Saved to {}", ui::path(path))));

        if open_file {
            let _ = open::that(path);
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
            mockpit::fake_data::fake_short_hash(),
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
