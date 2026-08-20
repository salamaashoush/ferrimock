//! Fields of one record that are functions of other fields of that record.
//!
//! The loudest within-record tells are not correlations. `email` holds a slug
//! of `name`; `full_name` *is* `first_name` plus `last_name`; `slug` *is*
//! `title` run through a slugifier; an avatar URL ends in the record's own id.
//! No latent vector produces any of them at any dimension, because they are
//! not mediated by a hidden variable — one field simply is a function of
//! another, and a record where the two disagree is wrong rather than
//! improbable.
//!
//! The primitive is a bus: build the record, then let dependent fields read
//! what is already there instead of the bytes they drew. `order_lifecycle` was
//! already this — a pass over values the record had drawn, dealing them back
//! out — which makes it the precedent.

use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::core::world::model::FieldDef;

/// What a record makes available, in the order it becomes available: a
/// person's name before a handle derived from it, a handle before a link that
/// embeds it.
type Pass = fn(&Held<'_>, &JsonMap<String, JsonValue>) -> Option<JsonValue>;

const PASSES: [Pass; 3] = [composed_name, handle_from_name, link_from_key];

/// One field of a record, under both the name it was written with and the
/// name a rule matches on.
struct Held<'a> {
    written: &'a str,
    normalised: String,
}

impl Held<'_> {
    fn text(&self, record: &JsonMap<String, JsonValue>) -> Option<String> {
        record.get(self.written)?.as_str().map(ToString::to_string)
    }
}

/// Rewrite every field of a record that is a function of another.
///
/// A field the caller stated is left alone: what a client wrote stands, even
/// where it disagrees with the rest of the record, because that is what the
/// client asked for. A field that is absent stays absent — deriving a value
/// for a key the record does not carry would put it back.
pub fn wire(fields: &[FieldDef], record: &mut JsonMap<String, JsonValue>, stated: &[String]) {
    for pass in PASSES {
        for field in fields {
            let name = field.name.as_str();
            if !record.contains_key(name) || stated.iter().any(|held| held == name) {
                continue;
            }
            let held = Held {
                written: name,
                normalised: normalise(name),
            };
            let Some(value) = pass(&held, record) else {
                continue;
            };
            record.insert(name.to_string(), value);
        }
    }
}

/// A name written out of the parts beside it.
fn composed_name(field: &Held<'_>, record: &JsonMap<String, JsonValue>) -> Option<JsonValue> {
    let first = text_of(record, &["firstname", "givenname", "forename"])?;
    let last = text_of(record, &["lastname", "familyname", "surname"])?;
    match field.normalised.as_str() {
        "fullname" | "displayname" | "name" => Some(JsonValue::from(format!("{first} {last}"))),
        "initials" => {
            let letters: String = [&first, &last]
                .into_iter()
                .filter_map(|part| part.chars().next())
                .flat_map(char::to_uppercase)
                .collect();
            (!letters.is_empty()).then(|| JsonValue::from(letters))
        }
        _ => None,
    }
}

/// A handle, an address or a slug, written out of the text it belongs to.
fn handle_from_name(field: &Held<'_>, record: &JsonMap<String, JsonValue>) -> Option<JsonValue> {
    match field.normalised.as_str() {
        "email" | "emailaddress" | "contactemail" => {
            let person = text_of(record, &["fullname", "displayname", "name", "username"])?;
            // The domain the generator already drew is kept: it carries the
            // locale mix, and only the local part is a function of the name.
            let held = field.text(record)?;
            let domain = held.rsplit('@').next().filter(|d| !d.is_empty())?;
            Some(JsonValue::from(format!(
                "{}@{domain}",
                slugify(&person, '.')
            )))
        }
        "username" | "handle" | "login" | "screenname" => {
            let person = text_of(record, &["fullname", "displayname", "name"])?;
            Some(JsonValue::from(slugify(&person, '.')))
        }
        "slug" | "permalink" | "urlslug" => {
            let text = text_of(record, &["title", "headline", "name", "subject"])?;
            Some(JsonValue::from(slugify(&text, '-')))
        }
        _ => None,
    }
}

/// A link that ends in the record's own key.
fn link_from_key(field: &Held<'_>, record: &JsonMap<String, JsonValue>) -> Option<JsonValue> {
    const LINKED: [&str; 6] = [
        "avatarurl",
        "imageurl",
        "iconurl",
        "photourl",
        "thumbnailurl",
        "selfurl",
    ];
    if !LINKED.contains(&field.normalised.as_str()) {
        return None;
    }
    let key = text_of(record, &["id", "uuid", "identifier"])?;
    let held = field.text(record)?;
    // Whatever the generator drew for the host and path stays; only the last
    // segment is a function of the record.
    let base = held
        .rsplit_once('/')
        .map_or(held.as_str(), |(base, _)| base);
    Some(JsonValue::from(format!("{base}/{key}")))
}

/// The first of several field names the record actually carries text under.
fn text_of(record: &JsonMap<String, JsonValue>, wanted: &[&str]) -> Option<String> {
    record.iter().find_map(|(name, value)| {
        let text = value.as_str()?;
        (!text.is_empty() && wanted.contains(&normalise(name).as_str())).then(|| text.to_string())
    })
}

fn normalise(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Text as it would be written into a URL or an address.
fn slugify(text: &str, separator: char) -> String {
    let mut written = String::with_capacity(text.len());
    let mut pending = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            if pending && !written.is_empty() {
                written.push(separator);
            }
            pending = false;
            written.extend(ch.to_lowercase());
        } else {
            pending = true;
        }
    }
    written
}

#[cfg(test)]
mod tests;
