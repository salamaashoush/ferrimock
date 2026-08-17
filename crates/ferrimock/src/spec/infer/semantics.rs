//! What a field means, when all you have is its name and declared type.
//!
//! [`detect_from_semantic_context`] is built for recordings: most of its rules
//! confirm a guess about the name against the values that were actually seen,
//! so with no samples they cannot fire. A spec has no samples — it has
//! something the recording path never gets instead, a declared type name and a
//! declared format. This maps those two.
//!
//! Case conventions are handled by the detector's own matcher, so `createdAt`,
//! `created_at` and `CREATED-AT` all land in the same place.

use crate::core::world::model::TextShape;
use crate::type_detector::semantic::matches_field_name;
use crate::type_detector::{DateFormat, FieldType, TimestampFormat};

/// Infer a field's meaning from its name and, failing that, from the name of
/// the type the spec gave it.
#[must_use]
pub fn semantic_of(field_name: &str, type_name: &str, format: Option<&str>) -> Option<FieldType> {
    // A declared format is the spec stating the answer, so it wins.
    if let Some(format) = format
        && let Some(field_type) = from_format(format)
    {
        return Some(field_type);
    }
    if let Some(field_type) = from_type_name(type_name) {
        return Some(field_type);
    }
    from_field_name(field_name)
}

/// OpenAPI `format` and the common nonstandard spellings around it.
#[must_use]
pub fn from_format(format: &str) -> Option<FieldType> {
    Some(match format.to_ascii_lowercase().as_str() {
        "uuid" | "guid" => FieldType::Uuid,
        "email" | "idn-email" => FieldType::Email,
        "uri" | "url" | "iri" | "hostname" | "idn-hostname" => FieldType::Url,
        "ipv4" | "ipv6" | "ip" => FieldType::IpAddress,
        "date" => FieldType::IsoDate {
            format: DateFormat::Iso,
        },
        "date-time" | "datetime" => FieldType::Timestamp {
            format: TimestampFormat::Rfc3339Utc,
        },
        "phone" | "phone-number" | "tel" => FieldType::PhoneNumber,
        // `byte`, `binary` and `password` have no faithful scalar rendering,
        // so the declared kind answers instead of a wrong-shaped guess.
        _ => return None,
    })
}

/// A custom scalar's name is a declaration in its own right: a schema that
/// bothered to define `DateTime` means it.
#[must_use]
pub fn from_type_name(type_name: &str) -> Option<FieldType> {
    let lowered = type_name.to_ascii_lowercase();
    Some(match lowered.as_str() {
        "uuid" | "guid" => FieldType::Uuid,
        "email" | "emailaddress" => FieldType::Email,
        "url" | "uri" | "link" => FieldType::Url,
        "datetime" | "timestamp" | "isodatetime" => FieldType::Timestamp {
            format: TimestampFormat::Rfc3339Utc,
        },
        "date" | "isodate" => FieldType::IsoDate {
            format: DateFormat::Iso,
        },
        "phone" | "phonenumber" => FieldType::PhoneNumber,
        "ipaddress" | "ip" => FieldType::IpAddress,
        _ => return None,
    })
}

/// How a string field's *name* says its value should read.
///
/// A schema cannot distinguish a title from a status code — both are
/// `String` — but `collectionType`, `accessState` and `syncMode` all hold a
/// short token, and answering them with a lorem sentence is wrong in a way a
/// client switching on the value notices immediately.
#[must_use]
pub fn text_shape_of(field_name: &str) -> TextShape {
    const TOKEN_SUFFIXES: [&str; 9] = [
        "type", "kind", "status", "state", "mode", "level", "role", "stage", "phase",
    ];
    const SLUG_SUFFIXES: [&str; 4] = ["slug", "key", "code", "handle"];

    let lowered = field_name.to_ascii_lowercase().replace(['_', '-'], "");
    if SLUG_SUFFIXES.iter().any(|s| lowered.ends_with(s)) {
        return TextShape::Slug;
    }
    if TOKEN_SUFFIXES.iter().any(|s| lowered.ends_with(s)) {
        return TextShape::Word;
    }
    TextShape::Prose
}

/// Field-name conventions, ordered so the more specific match wins.
#[must_use]
pub fn from_field_name(field_name: &str) -> Option<FieldType> {
    let any = |patterns: &[&str]| patterns.iter().any(|p| matches_field_name(field_name, p));
    let ends = |suffixes: &[&str]| {
        let lowered = field_name.to_ascii_lowercase().replace(['_', '-'], "");
        suffixes.iter().any(|s| lowered.ends_with(s))
    };
    let contains = |needles: &[&str]| {
        let lowered = field_name.to_ascii_lowercase().replace(['_', '-'], "");
        needles.iter().any(|n| lowered.contains(n))
    };

    if any(&["id", "uuid", "guid"]) || ends(&["uuid", "guid"]) {
        return Some(FieldType::Uuid);
    }
    if contains(&["email"]) {
        return Some(FieldType::Email);
    }
    if any(&["username", "login", "handle", "nickname"]) {
        return Some(FieldType::Username);
    }
    if any(&["name", "fullname", "firstname", "lastname", "displayname"]) {
        return Some(FieldType::Name);
    }
    if contains(&["avatar", "thumbnail", "imageurl", "photourl", "picture"]) {
        return Some(FieldType::ImageUrl);
    }
    if ends(&["url", "uri", "href", "link"]) || any(&["website", "homepage"]) {
        return Some(FieldType::Url);
    }
    if any(&["phone", "mobile", "telephone", "tel"]) || ends(&["phone"]) {
        return Some(FieldType::PhoneNumber);
    }
    if ends(&["at"]) && contains(&["created", "updated", "deleted", "modified", "expires"]) {
        return Some(FieldType::Timestamp {
            format: TimestampFormat::Rfc3339Utc,
        });
    }
    if contains(&["timestamp"]) || ends(&["time"]) {
        return Some(FieldType::Timestamp {
            format: TimestampFormat::Rfc3339Utc,
        });
    }
    if ends(&["date"]) || any(&["birthday", "dob"]) {
        return Some(FieldType::IsoDate {
            format: DateFormat::Iso,
        });
    }
    if any(&["ip", "ipaddress", "clientip", "remoteaddr"]) {
        return Some(FieldType::IpAddress);
    }
    if any(&["filename", "file"]) || ends(&["filename"]) {
        return Some(FieldType::FileName);
    }
    if any(&["mimetype", "contenttype"]) {
        return Some(FieldType::MimeType);
    }
    if any(&["token", "accesstoken", "refreshtoken", "apikey"]) || ends(&["token"]) {
        return Some(FieldType::Token);
    }
    if any(&["etag"]) {
        return Some(FieldType::ETag);
    }
    if any(&["slug"]) {
        return Some(FieldType::Sentence);
    }
    if any(&[
        "description",
        "bio",
        "summary",
        "body",
        "content",
        "excerpt",
    ]) {
        return Some(FieldType::Paragraph);
    }
    if any(&["title", "subject", "headline", "label", "caption"]) {
        return Some(FieldType::Sentence);
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_format_wins_over_everything() {
        let detected = semantic_of("title", "String", Some("uuid")).unwrap();
        assert!(matches!(detected, FieldType::Uuid));
    }

    #[test]
    fn a_custom_scalar_name_beats_the_field_name() {
        let detected = semantic_of("cursor", "DateTime", None).unwrap();
        assert!(matches!(detected, FieldType::Timestamp { .. }));
    }

    #[test]
    fn field_names_are_matched_across_case_conventions() {
        for spelling in ["created_at", "createdAt", "CreatedAt", "CREATED_AT"] {
            let detected = semantic_of(spelling, "String", None);
            assert!(
                matches!(detected, Some(FieldType::Timestamp { .. })),
                "`{spelling}` should read as a timestamp"
            );
        }
    }

    #[test]
    fn common_conventions_are_recognised() {
        assert!(matches!(
            semantic_of("email", "String", None),
            Some(FieldType::Email)
        ));
        assert!(matches!(
            semantic_of("contactEmail", "String", None),
            Some(FieldType::Email)
        ));
        assert!(matches!(
            semantic_of("avatarUrl", "String", None),
            Some(FieldType::ImageUrl)
        ));
        assert!(matches!(
            semantic_of("homepageUrl", "String", None),
            Some(FieldType::Url)
        ));
        assert!(matches!(
            semantic_of("id", "ID", None),
            Some(FieldType::Uuid)
        ));
        assert!(matches!(
            semantic_of("description", "String", None),
            Some(FieldType::Paragraph)
        ));
    }

    #[test]
    fn token_shaped_names_are_recognised() {
        for name in [
            "collectionType",
            "accessState",
            "syncMode",
            "userRole",
            "log_level",
        ] {
            assert_eq!(
                text_shape_of(name),
                TextShape::Word,
                "`{name}` holds a token, not prose"
            );
        }
        for name in ["slug", "apiKey", "countryCode", "handle"] {
            assert_eq!(text_shape_of(name), TextShape::Slug, "`{name}`");
        }
        for name in ["title", "description", "name", "summary"] {
            assert_eq!(text_shape_of(name), TextShape::Prose, "`{name}`");
        }
    }

    #[test]
    fn an_unremarkable_field_stays_unremarkable() {
        assert!(semantic_of("colour", "String", None).is_none());
        assert!(semantic_of("weight", "Float", None).is_none());
    }

    #[test]
    fn an_unknown_format_falls_through_rather_than_guessing() {
        // `binary` has no faithful scalar rendering, so the declared kind wins.
        assert!(semantic_of("blob", "String", Some("binary")).is_none());
    }
}
