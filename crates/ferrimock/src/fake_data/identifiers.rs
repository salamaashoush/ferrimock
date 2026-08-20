//! Identifiers and codes generators

use super::rng::rng;
use fake::Fake;
use fake::faker::barcode::en::*;
use rand::RngExt;

/// Generate a random UUID v4
pub fn fake_uuid() -> String {
    super::rng::uuid_v4().to_string()
}

/// Generate a random ISBN
/// A ULID: a millisecond timestamp then randomness, in Crockford base32.
///
/// The point is the ordering. A v4 uuid carries nothing, so sorting a
/// collection by id puts it in an order unrelated to when anything happened —
/// which no real API does, because every id family in use either counts or
/// embeds a clock. Twenty-six characters, lexicographically sortable, and
/// still opaque to a client.
#[must_use]
pub fn ulid_at(millis: i64, high: u64, low: u64) -> String {
    const CROCKFORD: [u8; 32] = *b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

    let mut written = String::with_capacity(26);
    let time = u64::try_from(millis.max(0)).unwrap_or(0) & ((1 << 48) - 1);
    // Ten characters of time, most significant first, then sixteen of noise.
    for slot in (0..10).rev() {
        let at = usize::try_from((time >> (slot * 5)) & 0x1f).unwrap_or(0);
        written.push(char::from(CROCKFORD.get(at).copied().unwrap_or(b'0')));
    }
    for word in [high, low] {
        for slot in (0..8_u32).rev() {
            let at = usize::try_from((word >> (slot * 5)) & 0x1f).unwrap_or(0);
            written.push(char::from(CROCKFORD.get(at).copied().unwrap_or(b'0')));
        }
    }
    written
}

/// A short prefix an entity's ids can be told apart by.
///
/// A real API's opaque ids say what they address — `fold_`, `usr_`, `whsec_` —
/// so an id pasted into a bug report can be recognised without its context.
#[must_use]
pub fn id_prefix(entity: &str) -> String {
    let letters: String = entity
        .chars()
        .filter(char::is_ascii_alphabetic)
        .flat_map(char::to_lowercase)
        .take(4)
        .collect();
    if letters.is_empty() {
        "obj".to_string()
    } else {
        letters
    }
}

pub fn fake_isbn() -> String {
    Isbn().fake_with_rng(&mut rng())
}

/// Generate a random ISBN13
pub fn fake_isbn13() -> String {
    Isbn13().fake_with_rng(&mut rng())
}

/// Generate a random authentication token
pub fn fake_token() -> String {
    super::rng::uuid_v4().to_string().replace('-', "")
}

/// Generate an HTTP ETag value
pub fn fake_etag() -> String {
    let version = rng().random_range(0..100);
    format!("{version}")
}

/// Generate a numeric string ID (like database IDs)
pub fn fake_numeric_id() -> String {
    let id = rng().random_range(1_000_000_000..=9_999_999_999_999_i64);
    id.to_string()
}

/// Generate a short hash (like Git short SHA)
pub fn fake_short_hash() -> String {
    format!("{:x}", rng().random_range(0x0010_0000..=0x00FF_FFFF))
}

/// Generate a full SHA-256 hash
pub fn fake_sha256() -> String {
    use std::fmt::Write;
    (0..64).fold(String::with_capacity(64), |mut output, _| {
        let _ = write!(output, "{:x}", rng().random_range(0..16));
        output
    })
}

/// Generate a full SHA-1 hash
pub fn fake_sha1() -> String {
    use std::fmt::Write;
    (0..40).fold(String::with_capacity(40), |mut output, _| {
        let _ = write!(output, "{:x}", rng().random_range(0..16));
        output
    })
}

/// Generate a MD5 hash
pub fn fake_md5() -> String {
    use std::fmt::Write;
    (0..32).fold(String::with_capacity(32), |mut output, _| {
        let _ = write!(output, "{:x}", rng().random_range(0..16));
        output
    })
}

/// Generate a hexadecimal string of a given width and case.
///
/// The width is carried because a field of forty-character SHA-1s answered with
/// a thirty-two character MD5 is the wrong value, however right the class is.
pub fn fake_hex(length: usize, upper: bool) -> String {
    use std::fmt::Write;

    let width = length.clamp(1, 512);
    (0..width).fold(String::with_capacity(width), |mut output, _| {
        let digit = rng().random_range(0..16);
        let _ = if upper {
            write!(output, "{digit:X}")
        } else {
            write!(output, "{digit:x}")
        };
        output
    })
}

/// Generate a base64 encoded string
pub fn fake_base64() -> String {
    use base64::{Engine as _, engine::general_purpose};
    let bytes: Vec<u8> = (0..24).map(|_| rng().random_range(0..=255)).collect();
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
