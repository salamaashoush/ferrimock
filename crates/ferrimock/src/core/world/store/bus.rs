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
type Pass = fn(&str, &Available) -> Option<JsonValue>;

const PASSES: [Pass; 3] = [composed_name, handle_from_name, link_from_key];

/// What a record holds, under the names the rules match on.
///
/// Built once per record rather than per rule. Every rule asks for a value by
/// one of a handful of normalised names, and normalising the record's own keys
/// inside each of those questions made the cost of a record quadratic in its
/// own width — on the request path, for every record materialised.
#[derive(Default)]
struct Available {
    text: rustc_hash::FxHashMap<String, String>,
}

impl Available {
    /// The first of several names the record actually carries text under.
    fn any_of(&self, wanted: &[&str]) -> Option<&str> {
        wanted
            .iter()
            .find_map(|name| self.text.get(*name))
            .map(String::as_str)
            .filter(|held| !held.is_empty())
    }

    fn under(&self, name: &str) -> Option<&str> {
        self.text.get(name).map(String::as_str)
    }
}

/// Rewrite every field of a record that is a function of another.
///
/// A field the caller stated is left alone: what a client wrote stands, even
/// where it disagrees with the rest of the record, because that is what the
/// client asked for. A field that is absent stays absent — deriving a value
/// for a key the record does not carry would put it back.
pub fn wire(fields: &[FieldDef], record: &mut JsonMap<String, JsonValue>, stated: &[String]) {
    // The common case is a record with nothing to derive and nothing to derive
    // from, and it should cost one scan of the field names. `might_matter`
    // rejects `id`, `size` and `created_at` without allocating anything.
    if !fields
        .iter()
        .any(|field| might_be_written(field.name.as_str()))
    {
        return;
    }

    let names: Vec<(String, String)> = record
        .keys()
        .filter(|name| might_matter(name))
        .map(|name| (name.clone(), normalise(name)))
        .collect();
    let mut available = Available::default();
    for (written, normalised) in &names {
        if let Some(text) = record.get(written).and_then(JsonValue::as_str) {
            available.text.insert(normalised.clone(), text.to_string());
        }
    }

    let mut settled: Vec<(&str, &str, String)> = Vec::new();
    for pass in PASSES {
        for (written, normalised) in &names {
            if !might_be_written(written) || stated.iter().any(|held| held == written) {
                continue;
            }
            if let Some(JsonValue::String(text)) = pass(normalised, &available) {
                settled.push((written, normalised, text));
            }
        }
        // A pass reads what the ones before it settled — a name written out of
        // its parts, before the handle derived from that name — so each pass's
        // results land before the next one looks. Cleared rather than consumed,
        // because the next pass writes into the same allocation.
        for (written, normalised, text) in &settled {
            available
                .text
                .insert((*normalised).to_string(), text.clone());
            record.insert((*written).to_string(), JsonValue::String(text.clone()));
        }
        settled.clear();
    }
}

/// Whether a field name could possibly be one a rule *writes*.
///
/// A byte scan, no allocation. Every rule below writes a name holding one of
/// these markers, and a record's other fields should not pay to find that out.
fn might_be_written(name: &str) -> bool {
    const WRITTEN: [&[u8]; 11] = [
        b"name", b"mail", b"user", b"slug", b"link", b"url", b"handle", b"login", b"initial",
        b"perma", b"screen",
    ];
    holds_any(name, &WRITTEN)
}

/// Whether a field name could take part in a derivation at all, as the thing
/// written or as something read to write it.
fn might_matter(name: &str) -> bool {
    const READ: [&[u8]; 10] = [
        b"title", b"head", b"subject", b"first", b"last", b"given", b"family", b"sur", b"id",
        b"uuid",
    ];
    might_be_written(name) || holds_any(name, &READ)
}

fn holds_any(name: &str, markers: &[&[u8]]) -> bool {
    let held = name.as_bytes();
    markers.iter().any(|marker| {
        held.len() >= marker.len()
            && held
                .windows(marker.len())
                .any(|window| window.eq_ignore_ascii_case(marker))
    })
}

/// A name written out of the parts beside it.
fn composed_name(name: &str, held: &Available) -> Option<JsonValue> {
    let first = held.any_of(&["firstname", "givenname", "forename"])?;
    let last = held.any_of(&["lastname", "familyname", "surname"])?;
    match name {
        "fullname" | "displayname" | "name" => Some(JsonValue::from(format!("{first} {last}"))),
        "initials" => {
            let letters: String = [first, last]
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
fn handle_from_name(name: &str, held: &Available) -> Option<JsonValue> {
    match name {
        "email" | "emailaddress" | "contactemail" => {
            let person = held.any_of(&["fullname", "displayname", "name", "username"])?;
            // The domain the generator already drew is kept: it carries the
            // locale mix, and only the local part is a function of the name.
            let domain = held
                .under(name)?
                .rsplit('@')
                .next()
                .filter(|domain| !domain.is_empty())?;
            Some(JsonValue::from(format!(
                "{}@{domain}",
                slugify(person, '.')
            )))
        }
        "username" | "handle" | "login" | "screenname" => {
            let person = held.any_of(&["fullname", "displayname", "name"])?;
            Some(JsonValue::from(slugify(person, '.')))
        }
        "slug" | "permalink" | "urlslug" => {
            let text = held.any_of(&["title", "headline", "name", "subject"])?;
            Some(JsonValue::from(slugify(text, '-')))
        }
        _ => None,
    }
}

/// A link that ends in the record's own key.
fn link_from_key(name: &str, held: &Available) -> Option<JsonValue> {
    const LINKED: [&str; 6] = [
        "avatarurl",
        "imageurl",
        "iconurl",
        "photourl",
        "thumbnailurl",
        "selfurl",
    ];
    if !LINKED.contains(&name) {
        return None;
    }
    let key = held.any_of(&["id", "uuid", "identifier"])?;
    let written = held.under(name)?;
    // Whatever the generator drew for the host and path stays; only the last
    // segment is a function of the record.
    let base = written.rsplit_once('/').map_or(written, |(base, _)| base);
    Some(JsonValue::from(format!("{base}/{key}")))
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
