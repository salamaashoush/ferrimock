use napi::bindgen_prelude::*;
use napi_derive::napi;

#[napi(object, namespace = "services")]
pub struct JsFakeImageInput {
    pub image_type: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub bg_color: Option<String>,
    pub text_color: Option<String>,
    pub text: Option<String>,
    pub initials: Option<String>,
    pub start_color: Option<String>,
    pub end_color: Option<String>,
    pub direction: Option<String>,
    pub image_format: Option<String>,
    pub quality: Option<u32>,
    pub colored: Option<bool>,
}

#[napi(object, namespace = "services")]
pub struct JsFakeImageResult {
    pub base64: String,
    pub mime_type: String,
}

#[napi(namespace = "services")]
pub fn fake_image(input: JsFakeImageInput) -> Result<JsFakeImageResult> {
    let defaults = mockpit::services::fake_image::FakeImageInput::default();
    let result =
        mockpit::services::fake_image::generate(mockpit::services::fake_image::FakeImageInput {
            image_type: input.image_type.unwrap_or(defaults.image_type),
            width: input.width.unwrap_or(defaults.width),
            height: input.height.unwrap_or(defaults.height),
            bg_color: input.bg_color,
            text_color: input.text_color,
            text: input.text,
            initials: input.initials,
            start_color: input.start_color,
            end_color: input.end_color,
            direction: input.direction.unwrap_or(defaults.direction),
            image_format: input.image_format.unwrap_or(defaults.image_format),
            quality: input.quality.unwrap_or(u32::from(defaults.quality)) as u8,
            colored: input.colored.unwrap_or(defaults.colored),
        })
        .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(JsFakeImageResult {
        base64: result.base64,
        mime_type: result.mime_type,
    })
}
