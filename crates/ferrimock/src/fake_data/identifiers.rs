//! Identifiers and codes generators

use fake::Fake;
use fake::faker::barcode::en::*;
use rand::RngExt;
use uuid::Uuid;

/// Generate a random UUID v4
pub fn fake_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Generate a random ISBN
pub fn fake_isbn() -> String {
    Isbn().fake()
}

/// Generate a random ISBN13
pub fn fake_isbn13() -> String {
    Isbn13().fake()
}

/// Generate a random authentication token
pub fn fake_token() -> String {
    Uuid::new_v4().to_string().replace('-', "")
}

/// Generate an HTTP ETag value
pub fn fake_etag() -> String {
    let version = rand::rng().random_range(0..100);
    format!("{version}")
}

/// Generate a numeric string ID (like database IDs)
pub fn fake_numeric_id() -> String {
    let id = rand::rng().random_range(1_000_000_000..=9_999_999_999_999_i64);
    id.to_string()
}

/// Generate a short hash (like Git short SHA)
pub fn fake_short_hash() -> String {
    format!("{:x}", rand::rng().random_range(0x0010_0000..=0x00FF_FFFF))
}

/// Generate a full SHA-256 hash
pub fn fake_sha256() -> String {
    use std::fmt::Write;
    (0..64).fold(String::with_capacity(64), |mut output, _| {
        let _ = write!(output, "{:x}", rand::rng().random_range(0..16));
        output
    })
}

/// Generate a MD5 hash
pub fn fake_md5() -> String {
    use std::fmt::Write;
    (0..32).fold(String::with_capacity(32), |mut output, _| {
        let _ = write!(output, "{:x}", rand::rng().random_range(0..16));
        output
    })
}

/// Generate a base64 encoded string
pub fn fake_base64() -> String {
    use base64::{Engine as _, engine::general_purpose};
    let bytes: Vec<u8> = (0..24).map(|_| rand::rng().random_range(0..=255)).collect();
    general_purpose::STANDARD.encode(&bytes)
}

/// Generate a JWT-like token
pub fn fake_jwt() -> String {
    let header = fake_base64();
    let payload = fake_base64();
    let signature = fake_base64();
    format!("{header}.{payload}.{signature}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_uuid() {
        let uuid = fake_uuid();
        assert_eq!(uuid.len(), 36);
        assert_eq!(
            uuid.chars()
                .nth(8)
                .expect("should have character at position 8"),
            '-'
        );
    }

    #[test]
    fn test_fake_isbn() {
        let isbn = fake_isbn();
        assert!(!isbn.is_empty());
    }

    #[test]
    fn test_fake_token() {
        let token = fake_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(char::is_alphanumeric));
    }

    #[test]
    fn test_fake_numeric_id() {
        let id = fake_numeric_id();
        assert!(id.len() >= 10);
        assert!(id.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn test_fake_short_hash() {
        let hash = fake_short_hash();
        assert!(hash.len() >= 5);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_fake_sha256() {
        let hash = fake_sha256();
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_fake_md5() {
        let hash = fake_md5();
        assert_eq!(hash.len(), 32);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_fake_base64() {
        let encoded = fake_base64();
        assert!(!encoded.is_empty());
        assert!(
            encoded
                .chars()
                .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
        );
    }

    #[test]
    fn test_fake_jwt() {
        let jwt = fake_jwt();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| !p.is_empty()));
    }
}
