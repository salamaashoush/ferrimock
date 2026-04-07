//! Tera template registration for fake data generators
//!
//! This module registers fake data generation functions with Tera templates.
//! The actual fake data generators are in the bdg-fake-data crate.

// Tera library callbacks require std::collections::HashMap - cannot use FxHashMap
#![allow(clippy::disallowed_types)]

use chrono::{Duration, Utc};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

// ============================================================================
// TERA REGISTRATION HELPER
// ============================================================================

/// Register all fake data generators with a Tera instance
///
/// This function registers all fake data generation functions as Tera functions
/// that can be used in templates.
pub fn register_all_functions(tera: &mut tera::Tera) {
    // uuid() - Generates a random UUID v4 (commonly used, so we include it here)
    tera.register_function(
        "uuid",
        |_args: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(Uuid::new_v4().to_string()))
        },
    );

    // ========== Identity & Personal Data ==========
    tera.register_function(
        "fake_name",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_name()))
        },
    );

    tera.register_function(
        "fake_first_name",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_first_name()))
        },
    );

    tera.register_function(
        "fake_last_name",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_last_name()))
        },
    );

    tera.register_function(
        "fake_username",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_username()))
        },
    );

    tera.register_function(
        "fake_password",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_password()))
        },
    );

    tera.register_function(
        "fake_title",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_title()))
        },
    );

    tera.register_function(
        "fake_suffix",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identity::fake_suffix()))
        },
    );

    // ========== Contact Information ==========
    tera.register_function(
        "fake_email",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::contact::fake_email()))
        },
    );

    tera.register_function(
        "fake_free_email",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::contact::fake_free_email()))
        },
    );

    tera.register_function(
        "fake_phone",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::contact::fake_phone()))
        },
    );

    tera.register_function(
        "fake_cell_phone",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::contact::fake_cell_phone()))
        },
    );

    // ========== Location & Address ==========
    tera.register_function(
        "fake_street",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_street()))
        },
    );

    tera.register_function(
        "fake_street_address",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::location::fake_street_address(),
            ))
        },
    );

    tera.register_function(
        "fake_city",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_city()))
        },
    );

    tera.register_function(
        "fake_state",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_state()))
        },
    );

    tera.register_function(
        "fake_state_abbr",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_state_abbr()))
        },
    );

    tera.register_function(
        "fake_zip",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_zip()))
        },
    );

    tera.register_function(
        "fake_country",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_country()))
        },
    );

    tera.register_function(
        "fake_country_code",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::location::fake_country_code(),
            ))
        },
    );

    tera.register_function(
        "fake_latitude",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_latitude()))
        },
    );

    tera.register_function(
        "fake_longitude",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::location::fake_longitude()))
        },
    );

    tera.register_function(
        "fake_postal_code",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::location::fake_postal_code(),
            ))
        },
    );

    tera.register_function(
        "fake_building_number",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::location::fake_building_number(),
            ))
        },
    );

    tera.register_function(
        "fake_secondary_address",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::location::fake_secondary_address(),
            ))
        },
    );

    // ========== Company & Job ==========
    tera.register_function(
        "fake_company",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::company::fake_company()))
        },
    );

    tera.register_function(
        "fake_company_suffix",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::company::fake_company_suffix(),
            ))
        },
    );

    tera.register_function(
        "fake_job_title",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::company::fake_job_title()))
        },
    );

    tera.register_function(
        "fake_industry",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::company::fake_industry()))
        },
    );

    tera.register_function(
        "fake_job_field",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::company::fake_job_field()))
        },
    );

    tera.register_function(
        "fake_job_position",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::company::fake_job_position(),
            ))
        },
    );

    tera.register_function(
        "fake_job_seniority",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::company::fake_job_seniority(),
            ))
        },
    );

    // ========== Internet & Networking ==========
    tera.register_function(
        "fake_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_url()))
        },
    );

    tera.register_function(
        "fake_domain",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_domain()))
        },
    );

    tera.register_function(
        "fake_ipv4",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_ipv4()))
        },
    );

    tera.register_function(
        "fake_ipv6",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_ipv6()))
        },
    );

    tera.register_function(
        "fake_mac_address",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_mac_address(),
            ))
        },
    );

    tera.register_function(
        "fake_user_agent",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_user_agent()))
        },
    );

    tera.register_function(
        "fake_color",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_color()))
        },
    );

    tera.register_function(
        "fake_pagination_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_pagination_url(),
            ))
        },
    );

    tera.register_function(
        "fake_pagination_url_offset",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_pagination_url_offset(),
            ))
        },
    );

    tera.register_function(
        "fake_search_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_search_url()))
        },
    );

    tera.register_function(
        "fake_file_download_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_file_download_url(),
            ))
        },
    );

    tera.register_function(
        "fake_api_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::internet::fake_api_url()))
        },
    );

    tera.register_function(
        "fake_webhook_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_webhook_url(),
            ))
        },
    );

    tera.register_function(
        "fake_api_endpoint",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_api_endpoint(),
            ))
        },
    );

    tera.register_function(
        "fake_resource_path",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_resource_path(),
            ))
        },
    );

    tera.register_function(
        "fake_user_agent_modern",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::internet::fake_user_agent_modern(),
            ))
        },
    );

    // ========== Text & Content ==========
    tera.register_function(
        "fake_words",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            Ok(Value::String(mockpit_fake_data::text::fake_words(count)))
        },
    );

    tera.register_function(
        "fake_sentence",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let word_count = args.get("word_count").and_then(|v| v.as_u64()).unwrap_or(5) as usize;
            Ok(Value::String(mockpit_fake_data::text::fake_sentence(
                word_count,
            )))
        },
    );

    tera.register_function(
        "fake_paragraph",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let sentence_count = args
                .get("sentence_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(3) as usize;
            Ok(Value::String(mockpit_fake_data::text::fake_paragraph(
                sentence_count,
            )))
        },
    );

    tera.register_function(
        "fake_word",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::text::fake_word()))
        },
    );

    tera.register_function(
        "fake_slug",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::text::fake_slug()))
        },
    );

    tera.register_function(
        "fake_alphanumeric",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let length = args.get("length").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
            Ok(Value::String(mockpit_fake_data::text::fake_alphanumeric(
                length,
            )))
        },
    );

    // ========== Finance & Commerce ==========
    tera.register_function(
        "fake_credit_card",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::finance::fake_credit_card()))
        },
    );

    tera.register_function(
        "fake_currency_code",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::finance::fake_currency_code(),
            ))
        },
    );

    tera.register_function(
        "fake_currency_name",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::finance::fake_currency_name(),
            ))
        },
    );

    tera.register_function(
        "fake_currency_symbol",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::finance::fake_currency_symbol(),
            ))
        },
    );

    tera.register_function(
        "fake_price",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let min = args.get("min").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let max = args.get("max").and_then(|v| v.as_f64()).unwrap_or(9999.99);
            let price = mockpit_fake_data::finance::fake_price(min, max);
            Ok(Value::Number(
                serde_json::Number::from_f64(price).unwrap_or_else(|| serde_json::Number::from(0)),
            ))
        },
    );

    tera.register_function(
        "fake_amount",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::finance::fake_amount()))
        },
    );

    // ========== Identifiers & Codes ==========
    tera.register_function(
        "fake_uuid",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_uuid()))
        },
    );

    tera.register_function(
        "fake_isbn",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_isbn()))
        },
    );

    tera.register_function(
        "fake_isbn13",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_isbn13()))
        },
    );

    tera.register_function(
        "fake_token",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_token()))
        },
    );

    tera.register_function(
        "fake_etag",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_etag()))
        },
    );

    tera.register_function(
        "fake_numeric_id",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::identifiers::fake_numeric_id(),
            ))
        },
    );

    tera.register_function(
        "fake_short_hash",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::identifiers::fake_short_hash(),
            ))
        },
    );

    tera.register_function(
        "fake_sha256",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_sha256()))
        },
    );

    tera.register_function(
        "fake_md5",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_md5()))
        },
    );

    tera.register_function(
        "fake_base64",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_base64()))
        },
    );

    tera.register_function(
        "fake_jwt",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::identifiers::fake_jwt()))
        },
    );

    // ========== Dates & Times ==========
    tera.register_function(
        "fake_date",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::datetime::fake_date()))
        },
    );

    tera.register_function(
        "fake_time",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::datetime::fake_time()))
        },
    );

    tera.register_function(
        "fake_iso_date",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::datetime::fake_iso_date()))
        },
    );

    tera.register_function(
        "fake_unix_timestamp",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::Number(
                mockpit_fake_data::datetime::fake_unix_timestamp().into(),
            ))
        },
    );

    tera.register_function(
        "fake_relative_time",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::datetime::fake_relative_time(),
            ))
        },
    );

    // ========== Web-Specific ==========
    tera.register_function(
        "fake_boolean",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::Bool(mockpit_fake_data::web::fake_boolean()))
        },
    );

    tera.register_function(
        "fake_filename",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_filename()))
        },
    );

    tera.register_function(
        "fake_file_size",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let min = args.get("min").and_then(|v| v.as_i64()).unwrap_or(1024);
            let max = args.get("max").and_then(|v| v.as_i64()).unwrap_or(1048576);
            Ok(Value::Number(
                mockpit_fake_data::web::fake_file_size(min, max).into(),
            ))
        },
    );

    tera.register_function(
        "fake_download_url",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_download_url()))
        },
    );

    tera.register_function(
        "fake_mime_type",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_mime_type()))
        },
    );

    tera.register_function(
        "fake_file_extension",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_file_extension()))
        },
    );

    tera.register_function(
        "fake_status_message",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_status_message()))
        },
    );

    tera.register_function(
        "fake_api_version",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_api_version()))
        },
    );

    tera.register_function(
        "fake_version",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_version()))
        },
    );

    tera.register_function(
        "fake_hex_color",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_hex_color()))
        },
    );

    tera.register_function(
        "fake_rgb_color",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_rgb_color()))
        },
    );

    tera.register_function(
        "fake_locale",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_locale()))
        },
    );

    tera.register_function(
        "fake_timezone",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_timezone()))
        },
    );

    tera.register_function(
        "fake_semver",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(mockpit_fake_data::web::fake_semver()))
        },
    );

    tera.register_function(
        "fake_semver_prerelease",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::String(
                mockpit_fake_data::web::fake_semver_prerelease(),
            ))
        },
    );

    tera.register_function(
        "fake_digit",
        |_: &HashMap<String, Value>| -> tera::Result<Value> {
            Ok(Value::Number(mockpit_fake_data::web::fake_digit().into()))
        },
    );

    tera.register_function(
        "fake_number",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let min = args.get("min").and_then(|v| v.as_i64()).unwrap_or(1);
            let max = args.get("max").and_then(|v| v.as_i64()).unwrap_or(1000);
            Ok(Value::Number(
                mockpit_fake_data::web::fake_number(min, max).into(),
            ))
        },
    );

    tera.register_function(
        "fake_float",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let min = args.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let max = args.get("max").and_then(|v| v.as_f64()).unwrap_or(1.0);
            let float_val = mockpit_fake_data::web::fake_float(min, max);
            Ok(Value::Number(
                serde_json::Number::from_f64(float_val)
                    .unwrap_or_else(|| serde_json::Number::from(0)),
            ))
        },
    );

    // ========== File Generation (PDF, Images) ==========
    tera.register_function(
        "fake_pdf",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let text = args.get("text").and_then(|v| v.as_str());
            let pages = args.get("pages").and_then(|v| v.as_u64()).map(|v| v as u32);
            Ok(Value::String(mockpit_fake_data::files::fake_pdf(
                text, pages,
            )))
        },
    );

    tera.register_function(
        "fake_png",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let color = args.get("color").and_then(|v| v.as_str());
            Ok(Value::String(mockpit_fake_data::files::fake_png(
                width, height, color,
            )))
        },
    );

    tera.register_function(
        "fake_jpeg",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let color = args.get("color").and_then(|v| v.as_str());
            let quality = args
                .get("quality")
                .and_then(|v| v.as_u64())
                .map(|v| v as u8);
            Ok(Value::String(mockpit_fake_data::files::fake_jpeg(
                width, height, color, quality,
            )))
        },
    );

    tera.register_function(
        "fake_pdf_data_uri",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let text = args.get("text").and_then(|v| v.as_str());
            let pages = args.get("pages").and_then(|v| v.as_u64()).map(|v| v as u32);
            Ok(Value::String(mockpit_fake_data::files::fake_pdf_data_uri(
                text, pages,
            )))
        },
    );

    tera.register_function(
        "fake_png_data_uri",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let color = args.get("color").and_then(|v| v.as_str());
            Ok(Value::String(mockpit_fake_data::files::fake_png_data_uri(
                width, height, color,
            )))
        },
    );

    tera.register_function(
        "fake_jpeg_data_uri",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let color = args.get("color").and_then(|v| v.as_str());
            let quality = args
                .get("quality")
                .and_then(|v| v.as_u64())
                .map(|v| v as u8);
            Ok(Value::String(mockpit_fake_data::files::fake_jpeg_data_uri(
                width, height, color, quality,
            )))
        },
    );

    tera.register_function(
        "fake_image_with_text",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let text = args.get("text").and_then(|v| v.as_str());
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let bg_color = args.get("bg_color").and_then(|v| v.as_str());
            let text_color = args.get("text_color").and_then(|v| v.as_str());
            let font_size = args
                .get("font_size")
                .and_then(|v| v.as_f64())
                .map(|v| v as f32);
            Ok(Value::String(
                mockpit_fake_data::files::fake_image_with_text(
                    text, width, height, bg_color, text_color, font_size,
                ),
            ))
        },
    );

    tera.register_function(
        "fake_image_gradient",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let start_color = args.get("start_color").and_then(|v| v.as_str());
            let end_color = args.get("end_color").and_then(|v| v.as_str());
            let direction = args.get("direction").and_then(|v| v.as_str());
            Ok(Value::String(
                mockpit_fake_data::files::fake_image_gradient(
                    width,
                    height,
                    start_color,
                    end_color,
                    direction,
                ),
            ))
        },
    );

    tera.register_function(
        "fake_image_checkerboard",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let color1 = args.get("color1").and_then(|v| v.as_str());
            let color2 = args.get("color2").and_then(|v| v.as_str());
            let square_size = args
                .get("square_size")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            Ok(Value::String(
                mockpit_fake_data::files::fake_image_checkerboard(
                    width,
                    height,
                    color1,
                    color2,
                    square_size,
                ),
            ))
        },
    );

    tera.register_function(
        "fake_image_noise",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let colored = args.get("colored").and_then(|v| v.as_bool());
            Ok(Value::String(mockpit_fake_data::files::fake_image_noise(
                width, height, colored,
            )))
        },
    );

    tera.register_function(
        "fake_image_stripes",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let color1 = args.get("color1").and_then(|v| v.as_str());
            let color2 = args.get("color2").and_then(|v| v.as_str());
            let stripe_width = args
                .get("stripe_width")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let direction = args.get("direction").and_then(|v| v.as_str());
            Ok(Value::String(mockpit_fake_data::files::fake_image_stripes(
                width,
                height,
                color1,
                color2,
                stripe_width,
                direction,
            )))
        },
    );

    tera.register_function(
        "fake_placeholder",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let width = args.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            let height = args
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let text = args.get("text").and_then(|v| v.as_str());
            let bg_color = args.get("bg_color").and_then(|v| v.as_str());
            let text_color = args.get("text_color").and_then(|v| v.as_str());
            Ok(Value::String(mockpit_fake_data::files::fake_placeholder(
                width, height, text, bg_color, text_color,
            )))
        },
    );

    tera.register_function(
        "fake_avatar",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let initials = args.get("initials").and_then(|v| v.as_str());
            let size = args.get("size").and_then(|v| v.as_u64()).map(|v| v as u32);
            let bg_color = args.get("bg_color").and_then(|v| v.as_str());
            let text_color = args.get("text_color").and_then(|v| v.as_str());
            Ok(Value::String(mockpit_fake_data::files::fake_avatar(
                initials, size, bg_color, text_color,
            )))
        },
    );

    // ========== Date Arithmetic ==========

    // now_plus(days=0, hours=0, minutes=0, seconds=0, format="%Y-%m-%dT%H:%M:%S%.3fZ")
    // Returns a date/time offset from now into the future.
    // Example: {{ now_plus(days=30) }} -> "2026-03-09T14:30:00.000Z"
    // Example: {{ now_plus(hours=2, format="%Y-%m-%d") }} -> "2026-02-07"
    tera.register_function(
        "now_plus",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(0);
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(0);
            let minutes = args.get("minutes").and_then(|v| v.as_i64()).unwrap_or(0);
            let seconds = args.get("seconds").and_then(|v| v.as_i64()).unwrap_or(0);
            let format = args
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("%Y-%m-%dT%H:%M:%S%.3fZ");

            let offset = Duration::days(days)
                + Duration::hours(hours)
                + Duration::minutes(minutes)
                + Duration::seconds(seconds);
            let result = Utc::now() + offset;
            Ok(Value::String(result.format(format).to_string()))
        },
    );

    // now_minus(days=0, hours=0, minutes=0, seconds=0, format="%Y-%m-%dT%H:%M:%S%.3fZ")
    // Returns a date/time offset from now into the past.
    // Example: {{ now_minus(days=7) }} -> "2026-01-31T14:30:00.000Z"
    tera.register_function(
        "now_minus",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(0);
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(0);
            let minutes = args.get("minutes").and_then(|v| v.as_i64()).unwrap_or(0);
            let seconds = args.get("seconds").and_then(|v| v.as_i64()).unwrap_or(0);
            let format = args
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("%Y-%m-%dT%H:%M:%S%.3fZ");

            let offset = Duration::days(days)
                + Duration::hours(hours)
                + Duration::minutes(minutes)
                + Duration::seconds(seconds);
            let result = Utc::now() - offset;
            Ok(Value::String(result.format(format).to_string()))
        },
    );

    // fake_iso_date_offset(days=0) - Generate date relative to today
    // Example: {{ fake_iso_date_offset(days=-7) }} -> "2026-01-31"
    // Example: {{ fake_iso_date_offset(days=30) }} -> "2026-03-09"
    tera.register_function(
        "fake_iso_date_offset",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(0);
            let result = Utc::now() + Duration::days(days);
            Ok(Value::String(result.format("%Y-%m-%d").to_string()))
        },
    );

    // ========== Mock File Service URL Helpers ==========
    //
    // These functions construct URLs to the built-in file service at /__box_dev_gate_files/
    // for use in mock templates. They accept the same params as the HTTP endpoints.

    // mock_pdf_url(text="...", pages=2, filename="doc.pdf", id="...")
    //   -> "/__box_dev_gate_files/pdf/{id}?text=...&pages=2&filename=doc.pdf"
    tera.register_function(
        "mock_pdf_url",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let mut params = Vec::new();
            if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                params.push(format!("text={}", urlencoding::encode(text)));
            }
            if let Some(pages) = args.get("pages").and_then(|v| v.as_u64()) {
                params.push(format!("pages={}", pages));
            }
            if let Some(filename) = args.get("filename").and_then(|v| v.as_str()) {
                params.push(format!("filename={}", urlencoding::encode(filename)));
            }

            let query = if params.is_empty() {
                String::new()
            } else {
                format!("?{}", params.join("&"))
            };

            Ok(Value::String(format!(
                "/__box_dev_gate_files/pdf/{}{}",
                id, query
            )))
        },
    );

    // mock_image_url(width=800, height=600, type="placeholder", text="...", format="png", id="...")
    //   -> "/__box_dev_gate_files/image/{id}?width=800&height=600&type=placeholder&text=..."
    tera.register_function(
        "mock_image_url",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let mut params = Vec::new();
            if let Some(w) = args.get("width").and_then(|v| v.as_u64()) {
                params.push(format!("width={}", w));
            }
            if let Some(h) = args.get("height").and_then(|v| v.as_u64()) {
                params.push(format!("height={}", h));
            }
            if let Some(t) = args.get("type").and_then(|v| v.as_str()) {
                params.push(format!("type={}", urlencoding::encode(t)));
            }
            if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                params.push(format!("text={}", urlencoding::encode(text)));
            }
            if let Some(fmt) = args.get("format").and_then(|v| v.as_str()) {
                params.push(format!("format={}", urlencoding::encode(fmt)));
            }
            if let Some(q) = args.get("quality").and_then(|v| v.as_u64()) {
                params.push(format!("quality={}", q));
            }
            if let Some(bg) = args.get("bg").and_then(|v| v.as_str()) {
                params.push(format!("bg={}", urlencoding::encode(bg)));
            }
            if let Some(fg) = args.get("fg").and_then(|v| v.as_str()) {
                params.push(format!("fg={}", urlencoding::encode(fg)));
            }
            if let Some(color) = args.get("color").and_then(|v| v.as_str()) {
                params.push(format!("color={}", urlencoding::encode(color)));
            }
            if let Some(start) = args.get("start").and_then(|v| v.as_str()) {
                params.push(format!("start={}", urlencoding::encode(start)));
            }
            if let Some(end) = args.get("end").and_then(|v| v.as_str()) {
                params.push(format!("end={}", urlencoding::encode(end)));
            }
            if let Some(dir) = args.get("direction").and_then(|v| v.as_str()) {
                params.push(format!("direction={}", urlencoding::encode(dir)));
            }
            if let Some(sq) = args.get("square").and_then(|v| v.as_u64()) {
                params.push(format!("square={}", sq));
            }
            if let Some(st) = args.get("stripe").and_then(|v| v.as_u64()) {
                params.push(format!("stripe={}", st));
            }
            if let Some(colored) = args.get("colored").and_then(|v| v.as_bool()) {
                params.push(format!("colored={}", colored));
            }

            let query = if params.is_empty() {
                String::new()
            } else {
                format!("?{}", params.join("&"))
            };

            Ok(Value::String(format!(
                "/__box_dev_gate_files/image/{}{}",
                id, query
            )))
        },
    );

    // mock_avatar_url(initials="SA", size=200, bg="#FF6B6B", fg="#FFFFFF", id="...")
    //   -> "/__box_dev_gate_files/avatar/{id}?initials=SA&size=200&bg=%23FF6B6B"
    tera.register_function(
        "mock_avatar_url",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());

            let mut params = Vec::new();
            if let Some(initials) = args.get("initials").and_then(|v| v.as_str()) {
                params.push(format!("initials={}", urlencoding::encode(initials)));
            }
            if let Some(size) = args.get("size").and_then(|v| v.as_u64()) {
                params.push(format!("size={}", size));
            }
            if let Some(bg) = args.get("bg").and_then(|v| v.as_str()) {
                params.push(format!("bg={}", urlencoding::encode(bg)));
            }
            if let Some(fg) = args.get("fg").and_then(|v| v.as_str()) {
                params.push(format!("fg={}", urlencoding::encode(fg)));
            }

            let query = if params.is_empty() {
                String::new()
            } else {
                format!("?{}", params.join("&"))
            };

            Ok(Value::String(format!(
                "/__box_dev_gate_files/avatar/{}{}",
                id, query
            )))
        },
    );

    // mock_font_url(family="DejaVuSans")
    //   -> "/__box_dev_gate_files/font?family=DejaVuSans"
    tera.register_function(
        "mock_font_url",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let mut params = Vec::new();
            if let Some(family) = args.get("family").and_then(|v| v.as_str()) {
                params.push(format!("family={}", urlencoding::encode(family)));
            }

            let query = if params.is_empty() {
                String::new()
            } else {
                format!("?{}", params.join("&"))
            };

            Ok(Value::String(format!(
                "/__box_dev_gate_files/font{}",
                query
            )))
        },
    );

    // ========== Array Generation Helper ==========

    // fake_array(type="name", count=5) - Generate an array of fake data
    // Supported types: name, email, uuid, company, city, phone, url, word,
    //   sentence, number, boolean, date, username, job_title, ipv4
    // Example: {{ fake_array(type="name", count=3) }} -> ["John Doe", "Jane Smith", "Bob Wilson"]
    // Example: {{ fake_array(type="email", count=2) }} -> ["john@example.com", "jane@test.org"]
    tera.register_function(
        "fake_array",
        |args: &HashMap<String, Value>| -> tera::Result<Value> {
            let data_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| tera::Error::msg("fake_array requires 'type' parameter"))?;
            let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

            let items: Vec<Value> = (0..count)
                .map(|_| match data_type {
                    "name" => Value::String(mockpit_fake_data::identity::fake_name()),
                    "first_name" => Value::String(mockpit_fake_data::identity::fake_first_name()),
                    "last_name" => Value::String(mockpit_fake_data::identity::fake_last_name()),
                    "username" => Value::String(mockpit_fake_data::identity::fake_username()),
                    "email" => Value::String(mockpit_fake_data::contact::fake_email()),
                    "phone" => Value::String(mockpit_fake_data::contact::fake_phone()),
                    "company" => Value::String(mockpit_fake_data::company::fake_company()),
                    "job_title" => Value::String(mockpit_fake_data::company::fake_job_title()),
                    "city" => Value::String(mockpit_fake_data::location::fake_city()),
                    "country" => Value::String(mockpit_fake_data::location::fake_country()),
                    "url" => Value::String(mockpit_fake_data::internet::fake_url()),
                    "domain" => Value::String(mockpit_fake_data::internet::fake_domain()),
                    "ipv4" => Value::String(mockpit_fake_data::internet::fake_ipv4()),
                    "uuid" => Value::String(mockpit_fake_data::identifiers::fake_uuid()),
                    "word" => Value::String(mockpit_fake_data::text::fake_word()),
                    "sentence" => Value::String(mockpit_fake_data::text::fake_sentence(5)),
                    "date" => Value::String(mockpit_fake_data::datetime::fake_iso_date()),
                    "number" => Value::Number(mockpit_fake_data::web::fake_number(1, 1000).into()),
                    "boolean" => Value::Bool(mockpit_fake_data::web::fake_boolean()),
                    other => Value::String(format!("[unknown type: {}]", other)),
                })
                .collect();

            Ok(Value::Array(items))
        },
    );
}
