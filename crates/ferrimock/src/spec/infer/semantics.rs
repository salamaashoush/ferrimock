//! What a field means, when all you have is its name and declared type.
//!
//! [`crate::type_detector::semantic::detect_from_semantic_context`] is built
//! for recordings: most of its rules
//! confirm a guess about the name against the values that were actually seen,
//! so with no samples they cannot fire. A spec has no samples — it has
//! something the recording path never gets instead, a declared type name and a
//! declared format. This maps those two.
//!
//! Case conventions are handled by the detector's own matcher, so `createdAt`,
//! `created_at` and `CREATED-AT` all land in the same place.

use crate::core::world::model::TextShape;
use crate::type_detector::semantic::{matches_any_field_name, matches_field_name};
use crate::type_detector::{DateFormat, FieldType, TimestampFormat};

/// Infer a field's meaning from its name and, failing that, from the name of
/// the type the spec gave it.
///
/// `owner` is the entity the field belongs to, which is the only thing that can
/// settle a bare `name`: on a `User` it is a person's name, on a `Folder` it is
/// a folder's, and on a `File` it is a filename. Answering all three with
/// `Cloyd Oberbrunner` is the kind of wrong a screenshot shows immediately.
#[must_use]
pub fn semantic_of(
    field_name: &str,
    type_name: &str,
    format: Option<&str>,
    owner: &str,
    examples: &[serde_json::Value],
) -> Option<FieldType> {
    // A declared format is the spec stating the answer, so it wins.
    if let Some(format) = format
        && let Some(field_type) = from_format(format)
    {
        return Some(field_type);
    }
    // Then a value the document itself wrote. It is the only evidence in a
    // spec that is not an inference — `example: "usr_01H8XG..."` says what an
    // id family is, and nothing in the word `id` could.
    if let Some(field_type) = from_examples(field_name, examples) {
        return Some(field_type);
    }
    if let Some(field_type) = from_type_name(type_name) {
        return Some(field_type);
    }
    if let Some(field_type) = from_owned_name(field_name, owner) {
        return Some(field_type);
    }
    from_field_name(field_name)
}

/// What the values a document wrote for a field say the field holds.
///
/// Read through the same detector the recording lane uses, so a spec and a
/// recording agree about what `2024-03-17T09:41:22Z` is. A weak reading is
/// discarded: an example that is just a word says nothing a field name does
/// not already say better.
fn from_examples(field_name: &str, examples: &[serde_json::Value]) -> Option<FieldType> {
    const CONVINCING: f64 = 0.8;

    if examples.is_empty() {
        return None;
    }
    let values: Vec<&serde_json::Value> = examples.iter().collect();
    let (field_type, confidence) =
        crate::type_detector::TypeDetector::new().detect_type(field_name, &values);
    (confidence >= CONVINCING
        && !matches!(field_type, FieldType::RandomString | FieldType::Constant(_)))
    .then_some(field_type)
}

/// What a bare `name` or `title` means, given what owns it.
///
/// Only the ambiguous spellings are decided here — `first_name` is a person's
/// wherever it appears, and [`from_field_name`] still answers for it.
fn from_owned_name(field_name: &str, owner: &str) -> Option<FieldType> {
    const PEOPLE: [&str; 12] = [
        "user",
        "person",
        "author",
        "customer",
        "contact",
        "member",
        "owner",
        "employee",
        "profile",
        "recipient",
        "sender",
        "assignee",
    ];
    const DOCUMENTS: [&str; 8] = [
        "file",
        "document",
        "attachment",
        "asset",
        "image",
        "photo",
        "upload",
        "media",
    ];

    let bare = matches_any_field_name(field_name, &["name", "title", "label"]);
    if !bare {
        return None;
    }

    let owner = owner.to_ascii_lowercase().replace(['_', '-', '.'], "");
    if PEOPLE.iter().any(|kind| owner.contains(kind)) {
        return Some(FieldType::Name);
    }
    if DOCUMENTS.iter().any(|kind| owner.contains(kind)) {
        return Some(FieldType::FileName);
    }
    // Anything else is named the way things are named, not the way people are:
    // left to the text shape, which composes a noun phrase.
    Some(FieldType::Sentence)
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
    // `name` on its own is decided by `from_owned_name`, which knows what owns
    // it; these spellings are a person's name wherever they appear.
    if any(&[
        "fullname",
        "firstname",
        "lastname",
        "displayname",
        "givenname",
        "surname",
    ]) {
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
    // A slug is left to `text_shape_of`, which spells it as one. Claiming a
    // semantic here would win over the shape and answer with prose.
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

    /// The two lanes share a vocabulary, not a rule set — and where both of
    /// them answer they must agree.
    ///
    /// The recording lane confirms a guess about a name against the values it
    /// saw; a spec has no values, so this one decides from the name, the
    /// declared type and the format alone. Neither is a subset of the other.
    /// But a `User` recorded and a `User` declared are the same `User`, so a
    /// field they both have an opinion about had better get the same one.
    #[test]
    fn the_two_lanes_agree_wherever_both_of_them_answer() {
        use crate::type_detector::{DetectionContext, detect_from_semantic_context};

        let recording = |name: &str| {
            detect_from_semantic_context(name, &[], &DetectionContext::builtin()).map(|(t, _)| t)
        };

        for name in [
            "first_name",
            "description",
            "summary",
            "avatar_url",
            "email",
            "username",
            "created_at",
            "phone",
            "filename",
            "etag",
            "slug",
        ] {
            let (Some(spec), Some(recorded)) = (
                semantic_of(name, "String", None, "Thing", &[]),
                recording(name),
            ) else {
                continue;
            };
            assert_eq!(
                std::mem::discriminant(&spec),
                std::mem::discriminant(&recorded),
                "`{name}`: spec says {spec:?}, recording says {recorded:?}"
            );
        }

        // `name` is the deliberate exception: this lane knows what owns the
        // field and the recording lane does not, so `Folder.name` is a folder's
        // name here. Owned by a person, the two agree again.
        assert!(matches!(
            semantic_of("name", "String", None, "User", &[]),
            Some(FieldType::Name)
        ));
        assert!(matches!(
            semantic_of("name", "String", None, "Folder", &[]),
            Some(FieldType::Sentence)
        ));
    }

    #[test]
    fn a_declared_format_wins_over_everything() {
        let detected = semantic_of("title", "String", Some("uuid"), "Thing", &[]).unwrap();
        assert!(matches!(detected, FieldType::Uuid));
    }

    #[test]
    fn a_custom_scalar_name_beats_the_field_name() {
        let detected = semantic_of("cursor", "DateTime", None, "Thing", &[]).unwrap();
        assert!(matches!(detected, FieldType::Timestamp { .. }));
    }

    #[test]
    fn field_names_are_matched_across_case_conventions() {
        for spelling in ["created_at", "createdAt", "CreatedAt", "CREATED_AT"] {
            let detected = semantic_of(spelling, "String", None, "Thing", &[]);
            assert!(
                matches!(detected, Some(FieldType::Timestamp { .. })),
                "`{spelling}` should read as a timestamp"
            );
        }
    }

    #[test]
    fn common_conventions_are_recognised() {
        assert!(matches!(
            semantic_of("email", "String", None, "Thing", &[]),
            Some(FieldType::Email)
        ));
        assert!(matches!(
            semantic_of("contactEmail", "String", None, "Thing", &[]),
            Some(FieldType::Email)
        ));
        assert!(matches!(
            semantic_of("avatarUrl", "String", None, "Thing", &[]),
            Some(FieldType::ImageUrl)
        ));
        assert!(matches!(
            semantic_of("homepageUrl", "String", None, "Thing", &[]),
            Some(FieldType::Url)
        ));
        assert!(matches!(
            semantic_of("id", "ID", None, "Thing", &[]),
            Some(FieldType::Uuid)
        ));
        assert!(matches!(
            semantic_of("description", "String", None, "Thing", &[]),
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
    fn a_slug_is_left_to_its_shape_rather_than_typed_as_prose() {
        assert!(
            semantic_of("slug", "String", None, "Thing", &[]).is_none(),
            "claiming a semantic here wins over the shape and answers with a sentence"
        );
        assert_eq!(text_shape_of("slug"), TextShape::Slug);
    }

    #[test]
    fn what_owns_a_bare_name_decides_what_it_holds() {
        assert!(
            matches!(
                semantic_of("name", "String", None, "User", &[]),
                Some(FieldType::Name)
            ),
            "a user's name is a person's"
        );
        assert!(
            matches!(
                semantic_of("name", "String", None, "File", &[]),
                Some(FieldType::FileName)
            ),
            "a file's name is a filename"
        );
        assert!(
            matches!(
                semantic_of("name", "String", None, "Folder", &[]),
                Some(FieldType::Sentence)
            ),
            "a folder is not called Cloyd Oberbrunner"
        );
    }

    #[test]
    fn an_explicit_person_name_is_one_whatever_owns_it() {
        for spelling in ["first_name", "lastName", "full_name", "displayName"] {
            assert!(
                matches!(
                    semantic_of(spelling, "String", None, "Folder", &[]),
                    Some(FieldType::Name)
                ),
                "`{spelling}` names a person wherever it appears"
            );
        }
    }

    #[test]
    fn an_unremarkable_field_stays_unremarkable() {
        assert!(semantic_of("colour", "String", None, "Thing", &[]).is_none());
        assert!(semantic_of("weight", "Float", None, "Thing", &[]).is_none());
    }

    #[test]
    fn an_unknown_format_falls_through_rather_than_guessing() {
        // `binary` has no faithful scalar rendering, so the declared kind wins.
        assert!(semantic_of("blob", "String", Some("binary"), "Thing", &[]).is_none());
    }
}
