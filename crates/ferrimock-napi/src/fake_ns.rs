//! Direct fake data generators — mirrors everything registered in the Tera template system.
//!
//! ```ts
//! import { fake } from 'ferrimock'
//! fake.name()     // "John Doe"
//! fake.email()    // "john@example.com"
//! fake.uuid()     // "550e8400-..."
//! ```

use napi_derive::napi;

// ===== Identity =====
#[napi(namespace = "fake")]
pub fn name() -> String {
    ferrimock::fake_data::fake_name()
}
#[napi(namespace = "fake")]
pub fn first_name() -> String {
    ferrimock::fake_data::fake_first_name()
}
#[napi(namespace = "fake")]
pub fn last_name() -> String {
    ferrimock::fake_data::fake_last_name()
}
#[napi(namespace = "fake")]
pub fn username() -> String {
    ferrimock::fake_data::fake_username()
}
#[napi(namespace = "fake")]
pub fn password() -> String {
    ferrimock::fake_data::fake_password()
}
#[napi(namespace = "fake")]
pub fn title() -> String {
    ferrimock::fake_data::fake_title()
}
#[napi(namespace = "fake")]
pub fn suffix() -> String {
    ferrimock::fake_data::fake_suffix()
}

// ===== Contact =====
#[napi(namespace = "fake")]
pub fn email() -> String {
    ferrimock::fake_data::fake_email()
}
#[napi(namespace = "fake")]
pub fn free_email() -> String {
    ferrimock::fake_data::fake_free_email()
}
#[napi(namespace = "fake")]
pub fn phone() -> String {
    ferrimock::fake_data::fake_phone()
}
#[napi(namespace = "fake")]
pub fn cell_phone() -> String {
    ferrimock::fake_data::fake_cell_phone()
}

// ===== Location =====
#[napi(namespace = "fake")]
pub fn street() -> String {
    ferrimock::fake_data::fake_street()
}
#[napi(namespace = "fake")]
pub fn street_address() -> String {
    ferrimock::fake_data::fake_street_address()
}
#[napi(namespace = "fake")]
pub fn city() -> String {
    ferrimock::fake_data::fake_city()
}
#[napi(namespace = "fake")]
pub fn state() -> String {
    ferrimock::fake_data::fake_state()
}
#[napi(namespace = "fake")]
pub fn state_abbr() -> String {
    ferrimock::fake_data::fake_state_abbr()
}
#[napi(namespace = "fake")]
pub fn zip() -> String {
    ferrimock::fake_data::fake_zip()
}
#[napi(namespace = "fake")]
pub fn postal_code() -> String {
    ferrimock::fake_data::fake_postal_code()
}
#[napi(namespace = "fake")]
pub fn country() -> String {
    ferrimock::fake_data::fake_country()
}
#[napi(namespace = "fake")]
pub fn country_code() -> String {
    ferrimock::fake_data::fake_country_code()
}
#[napi(namespace = "fake")]
pub fn latitude() -> String {
    ferrimock::fake_data::fake_latitude()
}
#[napi(namespace = "fake")]
pub fn longitude() -> String {
    ferrimock::fake_data::fake_longitude()
}
#[napi(namespace = "fake")]
pub fn building_number() -> String {
    ferrimock::fake_data::fake_building_number()
}
#[napi(namespace = "fake")]
pub fn secondary_address() -> String {
    ferrimock::fake_data::fake_secondary_address()
}

// ===== Company =====
#[napi(namespace = "fake")]
pub fn company() -> String {
    ferrimock::fake_data::fake_company()
}
#[napi(namespace = "fake")]
pub fn company_suffix() -> String {
    ferrimock::fake_data::fake_company_suffix()
}
#[napi(namespace = "fake")]
pub fn job_title() -> String {
    ferrimock::fake_data::fake_job_title()
}
#[napi(namespace = "fake")]
pub fn industry() -> String {
    ferrimock::fake_data::fake_industry()
}
#[napi(namespace = "fake")]
pub fn job_field() -> String {
    ferrimock::fake_data::fake_job_field()
}
#[napi(namespace = "fake")]
pub fn job_position() -> String {
    ferrimock::fake_data::fake_job_position()
}
#[napi(namespace = "fake")]
pub fn job_seniority() -> String {
    ferrimock::fake_data::fake_job_seniority()
}

// ===== Internet =====
#[napi(namespace = "fake")]
pub fn url() -> String {
    ferrimock::fake_data::fake_url()
}
#[napi(namespace = "fake")]
pub fn domain() -> String {
    ferrimock::fake_data::fake_domain()
}
#[napi(namespace = "fake")]
pub fn ipv4() -> String {
    ferrimock::fake_data::fake_ipv4()
}
#[napi(namespace = "fake")]
pub fn ipv6() -> String {
    ferrimock::fake_data::fake_ipv6()
}
#[napi(namespace = "fake")]
pub fn mac_address() -> String {
    ferrimock::fake_data::fake_mac_address()
}
#[napi(namespace = "fake")]
pub fn user_agent() -> String {
    ferrimock::fake_data::fake_user_agent()
}
#[napi(namespace = "fake")]
pub fn user_agent_modern() -> String {
    ferrimock::fake_data::fake_user_agent_modern()
}
#[napi(namespace = "fake")]
pub fn hex_color() -> String {
    ferrimock::fake_data::fake_hex_color()
}
#[napi(namespace = "fake")]
pub fn rgb_color() -> String {
    ferrimock::fake_data::fake_rgb_color()
}
#[napi(namespace = "fake")]
pub fn color() -> String {
    ferrimock::fake_data::fake_color()
}
#[napi(namespace = "fake")]
pub fn pagination_url() -> String {
    ferrimock::fake_data::fake_pagination_url()
}
#[napi(namespace = "fake")]
pub fn pagination_url_offset() -> String {
    ferrimock::fake_data::fake_pagination_url_offset()
}
#[napi(namespace = "fake")]
pub fn search_url() -> String {
    ferrimock::fake_data::fake_search_url()
}
#[napi(namespace = "fake")]
pub fn file_download_url() -> String {
    ferrimock::fake_data::fake_file_download_url()
}
#[napi(namespace = "fake")]
pub fn api_url() -> String {
    ferrimock::fake_data::fake_api_url()
}
#[napi(namespace = "fake")]
pub fn webhook_url() -> String {
    ferrimock::fake_data::fake_webhook_url()
}
#[napi(namespace = "fake")]
pub fn api_endpoint() -> String {
    ferrimock::fake_data::fake_api_endpoint()
}
#[napi(namespace = "fake")]
pub fn resource_path() -> String {
    ferrimock::fake_data::fake_resource_path()
}

// ===== Finance =====
#[napi(namespace = "fake")]
pub fn credit_card() -> String {
    ferrimock::fake_data::fake_credit_card()
}
#[napi(namespace = "fake")]
pub fn currency_code() -> String {
    ferrimock::fake_data::fake_currency_code()
}
#[napi(namespace = "fake")]
pub fn currency_name() -> String {
    ferrimock::fake_data::fake_currency_name()
}
#[napi(namespace = "fake")]
pub fn currency_symbol() -> String {
    ferrimock::fake_data::fake_currency_symbol()
}
#[napi(namespace = "fake")]
pub fn amount() -> String {
    ferrimock::fake_data::fake_amount()
}
#[napi(namespace = "fake")]
pub fn price(min: Option<f64>, max: Option<f64>) -> f64 {
    ferrimock::fake_data::fake_price(min.unwrap_or(1.0), max.unwrap_or(9999.99))
}

// ===== Identifiers =====
#[napi(namespace = "fake")]
pub fn uuid() -> String {
    ferrimock::fake_data::fake_uuid()
}
#[napi(namespace = "fake")]
pub fn token() -> String {
    ferrimock::fake_data::fake_token()
}
#[napi(namespace = "fake")]
pub fn etag() -> String {
    ferrimock::fake_data::fake_etag()
}
#[napi(namespace = "fake")]
pub fn numeric_id() -> String {
    ferrimock::fake_data::fake_numeric_id()
}
#[napi(namespace = "fake")]
pub fn short_hash() -> String {
    ferrimock::fake_data::fake_short_hash()
}
#[napi(namespace = "fake")]
pub fn sha256() -> String {
    ferrimock::fake_data::fake_sha256()
}
#[napi(namespace = "fake")]
pub fn md5() -> String {
    ferrimock::fake_data::fake_md5()
}
#[napi(namespace = "fake")]
pub fn base64_data() -> String {
    ferrimock::fake_data::fake_base64()
}
#[napi(namespace = "fake")]
pub fn jwt() -> String {
    ferrimock::fake_data::fake_jwt()
}
#[napi(namespace = "fake")]
pub fn isbn() -> String {
    ferrimock::fake_data::fake_isbn()
}
#[napi(namespace = "fake")]
pub fn isbn13() -> String {
    ferrimock::fake_data::fake_isbn13()
}

// ===== DateTime =====
#[napi(namespace = "fake")]
pub fn date() -> String {
    ferrimock::fake_data::fake_date()
}
#[napi(namespace = "fake")]
pub fn time() -> String {
    ferrimock::fake_data::fake_time()
}
#[napi(namespace = "fake")]
pub fn iso_date() -> String {
    ferrimock::fake_data::fake_iso_date()
}
#[napi(namespace = "fake")]
pub fn unix_timestamp() -> i64 {
    ferrimock::fake_data::fake_unix_timestamp()
}
#[napi(namespace = "fake")]
pub fn relative_time() -> String {
    ferrimock::fake_data::fake_relative_time()
}

// ===== Text =====
#[napi(namespace = "fake")]
pub fn word() -> String {
    ferrimock::fake_data::fake_word()
}
#[napi(namespace = "fake")]
pub fn words(count: Option<u32>) -> String {
    ferrimock::fake_data::fake_words(count.unwrap_or(5) as usize)
}
#[napi(namespace = "fake")]
pub fn sentence(word_count: Option<u32>) -> String {
    ferrimock::fake_data::fake_sentence(word_count.unwrap_or(5) as usize)
}
#[napi(namespace = "fake")]
pub fn paragraph(sentence_count: Option<u32>) -> String {
    ferrimock::fake_data::fake_paragraph(sentence_count.unwrap_or(3) as usize)
}
#[napi(namespace = "fake")]
pub fn slug() -> String {
    ferrimock::fake_data::fake_slug()
}
#[napi(namespace = "fake")]
pub fn alphanumeric(length: Option<u32>) -> String {
    ferrimock::fake_data::fake_alphanumeric(length.unwrap_or(10) as usize)
}

// ===== Web =====
#[napi(namespace = "fake")]
pub fn boolean() -> bool {
    ferrimock::fake_data::fake_boolean()
}
#[napi(namespace = "fake")]
pub fn filename() -> String {
    ferrimock::fake_data::fake_filename()
}
#[napi(namespace = "fake")]
pub fn download_url() -> String {
    ferrimock::fake_data::fake_download_url()
}
#[napi(namespace = "fake")]
pub fn mime_type() -> String {
    ferrimock::fake_data::fake_mime_type()
}
#[napi(namespace = "fake")]
pub fn file_extension() -> String {
    ferrimock::fake_data::fake_file_extension()
}
#[napi(namespace = "fake")]
pub fn status_message() -> String {
    ferrimock::fake_data::fake_status_message()
}
#[napi(namespace = "fake")]
pub fn api_version() -> String {
    ferrimock::fake_data::fake_api_version()
}
#[napi(namespace = "fake")]
pub fn version() -> String {
    ferrimock::fake_data::fake_version()
}
#[napi(namespace = "fake")]
pub fn locale() -> String {
    ferrimock::fake_data::fake_locale()
}
#[napi(namespace = "fake")]
pub fn timezone() -> String {
    ferrimock::fake_data::fake_timezone()
}
#[napi(namespace = "fake")]
pub fn semver() -> String {
    ferrimock::fake_data::fake_semver()
}
#[napi(namespace = "fake")]
pub fn semver_prerelease() -> String {
    ferrimock::fake_data::fake_semver_prerelease()
}
#[napi(namespace = "fake")]
pub fn digit() -> i64 {
    ferrimock::fake_data::fake_digit()
}
#[napi(namespace = "fake")]
pub fn number(min: Option<i64>, max: Option<i64>) -> i64 {
    ferrimock::fake_data::fake_number(min.unwrap_or(1), max.unwrap_or(1000))
}
#[napi(namespace = "fake")]
pub fn float(min: Option<f64>, max: Option<f64>) -> f64 {
    ferrimock::fake_data::fake_float(min.unwrap_or(0.0), max.unwrap_or(1.0))
}
#[napi(namespace = "fake")]
pub fn file_size(min: Option<i64>, max: Option<i64>) -> i64 {
    ferrimock::fake_data::fake_file_size(min.unwrap_or(1024), max.unwrap_or(1_048_576))
}
