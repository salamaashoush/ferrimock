//! Fake image generation service.

/// Input for generating a fake image.
#[derive(Debug, Clone)]
pub struct FakeImageInput {
    /// Image type: placeholder, avatar, gradient, checkerboard, noise, stripes, text, solid
    pub image_type: String,
    pub width: u32,
    pub height: u32,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub text: Option<String>,
    pub initials: Option<String>,
    pub start_color: Option<String>,
    pub end_color: Option<String>,
    pub direction: String,
    pub image_format: String,
    pub quality: u8,
    pub colored: bool,
}

impl Default for FakeImageInput {
    fn default() -> Self {
        Self {
            image_type: "placeholder".into(),
            width: 200,
            height: 200,
            bg_color: None,
            text_color: None,
            text: None,
            initials: None,
            start_color: None,
            end_color: None,
            direction: "horizontal".into(),
            image_format: "png".into(),
            quality: 85,
            colored: false,
        }
    }
}

/// Result of image generation.
#[derive(Debug, Clone)]
pub struct FakeImageResult {
    pub base64: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// Generate a fake image.
#[allow(clippy::needless_pass_by_value)] // owned input is the service API boundary
pub fn generate(input: FakeImageInput) -> Result<FakeImageResult, crate::MockpitError> {
    use crate::fake_data::*;

    let w = Some(input.width);
    let h = Some(input.height);

    let base64_data = match input.image_type.as_str() {
        "placeholder" => fake_placeholder(
            w,
            h,
            input.text.as_deref(),
            input.bg_color.as_deref(),
            input.text_color.as_deref(),
        ),
        "avatar" => fake_avatar(
            input.initials.as_deref(),
            w,
            input.bg_color.as_deref(),
            input.text_color.as_deref(),
        ),
        "gradient" => fake_image_gradient(
            w,
            h,
            input.start_color.as_deref(),
            input.end_color.as_deref(),
            Some(input.direction.as_str()),
        ),
        "checkerboard" => fake_image_checkerboard(
            w,
            h,
            input.bg_color.as_deref(),
            input.text_color.as_deref(),
            None, // square_size
        ),
        "noise" => fake_image_noise(w, h, Some(input.colored)),
        "stripes" => fake_image_stripes(
            w,
            h,
            input.bg_color.as_deref(),
            input.text_color.as_deref(),
            None, // stripe_width
            Some(input.direction.as_str()),
        ),
        "text" => fake_image_with_text(
            input.text.as_deref(),
            w,
            h,
            input.bg_color.as_deref(),
            input.text_color.as_deref(),
            None, // font_size
        ),
        "solid" | "color" => fake_png(w, h, input.bg_color.as_deref()),
        other => crate::mp_bail!("Unknown image type: {other}"),
    };

    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &base64_data)
        .map_err(|e| crate::mp_err!("Failed to decode image: {e}"))?;

    let mime_type = if input.image_format == "jpeg" || input.image_format == "jpg" {
        "image/jpeg".into()
    } else {
        "image/png".into()
    };

    Ok(FakeImageResult {
        base64: base64_data,
        mime_type,
        bytes,
    })
}
