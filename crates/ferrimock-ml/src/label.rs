//! The label space a classifier predicts over.
//!
//! [`ferrimock::type_detector::FieldType`] is a rich type: variants carry
//! ranges, sample URLs, enum members, whole nested analyses. That richness is
//! what template generation needs and exactly what a classifier cannot produce
//! -- a model predicts *which kind*, and the detector fills in the particulars.
//!
//! So classification runs over this flat enum, and a prediction is converted
//! back into a `FieldType` with neutral parameters. Anything a model cannot
//! usefully choose between (a nested object's full analysis, an array's element
//! type) is deliberately absent: those are structural, decided by looking at the
//! JSON rather than by learning.

use ferrimock::type_detector::FieldType;
use serde::{Deserialize, Serialize};

/// A field type a model can be asked to predict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldLabel {
    Uuid,
    Email,
    Url,
    ImageUrl,
    IsoDate,
    Timestamp,
    UnixTimestamp,
    PhoneNumber,
    IpAddress,
    Semver,
    HexString,
    Base64,
    CountryCode,
    CurrencyCode,
    LocaleCode,
    Timezone,
    PostalCode,
    MimeType,
    FileName,
    FilePath,
    Username,
    PersonName,
    Sentence,
    NumericStringId,
    Token,
    ETag,
    Boolean,
    Number,
    /// Nothing above fits. The residual is the interesting class: it is where
    /// the built-in detector gives up, and therefore where a model has room to
    /// be useful rather than merely agreeing.
    Opaque,
}

impl FieldLabel {
    /// Every label, in a fixed order. Index in this slice is the class index a
    /// model predicts, so the order is part of the model artifact's contract.
    pub const ALL: [Self; 29] = [
        Self::Uuid,
        Self::Email,
        Self::Url,
        Self::ImageUrl,
        Self::IsoDate,
        Self::Timestamp,
        Self::UnixTimestamp,
        Self::PhoneNumber,
        Self::IpAddress,
        Self::Semver,
        Self::HexString,
        Self::Base64,
        Self::CountryCode,
        Self::CurrencyCode,
        Self::LocaleCode,
        Self::Timezone,
        Self::PostalCode,
        Self::MimeType,
        Self::FileName,
        Self::FilePath,
        Self::Username,
        Self::PersonName,
        Self::Sentence,
        Self::NumericStringId,
        Self::Token,
        Self::ETag,
        Self::Boolean,
        Self::Number,
        Self::Opaque,
    ];

    pub fn class_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|label| *label == self)
            .unwrap_or(Self::ALL.len() - 1)
    }

    pub fn from_class_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::Email => "email",
            Self::Url => "url",
            Self::ImageUrl => "image_url",
            Self::IsoDate => "iso_date",
            Self::Timestamp => "timestamp",
            Self::UnixTimestamp => "unix_timestamp",
            Self::PhoneNumber => "phone_number",
            Self::IpAddress => "ip_address",
            Self::Semver => "semver",
            Self::HexString => "hex_string",
            Self::Base64 => "base64",
            Self::CountryCode => "country_code",
            Self::CurrencyCode => "currency_code",
            Self::LocaleCode => "locale_code",
            Self::Timezone => "timezone",
            Self::PostalCode => "postal_code",
            Self::MimeType => "mime_type",
            Self::FileName => "file_name",
            Self::FilePath => "file_path",
            Self::Username => "username",
            Self::PersonName => "person_name",
            Self::Sentence => "sentence",
            Self::NumericStringId => "numeric_string_id",
            Self::Token => "token",
            Self::ETag => "etag",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::Opaque => "opaque",
        }
    }

    /// The `FieldType` a prediction of this label stands for.
    ///
    /// Parameters are left neutral: a model says *what kind of thing* a field
    /// is, and the ranges and samples that make a template generate plausible
    /// values are read off the data by the detector.
    pub fn to_field_type(self) -> FieldType {
        match self {
            Self::Uuid => FieldType::Uuid,
            Self::Email => FieldType::Email,
            Self::Url => FieldType::Url,
            Self::ImageUrl => FieldType::ImageUrl,
            Self::IsoDate => FieldType::IsoDate {
                format: ferrimock::type_detector::DateFormat::Iso,
            },
            Self::Timestamp => FieldType::Timestamp {
                format: ferrimock::type_detector::TimestampFormat::Rfc3339Utc,
            },
            Self::UnixTimestamp => FieldType::UnixTimestamp,
            Self::PhoneNumber => FieldType::PhoneNumber,
            Self::IpAddress => FieldType::IpAddress,
            Self::Semver => FieldType::Semver,
            Self::HexString => FieldType::HexString {
                length: None,
                upper: false,
            },
            Self::Base64 => FieldType::Base64,
            Self::CountryCode => FieldType::CountryCode,
            Self::CurrencyCode => FieldType::CurrencyCode,
            Self::LocaleCode => FieldType::LocaleCode,
            Self::Timezone => FieldType::Timezone,
            Self::PostalCode => FieldType::PostalCode,
            Self::MimeType => FieldType::MimeType,
            Self::FileName => FieldType::FileName,
            Self::FilePath => FieldType::FilePath,
            Self::Username => FieldType::Username,
            Self::PersonName => FieldType::Name,
            Self::Sentence => FieldType::Sentence,
            Self::NumericStringId => FieldType::NumericStringId,
            Self::Token => FieldType::Token,
            Self::ETag => FieldType::ETag,
            Self::Boolean => FieldType::Boolean {
                spelling: ferrimock::type_detector::BooleanSpelling::default(),
            },
            Self::Number => FieldType::RandomNumber {
                min: None,
                max: None,
            },
            Self::Opaque => FieldType::RandomString,
        }
    }

    /// Which label a detector's answer corresponds to, for scoring the built-in
    /// heuristic on the same footing as a model.
    ///
    /// Structural answers -- an array, a nested object, a constant, an enum --
    /// are not in the label space and have no honest projection into it, so they
    /// return `None` and are excluded from the comparison rather than silently
    /// counted as `Opaque`.
    pub fn from_field_type(field_type: &FieldType) -> Option<Self> {
        Some(match field_type {
            // The wrapper says how the value was written, not what it is; the
            // label space is about the class.
            FieldType::Stringified(inner) => return Self::from_field_type(inner),
            FieldType::Uuid => Self::Uuid,
            FieldType::Email => Self::Email,
            FieldType::Url | FieldType::ApiEndpoint | FieldType::PaginationUrl(_) => Self::Url,
            FieldType::ImageUrl | FieldType::DownloadUrl { .. } => Self::ImageUrl,
            FieldType::IsoDate { .. } => Self::IsoDate,
            FieldType::Timestamp { .. } => Self::Timestamp,
            FieldType::UnixTimestamp
            | FieldType::MillisecondTimestamp
            | FieldType::MicrosecondTimestamp => Self::UnixTimestamp,
            FieldType::PhoneNumber => Self::PhoneNumber,
            FieldType::IpAddress => Self::IpAddress,
            FieldType::Semver => Self::Semver,
            FieldType::HexString { .. } => Self::HexString,
            FieldType::Base64 | FieldType::DataUri { .. } => Self::Base64,
            FieldType::CountryCode => Self::CountryCode,
            FieldType::CurrencyCode => Self::CurrencyCode,
            FieldType::LocaleCode => Self::LocaleCode,
            FieldType::Timezone => Self::Timezone,
            FieldType::PostalCode => Self::PostalCode,
            FieldType::MimeType => Self::MimeType,
            FieldType::FileName => Self::FileName,
            FieldType::FilePath => Self::FilePath,
            FieldType::Username => Self::Username,
            FieldType::Name => Self::PersonName,
            FieldType::Sentence | FieldType::Paragraph => Self::Sentence,
            FieldType::NumericStringId => Self::NumericStringId,
            FieldType::Token => Self::Token,
            FieldType::ETag => Self::ETag,
            FieldType::Boolean { .. } => Self::Boolean,
            FieldType::SequentialNumber { .. }
            | FieldType::RandomNumber { .. }
            | FieldType::RandomFloat { .. }
            | FieldType::FileSize
            | FieldType::Latitude
            | FieldType::Longitude => Self::Number,
            FieldType::RandomString => Self::Opaque,
            FieldType::Categorical { .. }
            | FieldType::Constant(_)
            | FieldType::Array(_)
            | FieldType::Object(_) => return None,
        })
    }
}

impl std::fmt::Display for FieldLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn class_indices_round_trip() {
        for label in FieldLabel::ALL {
            assert_eq!(
                FieldLabel::from_class_index(label.class_index()),
                Some(label)
            );
        }
    }

    #[test]
    fn indices_are_dense_and_unique() {
        let mut indices: Vec<usize> = FieldLabel::ALL
            .iter()
            .map(|label| label.class_index())
            .collect();
        indices.sort_unstable();
        indices.dedup();
        assert_eq!(
            indices.len(),
            FieldLabel::ALL.len(),
            "two labels share a class index, so a model would predict one and mean the other"
        );
        assert_eq!(indices.last(), Some(&(FieldLabel::ALL.len() - 1)));
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = FieldLabel::ALL.iter().map(|l| l.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), FieldLabel::ALL.len());
    }

    #[test]
    fn every_label_survives_a_trip_through_field_type() {
        for label in FieldLabel::ALL {
            let projected = FieldLabel::from_field_type(&label.to_field_type());
            assert_eq!(
                projected,
                Some(label),
                "{label} does not come back from its own FieldType"
            );
        }
    }

    #[test]
    fn structural_types_have_no_label() {
        // Scoring these against a flat label set would be scoring a category
        // error, so they are excluded rather than folded into `opaque`.
        assert_eq!(
            FieldLabel::from_field_type(&FieldType::Categorical { values: vec![] }),
            None
        );
        assert_eq!(
            FieldLabel::from_field_type(&FieldType::Constant(serde_json::Value::Null)),
            None
        );
    }
}
