//! Web-specific generators (files, MIME types, etc.)

use fake::Fake;
use fake::faker::lorem::en::Word;
use rand::RngExt;
use rand::seq::IndexedRandom;
use uuid::Uuid;

/// Generate a random boolean value
pub fn fake_boolean() -> bool {
    rand::rng().random_bool(0.5)
}

/// Generate a random filename with extension
pub fn fake_filename() -> String {
    let name: String = Word().fake();
    let exts = ["pdf", "docx", "xlsx", "png", "jpg", "txt", "mp4", "zip"];
    let ext = exts.choose(&mut rand::rng()).copied().unwrap_or("pdf");
    format!("{name}.{ext}")
}

/// Generate a random file size in bytes (min, max)
pub fn fake_file_size(min: i64, max: i64) -> i64 {
    rand::rng().random_range(min..=max)
}

/// Generate a realistic long download URL
pub fn fake_download_url() -> String {
    let token = Uuid::new_v4().to_string().replace('-', "");
    format!(
        "https://dl.example.com/d/1/{}/?token={}&expires={}",
        token.get(..16).unwrap_or(&token),
        token.get(16..).unwrap_or_default(),
        chrono::Utc::now().timestamp() + 3600
    )
}

/// Generate a common MIME type
pub fn fake_mime_type() -> String {
    let types = [
        "application/json",
        "application/pdf",
        "application/xml",
        "text/html",
        "text/plain",
        "image/png",
        "image/jpeg",
        "video/mp4",
        "audio/mpeg",
    ];
    types
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or("application/json")
        .to_string()
}

/// Generate a random file extension
pub fn fake_file_extension() -> String {
    let exts = [
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "jpg", "png", "gif", "mp4",
        "mp3", "zip",
    ];
    exts.choose(&mut rand::rng())
        .copied()
        .unwrap_or("pdf")
        .to_string()
}

/// Generate a random HTTP status message
pub fn fake_status_message() -> String {
    let messages = [
        "OK",
        "Created",
        "Accepted",
        "No Content",
        "Bad Request",
        "Unauthorized",
        "Forbidden",
        "Not Found",
    ];
    messages
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or("OK")
        .to_string()
}

/// Generate a random API version string
pub fn fake_api_version() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    format!(
        "v{}.{}.{}",
        rng.random_range(0..=9),
        rng.random_range(0..=9),
        rng.random_range(0..=9)
    )
}

/// Generate a random semver version string
pub fn fake_version() -> String {
    use rand::RngExt;
    let mut rng = rand::rng();
    format!(
        "{}.{}.{}",
        rng.random_range(0..=9),
        rng.random_range(0..=9),
        rng.random_range(0..=9)
    )
}

/// Generate a random hex color code
pub fn fake_hex_color() -> String {
    format!("#{:06x}", rand::rng().random_range(0..0x00FF_FFFF))
}

/// Generate a random RGB color string
pub fn fake_rgb_color() -> String {
    format!(
        "rgb({}, {}, {})",
        rand::rng().random_range(0..=255),
        rand::rng().random_range(0..=255),
        rand::rng().random_range(0..=255)
    )
}

/// Generate a random locale code (en-US, fr-FR, etc.)
pub fn fake_locale() -> String {
    let locales = [
        "en-US", "en-GB", "en-CA", "en-AU", "en-NZ", "en-IE", "fr-FR", "fr-CA", "fr-BE", "fr-CH",
        "de-DE", "de-AT", "de-CH", "es-ES", "es-MX", "es-AR", "es-CL", "es-CO", "it-IT", "pt-BR",
        "pt-PT", "ja-JP", "zh-CN", "zh-TW", "zh-HK", "ko-KR", "ru-RU", "nl-NL", "nl-BE", "pl-PL",
        "tr-TR", "ar-SA", "ar-EG", "hi-IN", "th-TH", "vi-VN", "id-ID", "sv-SE", "no-NO", "da-DK",
        "fi-FI",
    ];
    locales
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or("en-US")
        .to_string()
}

/// Generate a random timezone
pub fn fake_timezone() -> String {
    let timezones = [
        "America/New_York",
        "America/Chicago",
        "America/Denver",
        "America/Los_Angeles",
        "America/Toronto",
        "America/Vancouver",
        "America/Montreal",
        "America/Mexico_City",
        "America/Bogota",
        "America/Lima",
        "America/Santiago",
        "America/Sao_Paulo",
        "America/Buenos_Aires",
        "America/Caracas",
        "Europe/London",
        "Europe/Paris",
        "Europe/Berlin",
        "Europe/Madrid",
        "Europe/Rome",
        "Europe/Amsterdam",
        "Europe/Brussels",
        "Europe/Vienna",
        "Europe/Stockholm",
        "Europe/Copenhagen",
        "Europe/Oslo",
        "Europe/Helsinki",
        "Europe/Warsaw",
        "Europe/Prague",
        "Europe/Athens",
        "Europe/Istanbul",
        "Asia/Tokyo",
        "Asia/Shanghai",
        "Asia/Hong_Kong",
        "Asia/Seoul",
        "Asia/Singapore",
        "Asia/Bangkok",
        "Asia/Jakarta",
        "Asia/Manila",
        "Asia/Dubai",
        "Asia/Kolkata",
        "Asia/Karachi",
        "Asia/Tehran",
        "Australia/Sydney",
        "Australia/Melbourne",
        "Australia/Brisbane",
        "Australia/Perth",
        "Pacific/Auckland",
        "Pacific/Honolulu",
        "Pacific/Fiji",
        "Africa/Cairo",
        "Africa/Johannesburg",
        "Africa/Lagos",
        "Africa/Nairobi",
    ];
    timezones
        .choose(&mut rand::rng())
        .copied()
        .unwrap_or("America/New_York")
        .to_string()
}

/// Generate a semantic version string
pub fn fake_semver() -> String {
    let major = rand::rng().random_range(0..=5);
    let minor = rand::rng().random_range(0..=20);
    let patch = rand::rng().random_range(0..=50);
    format!("{major}.{minor}.{patch}")
}

/// Generate a semantic version with pre-release tag
pub fn fake_semver_prerelease() -> String {
    let base = fake_semver();
    let tags = ["alpha", "beta", "rc"];
    let tag = tags.choose(&mut rand::rng()).copied().unwrap_or("alpha");
    let num = rand::rng().random_range(1..=10);
    format!("{base}-{tag}.{num}")
}

/// Generate a random digit (0-9)
pub fn fake_digit() -> i64 {
    rand::rng().random_range(0..=9)
}

/// Generate a random integer between min and max (inclusive)
pub fn fake_number(min: i64, max: i64) -> i64 {
    rand::rng().random_range(min..=max)
}

/// Generate a random float between min and max
pub fn fake_float(min: f64, max: f64) -> f64 {
    rand::rng().random_range(min..=max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_boolean() {
        let _value = fake_boolean();
    }

    #[test]
    fn test_fake_filename() {
        let filename = fake_filename();
        assert!(filename.contains('.'));
    }

    #[test]
    fn test_fake_file_size() {
        let size = fake_file_size(1000, 10000);
        assert!((1000..=10000).contains(&size));
    }

    #[test]
    fn test_fake_download_url() {
        let url = fake_download_url();
        assert!(url.starts_with("https://"));
        assert!(url.contains("token="));
        assert!(url.contains("expires="));
    }

    #[test]
    fn test_fake_mime_type() {
        let mime = fake_mime_type();
        assert!(mime.contains('/'));
    }

    #[test]
    fn test_fake_file_extension() {
        let ext = fake_file_extension();
        assert!(!ext.is_empty());
        assert!(!ext.contains('.'));
    }

    #[test]
    fn test_fake_status_message() {
        let msg = fake_status_message();
        assert!(!msg.is_empty());
    }

    #[test]
    fn test_fake_api_version() {
        let version = fake_api_version();
        assert!(version.starts_with('v'));
    }

    #[test]
    fn test_fake_version() {
        let version = fake_version();
        assert!(version.contains('.'));
    }

    #[test]
    fn test_fake_hex_color() {
        let color = fake_hex_color();
        assert!(color.starts_with('#'));
        assert_eq!(color.len(), 7);
    }

    #[test]
    fn test_fake_rgb_color() {
        let color = fake_rgb_color();
        assert!(color.starts_with("rgb("));
        assert!(color.ends_with(')'));
    }

    #[test]
    fn test_fake_locale() {
        let locale = fake_locale();
        assert!(locale.contains('-'));
    }

    #[test]
    fn test_fake_timezone() {
        let tz = fake_timezone();
        assert!(!tz.is_empty());
    }

    #[test]
    fn test_fake_semver() {
        let version = fake_semver();
        assert!(version.contains('.'));
        let parts: Vec<&str> = version.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.parse::<i32>().is_ok()));
    }

    #[test]
    fn test_fake_semver_prerelease() {
        let version = fake_semver_prerelease();
        assert!(version.contains('.'));
        assert!(version.contains('-'));
    }

    #[test]
    fn test_fake_number() {
        let num = fake_number(1, 100);
        assert!((1..=100).contains(&num));
    }

    #[test]
    fn test_fake_float() {
        let num = fake_float(1.0, 10.0);
        assert!((1.0..=10.0).contains(&num));
    }

    #[test]
    fn test_fake_digit() {
        let digit = fake_digit();
        assert!((0..=9).contains(&digit));
    }
}
