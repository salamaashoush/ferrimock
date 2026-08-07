//! The fake data generator registry.
//!
//! Generators are plain functions over a JSON argument map — they know nothing
//! about a template engine. Tera and the QuickJS `fake.*` binding are both
//! consumers of this registry, so neither has to go through the other.

use crate::error::FerrimockError;
use chrono::{Duration, Utc};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Named arguments passed to a generator.
pub type Args = HashMap<String, Value>;

/// A fake data generator: named arguments in, JSON out.
pub type Generator = fn(&Args) -> Result<Value>;

type Result<T> = std::result::Result<T, FerrimockError>;

/// Look up a generator by name.
pub fn generator(name: &str) -> Option<Generator> {
    registry().get(name).copied()
}

/// Every built-in generator, keyed by the name templates and scripts call it by.
pub fn registry() -> &'static HashMap<&'static str, Generator> {
    static REGISTRY: std::sync::OnceLock<HashMap<&'static str, Generator>> =
        std::sync::OnceLock::new();
    REGISTRY.get_or_init(build_registry)
}

#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
fn build_registry() -> HashMap<&'static str, Generator> {
    let mut generators: HashMap<&'static str, Generator> = HashMap::with_capacity(128);
    let mut register = |name: &'static str, f: Generator| {
        generators.insert(name, f);
    };

    // uuid() - Generates a random UUID v4 (commonly used, so we include it here)
    register("uuid", |_: &Args| -> Result<Value> {
        Ok(Value::from(Uuid::new_v4().to_string()))
    });

    // ========== Identity & Personal Data ==========
    register("fake_name", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_name()))
    });

    register("fake_first_name", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_first_name()))
    });

    register("fake_last_name", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_last_name()))
    });

    register("fake_username", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_username()))
    });

    register("fake_password", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_password()))
    });

    register("fake_title", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_title()))
    });

    register("fake_suffix", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identity::fake_suffix()))
    });

    // ========== Contact Information ==========
    register("fake_email", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::contact::fake_email()))
    });

    register("fake_free_email", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::contact::fake_free_email()))
    });

    register("fake_phone", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::contact::fake_phone()))
    });

    register("fake_cell_phone", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::contact::fake_cell_phone()))
    });

    // ========== Location & Address ==========
    register("fake_street", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_street()))
    });

    register("fake_street_address", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::location::fake_street_address(),
        ))
    });

    register("fake_city", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_city()))
    });

    register("fake_state", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_state()))
    });

    register("fake_state_abbr", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_state_abbr()))
    });

    register("fake_zip", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_zip()))
    });

    register("fake_country", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_country()))
    });

    register("fake_country_code", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_country_code()))
    });

    register("fake_latitude", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_latitude()))
    });

    register("fake_longitude", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_longitude()))
    });

    register("fake_postal_code", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::location::fake_postal_code()))
    });

    register("fake_building_number", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::location::fake_building_number(),
        ))
    });

    register("fake_secondary_address", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::location::fake_secondary_address(),
        ))
    });

    // ========== Company & Job ==========
    register("fake_company", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_company()))
    });

    register("fake_company_suffix", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_company_suffix()))
    });

    register("fake_job_title", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_job_title()))
    });

    register("fake_industry", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_industry()))
    });

    register("fake_job_field", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_job_field()))
    });

    register("fake_job_position", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_job_position()))
    });

    register("fake_job_seniority", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::company::fake_job_seniority()))
    });

    // ========== Internet & Networking ==========
    register("fake_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_url()))
    });

    register("fake_domain", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_domain()))
    });

    register("fake_ipv4", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_ipv4()))
    });

    register("fake_ipv6", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_ipv6()))
    });

    register("fake_mac_address", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_mac_address()))
    });

    register("fake_user_agent", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_user_agent()))
    });

    register("fake_color", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_color()))
    });

    register("fake_pagination_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::internet::fake_pagination_url(),
        ))
    });

    register("fake_pagination_url_offset", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::internet::fake_pagination_url_offset(),
        ))
    });

    register("fake_search_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_search_url()))
    });

    register("fake_file_download_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::internet::fake_file_download_url(),
        ))
    });

    register("fake_api_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_api_url()))
    });

    register("fake_webhook_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_webhook_url()))
    });

    register("fake_api_endpoint", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_api_endpoint()))
    });

    register("fake_resource_path", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::internet::fake_resource_path()))
    });

    register("fake_user_agent_modern", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::internet::fake_user_agent_modern(),
        ))
    });

    // ========== Text & Content ==========
    register("fake_words", |args: &Args| -> Result<Value> {
        let count = args.get("count").and_then(Value::as_u64).unwrap_or(5) as usize;
        Ok(Value::from(crate::fake_data::text::fake_words(count)))
    });

    register("fake_sentence", |args: &Args| -> Result<Value> {
        let word_count = args.get("word_count").and_then(Value::as_u64).unwrap_or(5) as usize;
        Ok(Value::from(crate::fake_data::text::fake_sentence(
            word_count,
        )))
    });

    register("fake_paragraph", |args: &Args| -> Result<Value> {
        let sentence_count = args
            .get("sentence_count")
            .and_then(Value::as_u64)
            .unwrap_or(3) as usize;
        Ok(Value::from(crate::fake_data::text::fake_paragraph(
            sentence_count,
        )))
    });

    register("fake_word", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::text::fake_word()))
    });

    register("fake_slug", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::text::fake_slug()))
    });

    register("fake_alphanumeric", |args: &Args| -> Result<Value> {
        let length = args.get("length").and_then(Value::as_u64).unwrap_or(10) as usize;
        Ok(Value::from(crate::fake_data::text::fake_alphanumeric(
            length,
        )))
    });

    // ========== Finance & Commerce ==========
    register("fake_credit_card", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::finance::fake_credit_card()))
    });

    register("fake_currency_code", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::finance::fake_currency_code()))
    });

    register("fake_currency_name", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::finance::fake_currency_name()))
    });

    register("fake_currency_symbol", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::finance::fake_currency_symbol(),
        ))
    });

    register("fake_price", |args: &Args| -> Result<Value> {
        let min = args.get("min").and_then(Value::as_f64).unwrap_or(1.0);
        let max = args.get("max").and_then(Value::as_f64).unwrap_or(9999.99);
        let price = crate::fake_data::finance::fake_price(min, max);
        Ok(Value::from(price))
    });

    register("fake_amount", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::finance::fake_amount()))
    });

    // ========== Identifiers & Codes ==========
    register("fake_uuid", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_uuid()))
    });

    register("fake_isbn", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_isbn()))
    });

    register("fake_isbn13", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_isbn13()))
    });

    register("fake_token", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_token()))
    });

    register("fake_etag", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_etag()))
    });

    register("fake_numeric_id", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_numeric_id()))
    });

    register("fake_short_hash", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_short_hash()))
    });

    register("fake_sha256", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_sha256()))
    });

    register("fake_sha1", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_sha1()))
    });

    register("fake_md5", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_md5()))
    });

    register("fake_base64", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_base64()))
    });

    register("fake_jwt", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::identifiers::fake_jwt()))
    });

    // ========== Dates & Times ==========
    register("fake_date", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::datetime::fake_date()))
    });

    register("fake_time", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::datetime::fake_time()))
    });

    register("fake_iso_date", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::datetime::fake_iso_date()))
    });

    register("fake_unix_timestamp", |_: &Args| -> Result<Value> {
        Ok(Value::from(
            crate::fake_data::datetime::fake_unix_timestamp(),
        ))
    });

    register("fake_relative_time", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::datetime::fake_relative_time()))
    });

    // ========== Web-Specific ==========
    register("fake_boolean", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_boolean()))
    });

    register("fake_filename", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_filename()))
    });

    register("fake_file_size", |args: &Args| -> Result<Value> {
        let min = args.get("min").and_then(Value::as_i64).unwrap_or(1024);
        let max = args.get("max").and_then(Value::as_i64).unwrap_or(1_048_576);
        Ok(Value::from(crate::fake_data::web::fake_file_size(min, max)))
    });

    register("fake_download_url", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_download_url()))
    });

    register("fake_mime_type", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_mime_type()))
    });

    register("fake_file_extension", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_file_extension()))
    });

    register("fake_status_message", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_status_message()))
    });

    register("fake_api_version", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_api_version()))
    });

    register("fake_version", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_version()))
    });

    register("fake_hex_color", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_hex_color()))
    });

    register("fake_rgb_color", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_rgb_color()))
    });

    register("fake_locale", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_locale()))
    });

    register("fake_timezone", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_timezone()))
    });

    register("fake_semver", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_semver()))
    });

    register("fake_semver_prerelease", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_semver_prerelease()))
    });

    register("fake_digit", |_: &Args| -> Result<Value> {
        Ok(Value::from(crate::fake_data::web::fake_digit()))
    });

    register("fake_number", |args: &Args| -> Result<Value> {
        let min = args.get("min").and_then(Value::as_i64).unwrap_or(1);
        let max = args.get("max").and_then(Value::as_i64).unwrap_or(1000);
        Ok(Value::from(crate::fake_data::web::fake_number(min, max)))
    });

    register("fake_float", |args: &Args| -> Result<Value> {
        let min = args.get("min").and_then(Value::as_f64).unwrap_or(0.0);
        let max = args.get("max").and_then(Value::as_f64).unwrap_or(1.0);
        let float_val = crate::fake_data::web::fake_float(min, max);
        Ok(Value::from(float_val))
    });

    // ========== File Generation (PDF, Images) ==========
    register("fake_pdf", |args: &Args| -> Result<Value> {
        let text = args.get("text").and_then(Value::as_str);
        let pages = args.get("pages").and_then(Value::as_u64).map(|v| v as u32);
        Ok(Value::from(crate::fake_data::files::fake_pdf(text, pages)))
    });

    register("fake_png", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let color = args.get("color").and_then(Value::as_str);
        Ok(Value::from(crate::fake_data::files::fake_png(
            width, height, color,
        )))
    });

    register("fake_jpeg", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let color = args.get("color").and_then(Value::as_str);
        let quality = args.get("quality").and_then(Value::as_u64).map(|v| v as u8);
        Ok(Value::from(crate::fake_data::files::fake_jpeg(
            width, height, color, quality,
        )))
    });

    register("fake_pdf_data_uri", |args: &Args| -> Result<Value> {
        let text = args.get("text").and_then(Value::as_str);
        let pages = args.get("pages").and_then(Value::as_u64).map(|v| v as u32);
        Ok(Value::from(crate::fake_data::files::fake_pdf_data_uri(
            text, pages,
        )))
    });

    register("fake_png_data_uri", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let color = args.get("color").and_then(Value::as_str);
        Ok(Value::from(crate::fake_data::files::fake_png_data_uri(
            width, height, color,
        )))
    });

    register("fake_jpeg_data_uri", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let color = args.get("color").and_then(Value::as_str);
        let quality = args.get("quality").and_then(Value::as_u64).map(|v| v as u8);
        Ok(Value::from(crate::fake_data::files::fake_jpeg_data_uri(
            width, height, color, quality,
        )))
    });

    register("fake_image_with_text", |args: &Args| -> Result<Value> {
        let text = args.get("text").and_then(Value::as_str);
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let bg_color = args.get("bg_color").and_then(Value::as_str);
        let text_color = args.get("text_color").and_then(Value::as_str);
        let font_size = args
            .get("font_size")
            .and_then(Value::as_f64)
            .map(|v| v as f32);
        Ok(Value::from(crate::fake_data::files::fake_image_with_text(
            text, width, height, bg_color, text_color, font_size,
        )))
    });

    register("fake_image_gradient", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let start_color = args.get("start_color").and_then(Value::as_str);
        let end_color = args.get("end_color").and_then(Value::as_str);
        let direction = args.get("direction").and_then(Value::as_str);
        Ok(Value::from(crate::fake_data::files::fake_image_gradient(
            width,
            height,
            start_color,
            end_color,
            direction,
        )))
    });

    register("fake_image_checkerboard", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let color1 = args.get("color1").and_then(Value::as_str);
        let color2 = args.get("color2").and_then(Value::as_str);
        let square_size = args
            .get("square_size")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        Ok(Value::from(
            crate::fake_data::files::fake_image_checkerboard(
                width,
                height,
                color1,
                color2,
                square_size,
            ),
        ))
    });

    register("fake_image_noise", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let colored = args.get("colored").and_then(Value::as_bool);
        Ok(Value::from(crate::fake_data::files::fake_image_noise(
            width, height, colored,
        )))
    });

    register("fake_image_stripes", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let color1 = args.get("color1").and_then(Value::as_str);
        let color2 = args.get("color2").and_then(Value::as_str);
        let stripe_width = args
            .get("stripe_width")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        let direction = args.get("direction").and_then(Value::as_str);
        Ok(Value::from(crate::fake_data::files::fake_image_stripes(
            width,
            height,
            color1,
            color2,
            stripe_width,
            direction,
        )))
    });

    register("fake_placeholder", |args: &Args| -> Result<Value> {
        let width = args.get("width").and_then(Value::as_u64).map(|v| v as u32);
        let height = args.get("height").and_then(Value::as_u64).map(|v| v as u32);
        let text = args.get("text").and_then(Value::as_str);
        let bg_color = args.get("bg_color").and_then(Value::as_str);
        let text_color = args.get("text_color").and_then(Value::as_str);
        Ok(Value::from(crate::fake_data::files::fake_placeholder(
            width, height, text, bg_color, text_color,
        )))
    });

    register("fake_avatar", |args: &Args| -> Result<Value> {
        let initials = args.get("initials").and_then(Value::as_str);
        let size = args.get("size").and_then(Value::as_u64).map(|v| v as u32);
        let bg_color = args.get("bg_color").and_then(Value::as_str);
        let text_color = args.get("text_color").and_then(Value::as_str);
        Ok(Value::from(crate::fake_data::files::fake_avatar(
            initials, size, bg_color, text_color,
        )))
    });

    // ========== Date Arithmetic ==========

    // now_plus(days=0, hours=0, minutes=0, seconds=0, format="%Y-%m-%dT%H:%M:%S%.3fZ")
    // Returns a date/time offset from now into the future.
    // Example: {{ now_plus(days=30) }} -> "2026-03-09T14:30:00.000Z"
    // Example: {{ now_plus(hours=2, format="%Y-%m-%d") }} -> "2026-02-07"
    register("now_plus", |args: &Args| -> Result<Value> {
        let days = args.get("days").and_then(Value::as_i64).unwrap_or(0);
        let hours = args.get("hours").and_then(Value::as_i64).unwrap_or(0);
        let minutes = args.get("minutes").and_then(Value::as_i64).unwrap_or(0);
        let seconds = args.get("seconds").and_then(Value::as_i64).unwrap_or(0);
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("%Y-%m-%dT%H:%M:%S%.3fZ");

        let offset = Duration::days(days)
            + Duration::hours(hours)
            + Duration::minutes(minutes)
            + Duration::seconds(seconds);
        let result = Utc::now() + offset;
        Ok(Value::from(result.format(format).to_string()))
    });

    // now_minus(days=0, hours=0, minutes=0, seconds=0, format="%Y-%m-%dT%H:%M:%S%.3fZ")
    // Returns a date/time offset from now into the past.
    // Example: {{ now_minus(days=7) }} -> "2026-01-31T14:30:00.000Z"
    register("now_minus", |args: &Args| -> Result<Value> {
        let days = args.get("days").and_then(Value::as_i64).unwrap_or(0);
        let hours = args.get("hours").and_then(Value::as_i64).unwrap_or(0);
        let minutes = args.get("minutes").and_then(Value::as_i64).unwrap_or(0);
        let seconds = args.get("seconds").and_then(Value::as_i64).unwrap_or(0);
        let format = args
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or("%Y-%m-%dT%H:%M:%S%.3fZ");

        let offset = Duration::days(days)
            + Duration::hours(hours)
            + Duration::minutes(minutes)
            + Duration::seconds(seconds);
        let result = Utc::now() - offset;
        Ok(Value::from(result.format(format).to_string()))
    });

    // fake_iso_date_offset(days=0) - Generate date relative to today
    // Example: {{ fake_iso_date_offset(days=-7) }} -> "2026-01-31"
    // Example: {{ fake_iso_date_offset(days=30) }} -> "2026-03-09"
    register("fake_iso_date_offset", |args: &Args| -> Result<Value> {
        let days = args.get("days").and_then(Value::as_i64).unwrap_or(0);
        let result = Utc::now() + Duration::days(days);
        Ok(Value::from(result.format("%Y-%m-%d").to_string()))
    });

    // ========== Array Generation Helper ==========

    // fake_array(type="name", count=5) - Generate an array of fake data
    // Supported types: name, email, uuid, company, city, phone, url, word,
    //   sentence, number, boolean, date, username, job_title, ipv4
    // Example: {{ fake_array(type="name", count=3) }} -> ["John Doe", "Jane Smith", "Bob Wilson"]
    // Example: {{ fake_array(type="email", count=2) }} -> ["john@example.com", "jane@test.org"]
    register("fake_array", |args: &Args| -> Result<Value> {
        let data_type = args.get("type").and_then(Value::as_str).ok_or_else(|| {
            FerrimockError::Template("fake_array requires 'type' parameter".to_string())
        })?;
        let count = args.get("count").and_then(Value::as_u64).unwrap_or(5) as usize;

        let items: Vec<Value> = (0..count)
            .map(|_| match data_type {
                "name" => Value::from(crate::fake_data::identity::fake_name()),
                "first_name" => Value::from(crate::fake_data::identity::fake_first_name()),
                "last_name" => Value::from(crate::fake_data::identity::fake_last_name()),
                "username" => Value::from(crate::fake_data::identity::fake_username()),
                "email" => Value::from(crate::fake_data::contact::fake_email()),
                "phone" => Value::from(crate::fake_data::contact::fake_phone()),
                "company" => Value::from(crate::fake_data::company::fake_company()),
                "job_title" => Value::from(crate::fake_data::company::fake_job_title()),
                "city" => Value::from(crate::fake_data::location::fake_city()),
                "country" => Value::from(crate::fake_data::location::fake_country()),
                "url" => Value::from(crate::fake_data::internet::fake_url()),
                "domain" => Value::from(crate::fake_data::internet::fake_domain()),
                "ipv4" => Value::from(crate::fake_data::internet::fake_ipv4()),
                "uuid" => Value::from(crate::fake_data::identifiers::fake_uuid()),
                "word" => Value::from(crate::fake_data::text::fake_word()),
                "sentence" => Value::from(crate::fake_data::text::fake_sentence(5)),
                "date" => Value::from(crate::fake_data::datetime::fake_iso_date()),
                "number" => Value::from(crate::fake_data::web::fake_number(1, 1000)),
                "boolean" => Value::from(crate::fake_data::web::fake_boolean()),
                other => Value::from(format!("[unknown type: {other}]")),
            })
            .collect();

        Ok(Value::Array(items))
    });

    generators
}

/// Expose every generator to Tera. Tera calls in with `Kwargs`, so translate at
/// the boundary and leave the generators themselves engine-agnostic.
pub fn register_all_functions(tera: &mut tera::Tera) {
    for (name, generator) in registry() {
        let generator = *generator;
        tera.register_function(
            *name,
            move |kwargs: tera::Kwargs, _: &tera::State<'_>| -> tera::TeraResult<tera::Value> {
                let args = super::convert::kwargs_to_args(&kwargs);
                let value = generator(&args).map_err(tera::Error::message)?;
                Ok(super::convert::to_tera(value))
            },
        );
    }
}
