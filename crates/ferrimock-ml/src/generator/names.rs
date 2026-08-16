//! Synthesising the name a field goes by.
//!
//! A name is the strongest single signal a classifier has and the least
//! reliable one, so the corpus has to contain both halves of that. Most names
//! here say what the field holds. Some say nothing -- real responses are full of
//! `value`, `data` and `v`. A few say something false, because a field called
//! `id` holding an email address is a thing that happens, and a model that has
//! never met one will trust the name over the values every time.
//!
//! The spelling is the family's, not this module's: the same word list rendered
//! as `created_at`, `createdAt`, `CreatedAt`, `created-at` and `CREATEDAT` is
//! four more conventions a model has to survive.

use super::dialect::{ApiDialect, NameStyle};
use super::rng::Rng;
use crate::label::FieldLabel;

/// A field name, and whether it gives the field away.
#[derive(Debug, Clone)]
pub struct GeneratedName {
    pub text: String,
    /// Whether the name says what the field holds. Read by value synthesis,
    /// which withholds irreducibly ambiguous spellings behind a vague name.
    pub informative: bool,
}

/// Names that carry no information at all. One field in six is given one.
const VAGUE: [&str; 14] = [
    "value", "data", "field", "attr", "v", "item", "val", "x", "obj", "payload", "content",
    "result", "entry", "raw",
];

/// The words a field of this label is usually named with.
///
/// Each entry is a word sequence rather than a spelling, so a family renders it
/// in its own convention.
#[allow(clippy::too_many_lines)] // One arm per label; splitting it only hides the table
fn vocabulary(label: FieldLabel) -> &'static [&'static [&'static str]] {
    match label {
        FieldLabel::Uuid => &[
            &["id"],
            &["uuid"],
            &["guid"],
            &["request", "id"],
            &["trace", "id"],
            &["correlation", "id"],
            &["session", "id"],
            &["external", "id"],
            &["resource", "id"],
            &["instance", "id"],
            &["batch", "id"],
            &["event", "id"],
        ],
        FieldLabel::Email => &[
            &["email"],
            &["email", "address"],
            &["user", "email"],
            &["contact", "email"],
            &["owner", "email"],
            &["login", "email"],
            &["reply", "to"],
            &["notification", "email"],
            &["billing", "email"],
            &["primary", "email"],
        ],
        FieldLabel::Url => &[
            &["url"],
            &["uri"],
            &["link"],
            &["href"],
            &["web", "url"],
            &["callback", "url"],
            &["redirect", "uri"],
            &["webhook", "url"],
            &["shared", "link"],
            &["api", "url"],
            &["self", "link"],
            &["next", "url"],
        ],
        FieldLabel::ImageUrl => &[
            &["avatar"],
            &["avatar", "url"],
            &["icon", "url"],
            &["thumbnail"],
            &["thumbnail", "url"],
            &["image"],
            &["image", "url"],
            &["picture"],
            &["photo", "url"],
            &["logo", "url"],
            &["preview", "image"],
            &["profile", "picture"],
        ],
        FieldLabel::IsoDate => &[
            &["date"],
            &["due", "date"],
            &["start", "date"],
            &["end", "date"],
            &["birth", "date"],
            &["effective", "date"],
            &["expiry", "date"],
            &["invoice", "date"],
            &["period", "start"],
            &["valid", "until"],
        ],
        FieldLabel::Timestamp => &[
            &["created", "at"],
            &["updated", "at"],
            &["modified", "at"],
            &["deleted", "at"],
            &["timestamp"],
            &["last", "seen"],
            &["published", "at"],
            &["completed", "at"],
            &["content", "created", "at"],
            &["trashed", "at"],
            &["last", "modified"],
            &["occurred", "at"],
        ],
        FieldLabel::UnixTimestamp => &[
            &["ts"],
            &["epoch"],
            &["created"],
            &["expires", "at"],
            &["issued", "at"],
            &["not", "before"],
            &["event", "time"],
            &["start", "time"],
            &["updated"],
            &["timestamp", "ms"],
        ],
        FieldLabel::PhoneNumber => &[
            &["phone"],
            &["phone", "number"],
            &["mobile"],
            &["tel"],
            &["telephone"],
            &["contact", "number"],
            &["work", "phone"],
            &["fax"],
            &["sms", "number"],
            &["msisdn"],
        ],
        FieldLabel::IpAddress => &[
            &["ip"],
            &["ip", "address"],
            &["client", "ip"],
            &["remote", "addr"],
            &["source", "ip"],
            &["host", "ip"],
            &["origin", "ip"],
            &["last", "login", "ip"],
        ],
        FieldLabel::Semver => &[
            &["version"],
            &["app", "version"],
            &["semver"],
            &["schema", "version"],
            &["sdk", "version"],
            &["api", "version"],
            &["client", "version"],
            &["min", "version"],
        ],
        FieldLabel::HexString => &[
            &["hash"],
            &["sha"],
            &["sha1"],
            &["sha256"],
            &["checksum"],
            &["digest"],
            &["fingerprint"],
            &["content", "hash"],
            &["signature"],
            &["commit"],
            &["object", "id"],
        ],
        FieldLabel::Base64 => &[
            &["payload"],
            &["blob"],
            &["encoded"],
            &["node", "id"],
            &["global", "id"],
            &["cursor"],
            &["data", "url"],
            &["body", "base64"],
            &["attachment"],
        ],
        FieldLabel::CountryCode => &[
            &["country"],
            &["country", "code"],
            &["region"],
            &["nationality"],
            &["billing", "country"],
            &["issuing", "country"],
            &["market"],
        ],
        FieldLabel::CurrencyCode => &[
            &["currency"],
            &["currency", "code"],
            &["settlement", "currency"],
            &["price", "currency"],
            &["payout", "currency"],
        ],
        FieldLabel::LocaleCode => &[
            &["locale"],
            &["language"],
            &["lang"],
            &["language", "code"],
            &["preferred", "locale"],
            &["ui", "language"],
        ],
        FieldLabel::Timezone => &[
            &["timezone"],
            &["tz"],
            &["time", "zone"],
            &["local", "timezone"],
            &["default", "timezone"],
        ],
        FieldLabel::PostalCode => &[
            &["zip"],
            &["zip", "code"],
            &["postal", "code"],
            &["postcode"],
            &["billing", "zip"],
            &["shipping", "postcode"],
        ],
        FieldLabel::MimeType => &[
            &["content", "type"],
            &["mime", "type"],
            &["type"],
            &["media", "type"],
            &["format"],
            &["file", "type"],
        ],
        FieldLabel::FileName => &[
            &["filename"],
            &["file", "name"],
            &["name"],
            &["original", "name"],
            &["display", "name"],
            &["attachment", "name"],
            &["document", "name"],
            &["basename"],
        ],
        FieldLabel::FilePath => &[
            &["path"],
            &["file", "path"],
            &["location"],
            &["full", "path"],
            &["directory"],
            &["folder", "path"],
            &["key"],
            &["object", "key"],
        ],
        FieldLabel::Username => &[
            &["username"],
            &["login"],
            &["handle"],
            &["account"],
            &["user", "name"],
            &["screen", "name"],
            &["slug"],
            &["nickname"],
        ],
        FieldLabel::PersonName => &[
            &["name"],
            &["full", "name"],
            &["display", "name"],
            &["owner"],
            &["created", "by"],
            &["author"],
            &["contact", "name"],
            &["assignee"],
            &["recipient"],
            &["signer", "name"],
        ],
        FieldLabel::Sentence => &[
            &["description"],
            &["summary"],
            &["message"],
            &["note"],
            &["comment"],
            &["title"],
            &["subject"],
            &["reason"],
            &["details"],
            &["body"],
            &["error", "message"],
            &["status", "text"],
        ],
        FieldLabel::NumericStringId => &[
            &["id"],
            &["file", "id"],
            &["object", "id"],
            &["parent", "id"],
            &["folder", "id"],
            &["user", "id"],
            &["account", "id"],
            &["order", "id"],
            &["message", "id"],
            &["item", "id"],
            &["external", "ref"],
        ],
        FieldLabel::Token => &[
            &["token"],
            &["access", "token"],
            &["refresh", "token"],
            &["api", "key"],
            &["secret"],
            &["session"],
            &["auth", "token"],
            &["client", "secret"],
            &["bearer"],
            &["signature"],
        ],
        FieldLabel::ETag => &[
            &["etag"],
            &["revision"],
            &["rev"],
            &["sequence", "id"],
            &["version", "tag"],
            &["change", "key"],
        ],
        FieldLabel::Boolean => &[
            &["enabled"],
            &["is", "active"],
            &["deleted"],
            &["has", "more"],
            &["is", "public"],
            &["can", "edit"],
            &["archived"],
            &["verified"],
            &["is", "default"],
            &["allow", "download"],
            &["livemode"],
            &["dry", "run"],
        ],
        FieldLabel::Number => &[
            &["count"],
            &["size"],
            &["total"],
            &["offset"],
            &["limit"],
            &["amount"],
            &["price"],
            &["quantity"],
            &["item", "count"],
            &["total", "count"],
            &["file", "size"],
            &["duration"],
            &["latitude"],
            &["score"],
        ],
        FieldLabel::Opaque => &[
            &["ref"],
            &["code"],
            &["marker"],
            &["cursor"],
            &["handle"],
            &["key"],
            &["reference"],
            &["identifier"],
            &["sid"],
            &["arn"],
            &["resource", "name"],
            &["shard"],
            &["partition", "key"],
            &["group"],
        ],
    }
}

/// Short spellings the same field turns up under.
///
/// A separate table rather than a truncation rule, because these are the
/// spellings services actually use, and a rule would invent ones nobody writes.
fn abbreviation(label: FieldLabel) -> Option<&'static [&'static str]> {
    match label {
        FieldLabel::Timestamp => Some(&["ctime", "mtime", "upd_dt", "crt_ts"]),
        FieldLabel::UnixTimestamp => Some(&["ts", "epoch_s", "exp", "iat", "nbf"]),
        FieldLabel::IsoDate => Some(&["dt", "dob", "eff_dt"]),
        FieldLabel::Number => Some(&["qty", "amt", "cnt", "num", "sz"]),
        FieldLabel::PersonName => Some(&["nm", "fname", "usr_nm"]),
        FieldLabel::PhoneNumber => Some(&["tel_no", "ph", "msisdn"]),
        FieldLabel::CountryCode => Some(&["cc", "ctry"]),
        FieldLabel::CurrencyCode => Some(&["ccy", "cur"]),
        FieldLabel::PostalCode => Some(&["pc", "zip_cd"]),
        FieldLabel::FileName => Some(&["fn", "fname"]),
        FieldLabel::Uuid | FieldLabel::NumericStringId => Some(&["oid", "rid", "pk"]),
        _ => None,
    }
}

/// Draw a name for a field of `label` in `dialect`.
pub fn draw(label: FieldLabel, dialect: ApiDialect, rng: &mut Rng) -> GeneratedName {
    // A vague name, an abbreviation, a misleading one, or the field's own words.
    // The misleading share is small and deliberate: it is what teaches a model
    // to read the values when the name disagrees with them.
    let (words, informative) = match rng.weighted(&[70, 16, 10, 4]) {
        0 => (vocabulary_words(label, rng), true),
        1 => return vague(dialect, rng),
        2 => match abbreviation(label) {
            Some(short) => (vec![rng.pick(short).to_string()], true),
            None => (vocabulary_words(label, rng), true),
        },
        _ => (vocabulary_words(misleading_label(label, rng), rng), false),
    };

    let mut parts: Vec<String> = words;
    if let Some(prefix) = affix(dialect.name_prefixes(), 4, rng) {
        parts.insert(0, prefix);
    }
    if let Some(suffix) = affix(dialect.name_suffixes(), 5, rng) {
        parts.push(suffix);
    }

    let borrowed: Vec<&str> = parts.iter().map(String::as_str).collect();
    let style = *rng
        .choose(dialect.name_styles())
        .unwrap_or(&NameStyle::Snake);
    let mut text = style.render(&borrowed);

    // The two disturbances that change what a name looks like without changing
    // what it means: a private-field marker, and a version counter.
    if rng.chance(1, 25) {
        text.insert(0, '_');
    }
    if rng.chance(1, 30) {
        text.push_str(&rng.digits(1));
    }

    GeneratedName { text, informative }
}

fn vocabulary_words(label: FieldLabel, rng: &mut Rng) -> Vec<String> {
    let table = vocabulary(label);
    rng.choose(table).map_or_else(
        || vec!["value".to_string()],
        |words| words.iter().map(|word| (*word).to_string()).collect(),
    )
}

fn vague(dialect: ApiDialect, rng: &mut Rng) -> GeneratedName {
    let word = rng.pick(&VAGUE).to_string();
    let style = *rng
        .choose(dialect.name_styles())
        .unwrap_or(&NameStyle::Snake);
    GeneratedName {
        text: style.render(&[word.as_str()]),
        informative: false,
    }
}

/// A label whose names would mislead about `label`.
///
/// Drawn rather than fixed, so the corpus does not teach one particular false
/// association -- `id` for an email, say -- as if it were the only one.
fn misleading_label(label: FieldLabel, rng: &mut Rng) -> FieldLabel {
    let mut candidate = label;
    for _ in 0..4 {
        let index = rng.below(FieldLabel::ALL.len());
        candidate = FieldLabel::from_class_index(index).unwrap_or(FieldLabel::Opaque);
        if candidate != label {
            return candidate;
        }
    }
    candidate
}

/// One of `options`, with probability one in `denominator`.
fn affix(options: &[&str], denominator: u32, rng: &mut Rng) -> Option<String> {
    if options.is_empty() || !rng.chance(1, denominator) {
        return None;
    }
    Some(rng.pick(options).to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rustc_hash::{FxHashMap, FxHashSet};

    fn names(label: FieldLabel, dialect: ApiDialect, count: u64) -> Vec<GeneratedName> {
        (0..count)
            .map(|seed| {
                let mut rng = Rng::for_index(1, seed);
                draw(label, dialect, &mut rng)
            })
            .collect()
    }

    #[test]
    fn every_label_has_names_to_draw_from() {
        for label in FieldLabel::ALL {
            let table = vocabulary(label);
            assert!(
                table.len() >= 5,
                "{} has only {} names",
                label.name(),
                table.len()
            );
            for words in table {
                assert!(!words.is_empty(), "{} has an empty name", label.name());
                for word in *words {
                    // Words, not spellings: a family renders them in its own
                    // convention, and a capital or a separator written here
                    // would survive into every family unchanged.
                    assert!(
                        word.chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                        "{} carries {word}, which is a spelling rather than a word",
                        label.name()
                    );
                }
            }
        }
    }

    #[test]
    fn a_name_is_spelled_the_way_its_family_spells_one() {
        // The same field in four families is four different strings, which is
        // most of what makes a model survive an API it has not seen.
        let spellings: FxHashSet<String> = [
            ApiDialect::ContentPlatform,
            ApiDialect::CloudResource,
            ApiDialect::CloudInfrastructure,
            ApiDialect::JsonApiService,
            ApiDialect::LegacyEnterprise,
        ]
        .into_iter()
        .flat_map(|dialect| {
            names(FieldLabel::Timestamp, dialect, 40)
                .into_iter()
                .map(|name| name.text)
        })
        .collect();

        assert!(
            spellings.iter().any(|name| name.contains('_')),
            "no snake_case name"
        );
        assert!(
            spellings
                .iter()
                .any(|name| name.chars().any(char::is_uppercase)),
            "no name with a capital in it"
        );
        assert!(spellings.len() > 20, "only {} spellings", spellings.len());
    }

    #[test]
    fn some_names_say_nothing_and_a_few_say_something_false() {
        // Both are what a real recording is full of, and both are what stops a
        // model from passing by reading the name alone.
        let drawn = names(FieldLabel::Email, ApiDialect::MixedLegacy, 600);
        let uninformative = drawn.iter().filter(|name| !name.informative).count();

        assert!(
            uninformative > 60,
            "only {uninformative} of 600 names withheld the answer"
        );
        assert!(
            uninformative < 300,
            "{uninformative} of 600 names withheld the answer, which is no longer a corpus \
             about names"
        );
    }

    #[test]
    fn a_misleading_name_is_marked_as_one_so_the_values_stay_unambiguous() {
        // A field named `is_enabled` may hold `1`; a field named misleadingly
        // must not, or the row is unreadable by anyone.
        let drawn = names(FieldLabel::Boolean, ApiDialect::MixedLegacy, 400);
        assert!(
            drawn.iter().any(|name| !name.informative),
            "no misleading or vague name was ever drawn"
        );
    }

    #[test]
    fn a_family_with_prefixes_uses_them() {
        let prefixed = names(FieldLabel::ETag, ApiDialect::ODataService, 200)
            .iter()
            .filter(|name| name.text.to_lowercase().contains("odata"))
            .count();
        assert!(prefixed > 0, "the family's own prefix never appeared");
    }

    #[test]
    fn a_name_is_never_empty_and_never_only_punctuation() {
        for dialect in ApiDialect::ALL {
            for label in FieldLabel::ALL {
                for name in names(label, dialect, 6) {
                    assert!(
                        !name.text.is_empty(),
                        "{} in {}",
                        label.name(),
                        dialect.name()
                    );
                    assert!(
                        name.text.chars().any(char::is_alphanumeric),
                        "{} produced {}",
                        label.name(),
                        name.text
                    );
                }
            }
        }
    }

    #[test]
    fn the_same_index_draws_the_same_name() {
        let mut first = Rng::for_index(9, 77);
        let mut second = Rng::for_index(9, 77);
        assert_eq!(
            draw(FieldLabel::Url, ApiDialect::CommercePlatform, &mut first).text,
            draw(FieldLabel::Url, ApiDialect::CommercePlatform, &mut second).text
        );
    }

    #[test]
    fn no_label_is_named_by_only_one_string() {
        let mut thin: FxHashMap<&str, usize> = FxHashMap::default();
        for label in FieldLabel::ALL {
            let distinct: FxHashSet<String> = names(label, ApiDialect::MixedLegacy, 200)
                .into_iter()
                .map(|name| name.text)
                .collect();
            thin.insert(label.name(), distinct.len());
        }
        for (label, count) in thin {
            assert!(count >= 20, "{label} was named only {count} ways");
        }
    }
}
