//! Asking the engine's own detector what it thinks.
//!
//! Everything here exists so the detector can be *examined*: scored against
//! ground truth, and asked for the raw answer it gave rather than a projection
//! of it. A projection is enough to score with and useless to fix with -- what a
//! fixer needs to know is that a ULID came back as `random_string`, not that it
//! came back as "not the right label".

use ferrimock::type_detector::{FieldType, TypeDetector};
use serde_json::Value as JsonValue;

/// What the built-in detector makes of a field.
///
/// The values go back to the JSON kinds they were recorded as first. Guessing
/// that from the text instead is how a numeric string id becomes a number before
/// the detector ever sees it, and then `random_number` is the right answer to
/// the wrong question.
pub fn detect(detector: &TypeDetector, field: &crate::Field<'_>) -> (FieldType, f64) {
    let json = field.json_values();
    let refs: Vec<&JsonValue> = json.iter().collect();
    detector.detect_type(field.name, &refs)
}

/// The name of a field type's variant, without its parameters.
///
/// The vocabulary a defect is reported in. Parameters are dropped on purpose:
/// `Categorical { values: [...] }` printed in full buries the one word that
/// matters under the data that made it.
#[allow(clippy::too_many_lines)] // One arm per variant; a catch-all would hide a new one
pub fn kind_of(field_type: &FieldType) -> &'static str {
    match field_type {
        // What it is, not how it was written.
        FieldType::Stringified(inner) => kind_of(inner),
        FieldType::SequentialNumber { .. } => "sequential_number",
        FieldType::RandomNumber { .. } => "random_number",
        FieldType::RandomFloat { .. } => "random_float",
        FieldType::Uuid => "uuid",
        FieldType::Timestamp { .. } => "timestamp",
        FieldType::Email => "email",
        FieldType::Username => "username",
        FieldType::Name => "name",
        FieldType::Sentence => "sentence",
        FieldType::Paragraph => "paragraph",
        FieldType::Url => "url",
        FieldType::ImageUrl => "image_url",
        FieldType::IpAddress => "ip_address",
        FieldType::PhoneNumber => "phone_number",
        FieldType::FileName => "file_name",
        FieldType::FileSize => "file_size",
        FieldType::DownloadUrl { .. } => "download_url",
        FieldType::DataUri { .. } => "data_uri",
        FieldType::Token => "token",
        FieldType::ETag => "etag",
        FieldType::MimeType => "mime_type",
        FieldType::RandomString => "random_string",
        FieldType::Boolean => "boolean",
        FieldType::Constant(_) => "constant",
        FieldType::Array(_) => "array",
        FieldType::Object(_) => "object",
        FieldType::NumericStringId => "numeric_string_id",
        FieldType::PaginationUrl(_) => "pagination_url",
        FieldType::ApiEndpoint => "api_endpoint",
        FieldType::IsoDate { .. } => "iso_date",
        FieldType::UnixTimestamp => "unix_timestamp",
        FieldType::MillisecondTimestamp => "millisecond_timestamp",
        FieldType::MicrosecondTimestamp => "microsecond_timestamp",
        FieldType::Semver => "semver",
        FieldType::HexString { .. } => "hex_string",
        FieldType::Base64 => "base64",
        FieldType::Latitude => "latitude",
        FieldType::Longitude => "longitude",
        FieldType::Categorical { .. } => "categorical",
        FieldType::CountryCode => "country_code",
        FieldType::CurrencyCode => "currency_code",
        FieldType::FilePath => "file_path",
        FieldType::PostalCode => "postal_code",
        FieldType::LocaleCode => "locale_code",
        FieldType::Timezone => "timezone",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn digits_recorded_as_text_reach_the_detector_as_text() {
        // The distinction the whole `ValueKind` exists for: these are the same
        // characters, and only one of them is a number.
        let detector = TypeDetector::new();
        let digits = ["12345678901", "98765432109", "55512345678"];

        let (as_text, _) = detect(&detector, &crate::Field::new("file_id", &digits));
        let (as_number, _) = detect(
            &detector,
            &crate::Field::new("file_id", &digits).of_kind(crate::ValueKind::Number),
        );

        assert_ne!(
            kind_of(&as_text),
            kind_of(&as_number),
            "quoting the digits has to change the answer, or the corpus is asking \
             the detector a question it cannot answer"
        );
    }

    #[test]
    fn the_detector_answers_with_a_kind_that_can_be_named() {
        let detector = TypeDetector::new();
        let values = [
            "550e8400-e29b-41d4-a716-446655440000",
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
        ];
        let (field_type, _) = detect(&detector, &crate::Field::new("id", &values));
        assert_eq!(kind_of(&field_type), "uuid");
    }

    #[test]
    fn a_ulid_is_read_as_a_token_today() {
        // Not an assertion that this is right -- it is the defect, pinned. A
        // ULID answered as `token` makes a merged mock fill the field with
        // `fake_token()`, which is thirty-two hex characters where twenty-six
        // Crockford base32 ones belong. Change the detector and change this.
        let detector = TypeDetector::new();
        let values = ["01ARZ3NDEKTSV4RRFFQ69G5FAV", "01BX5ZZKBKACTAV9WEVGEMMVRZ"];
        let (field_type, _) = detect(&detector, &crate::Field::new("reference", &values));
        assert_eq!(kind_of(&field_type), "token");
    }
}
