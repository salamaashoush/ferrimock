//! Synthesising a labelled corpus.
//!
//! Every example here is labelled by the thing that made it. The generator
//! decided to emit an email address, so the example is an email address -- the
//! label is not an opinion about the value, it is a record of the value's
//! origin.
//!
//! That is the difference from the earlier attempt, which asked the built-in
//! detector to label synthetic data and then measured the resulting model
//! against the detector. A student cannot outscore the teacher that graded the
//! exam, and the scores said so without anyone noticing what they meant.
//!
//! Synthetic data has a real limit: it can only contain what someone thought to
//! generate, and a model trained on it learns the generator as much as the
//! domain. It is the floor, not the corpus. [`crate::corpus::Provenance`] marks
//! which examples came from here so a measurement over real, reviewed traffic
//! can be reported separately.

use crate::corpus::{Corpus, Example, Provenance};
use crate::label::FieldLabel;

/// Deterministic value source. Seeded rather than random so a corpus can be
/// regenerated exactly, which is what makes two training runs comparable.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }

    /// Pick an option, falling back to `fallback` only if a table is empty --
    /// which would be a bug in the table, not in the draw.
    fn choose<'a>(&mut self, options: &'a [&'a str]) -> &'a str {
        let index = self.below(options.len());
        options.get(index).copied().unwrap_or("")
    }

    fn hex(&mut self, length: usize) -> String {
        const DIGITS: &[u8] = b"0123456789abcdef";
        (0..length)
            .map(|_| {
                let index = (self.next() % DIGITS.len() as u64) as usize;
                char::from(DIGITS.get(index).copied().unwrap_or(b'0'))
            })
            .collect()
    }

    fn digits(&mut self, length: usize) -> String {
        (0..length)
            .map(|_| char::from(b'0' + (self.next() % 10) as u8))
            .collect()
    }

    fn alnum(&mut self, length: usize) -> String {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        (0..length)
            .map(|_| {
                let index = (self.next() % ALPHABET.len() as u64) as usize;
                char::from(ALPHABET.get(index).copied().unwrap_or(b'a'))
            })
            .collect()
    }
}

const WORDS: [&str; 12] = [
    "alpha", "bravo", "delta", "echo", "kilo", "lima", "mike", "nova", "oscar", "romeo", "sierra",
    "tango",
];
const FIRST_NAMES: [&str; 8] = [
    "Ada", "Grace", "Alan", "Edsger", "Barbara", "Ken", "Radia", "Leslie",
];
const LAST_NAMES: [&str; 8] = [
    "Lovelace", "Hopper", "Turing", "Dijkstra", "Liskov", "Thompson", "Perlman", "Lamport",
];

/// Build a corpus with `per_label` examples of each label.
///
/// Field names are drawn from several plausible spellings per label, including
/// some that say nothing (`value`, `data`), so a model cannot pass by reading
/// the name alone. Real recordings are full of fields named `value`.
fn generate(per_label: usize, seed: u64) -> Vec<Example> {
    let mut rng = Rng(seed | 1);
    let mut examples = Vec::with_capacity(per_label * FieldLabel::ALL.len());

    for label in FieldLabel::ALL {
        for _ in 0..per_label {
            let sample_count = 1 + rng.below(6);
            let values: Vec<String> = (0..sample_count).map(|_| value_for(label, &mut rng)).collect();
            let name = name_for(label, &mut rng);
            examples.push(Example::new(name, values, label, Provenance::Generated));
        }
    }

    examples
}

fn name_for(label: FieldLabel, rng: &mut Rng) -> String {
    // One name in five says nothing useful, so the model cannot lean entirely on
    // the field name -- which is exactly what real traffic does to it.
    let uninformative = ["value", "data", "field", "attr", "v"];
    if rng.below(5) == 0 {
        return (rng.choose(&uninformative)).to_string();
    }

    let options: &[&str] = match label {
        FieldLabel::Uuid => &["id", "uuid", "guid", "request_id", "traceId"],
        FieldLabel::Email => &["email", "user_email", "contact", "emailAddress"],
        FieldLabel::Url => &["url", "link", "href", "callback_url"],
        FieldLabel::ImageUrl => &["avatar", "icon_url", "thumbnail", "image"],
        FieldLabel::IsoDate => &["date", "due_date", "birthday", "startDate"],
        FieldLabel::Timestamp => &["created_at", "updated_at", "timestamp", "modifiedAt"],
        FieldLabel::UnixTimestamp => &["ts", "epoch", "created", "expires_at"],
        FieldLabel::PhoneNumber => &["phone", "mobile", "tel", "phoneNumber"],
        FieldLabel::IpAddress => &["ip", "client_ip", "remote_addr", "ipAddress"],
        FieldLabel::Semver => &["version", "app_version", "semver"],
        FieldLabel::HexString => &["hash", "sha1", "checksum", "digest"],
        FieldLabel::Base64 => &["payload", "blob", "encoded", "content"],
        FieldLabel::CountryCode => &["country", "country_code", "region"],
        FieldLabel::CurrencyCode => &["currency", "currency_code"],
        FieldLabel::LocaleCode => &["locale", "language", "lang"],
        FieldLabel::Timezone => &["timezone", "tz", "time_zone"],
        FieldLabel::PostalCode => &["zip", "postal_code", "postcode"],
        FieldLabel::MimeType => &["content_type", "mime_type", "type"],
        FieldLabel::FileName => &["filename", "name", "file"],
        FieldLabel::FilePath => &["path", "location", "file_path"],
        FieldLabel::Username => &["username", "login", "handle", "account"],
        FieldLabel::PersonName => &["name", "full_name", "display_name", "owner"],
        FieldLabel::Sentence => &["description", "summary", "message", "note"],
        FieldLabel::NumericStringId => &["id", "file_id", "object_id", "parentId"],
        FieldLabel::Token => &["token", "access_token", "api_key", "session"],
        FieldLabel::ETag => &["etag", "revision", "_rev"],
        FieldLabel::Boolean => &["enabled", "is_active", "deleted", "hasMore"],
        FieldLabel::Number => &["count", "size", "total", "offset"],
        FieldLabel::Opaque => &["ref", "code", "marker", "cursor"],
    };
    rng.choose(options).to_string()
}

#[allow(clippy::too_many_lines)] // One arm per label; splitting it would only hide the table
fn value_for(label: FieldLabel, rng: &mut Rng) -> String {
    match label {
        FieldLabel::Uuid => format!(
            "{}-{}-4{}-{}{}-{}",
            rng.hex(8),
            rng.hex(4),
            rng.hex(3),
            rng.choose(&["8", "9", "a", "b"]),
            rng.hex(3),
            rng.hex(12)
        ),
        FieldLabel::Email => format!(
            "{}.{}@{}.{}",
            rng.choose(&WORDS),
            rng.choose(&WORDS),
            rng.choose(&["example", "test", "mail"]),
            rng.choose(&["com", "org", "net"])
        ),
        FieldLabel::Url => format!(
            "https://{}.example.com/{}/{}",
            rng.choose(&WORDS),
            rng.choose(&WORDS),
            rng.digits(3)
        ),
        FieldLabel::ImageUrl => format!(
            "https://cdn.example.com/{}/{}.{}",
            rng.choose(&WORDS),
            rng.hex(8),
            rng.choose(&["png", "jpg", "webp", "svg"])
        ),
        FieldLabel::IsoDate => format!(
            "20{:02}-{:02}-{:02}",
            rng.below(30),
            1 + rng.below(12),
            1 + rng.below(28)
        ),
        FieldLabel::Timestamp => format!(
            "20{:02}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            rng.below(30),
            1 + rng.below(12),
            1 + rng.below(28),
            rng.below(24),
            rng.below(60),
            rng.below(60)
        ),
        FieldLabel::UnixTimestamp => (1_600_000_000 + rng.below(90_000_000)).to_string(),
        FieldLabel::PhoneNumber => format!(
            "+{} {} {}",
            1 + rng.below(60),
            rng.digits(3),
            rng.digits(7)
        ),
        FieldLabel::IpAddress => format!(
            "{}.{}.{}.{}",
            rng.below(256),
            rng.below(256),
            rng.below(256),
            rng.below(256)
        ),
        FieldLabel::Semver => format!(
            "{}.{}.{}",
            rng.below(10),
            rng.below(30),
            rng.below(20)
        ),
        FieldLabel::HexString => rng.hex(40),
        FieldLabel::Base64 => {
            let length = 20 + rng.below(20);
            format!("{}==", rng.alnum(length))
        }
        FieldLabel::CountryCode => (rng.choose(&[
            "US", "GB", "DE", "FR", "JP", "BR", "IN", "AU", "CA", "NL",
        ]))
        .to_string(),
        FieldLabel::CurrencyCode => (rng.choose(&[
            "USD", "EUR", "GBP", "JPY", "CHF", "AUD", "CAD", "SEK",
        ]))
        .to_string(),
        FieldLabel::LocaleCode => (rng.choose(&[
            "en-US", "en-GB", "de-DE", "fr-FR", "ja-JP", "pt-BR", "es-ES",
        ]))
        .to_string(),
        FieldLabel::Timezone => (rng.choose(&[
            "America/New_York",
            "Europe/London",
            "Europe/Berlin",
            "Asia/Tokyo",
            "Australia/Sydney",
        ]))
        .to_string(),
        FieldLabel::PostalCode => {
            if rng.below(2) == 0 {
                rng.digits(5)
            } else {
                let outward = rng.alnum(3).to_uppercase();
                let inward = rng.alnum(3).to_uppercase();
                format!("{outward} {inward}")
            }
        }
        FieldLabel::MimeType => (rng.choose(&[
            "application/json",
            "text/html",
            "image/png",
            "application/pdf",
            "text/plain",
        ]))
        .to_string(),
        FieldLabel::FileName => format!(
            "{}_{}.{}",
            rng.choose(&WORDS),
            rng.digits(3),
            rng.choose(&["pdf", "docx", "png", "csv", "zip"])
        ),
        FieldLabel::FilePath => format!(
            "/{}/{}/{}.{}",
            rng.choose(&WORDS),
            rng.choose(&WORDS),
            rng.choose(&WORDS),
            rng.choose(&["txt", "json", "yaml"])
        ),
        FieldLabel::Username => format!("{}{}", rng.choose(&WORDS), rng.digits(2)),
        FieldLabel::PersonName => format!("{} {}", rng.choose(&FIRST_NAMES), rng.choose(&LAST_NAMES)),
        FieldLabel::Sentence => format!(
            "The {} {} was {} by the {}.",
            rng.choose(&WORDS),
            rng.choose(&WORDS),
            rng.choose(&["updated", "removed", "shared", "renamed"]),
            rng.choose(&WORDS)
        ),
        FieldLabel::NumericStringId => {
            let length = 11 + rng.below(3);
            rng.digits(length)
        }
        FieldLabel::Token => format!(
            "{}.{}.{}",
            rng.alnum(16),
            rng.alnum(32),
            rng.alnum(24)
        ),
        FieldLabel::ETag => {
            let length = 1 + rng.below(2);
            rng.digits(length)
        }
        FieldLabel::Boolean => (rng.choose(&["true", "false"])).to_string(),
        FieldLabel::Number => rng.below(100_000).to_string(),
        // The residual, and the only class that must not look like anything:
        // short opaque handles with no structure to find.
        FieldLabel::Opaque => {
            let length = 4 + rng.below(8);
            rng.alnum(length)
        }
    }
}

/// Wrap generated examples into a corpus.
pub fn generate_corpus(per_label: usize, seed: u64) -> Corpus {
    Corpus::new(generate(per_label, seed))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_label_is_represented_equally() {
        let corpus = generate_corpus(20, 1);
        let counts = corpus.label_counts();

        assert_eq!(counts.len(), FieldLabel::ALL.len());
        assert!(counts.values().all(|count| *count == 20));
    }

    #[test]
    fn the_same_seed_gives_the_same_corpus() {
        let first = generate_corpus(5, 42);
        let second = generate_corpus(5, 42);

        let render = |c: &Corpus| -> Vec<String> {
            c.examples
                .iter()
                .map(|e| format!("{}={}", e.field_name, e.values.join(",")))
                .collect()
        };
        assert_eq!(render(&first), render(&second));
    }

    #[test]
    fn a_different_seed_gives_different_values() {
        let first = generate_corpus(5, 1);
        let second = generate_corpus(5, 2);
        let values = |c: &Corpus| -> Vec<String> {
            c.examples.iter().flat_map(|e| e.values.clone()).collect()
        };
        assert_ne!(values(&first), values(&second));
    }

    #[test]
    fn some_fields_are_named_uninformatively() {
        // Otherwise the corpus teaches a model that the name always gives it
        // away, which real traffic will immediately disprove.
        let corpus = generate_corpus(40, 3);
        let vague = corpus
            .examples
            .iter()
            .filter(|e| ["value", "data", "field", "attr", "v"].contains(&e.field_name.as_str()))
            .count();

        assert!(
            vague > 0,
            "no example forces the model to look at the values"
        );
    }

    #[test]
    fn generated_values_look_like_what_they_claim() {
        // Spot-check the shapes a downstream feature extractor keys on. This is
        // not the detector grading the generator -- it is the generator being
        // held to its own contract.
        let corpus = generate_corpus(30, 9);
        for example in &corpus.examples {
            for value in &example.values {
                match example.label {
                    FieldLabel::Email => assert!(value.contains('@'), "{value}"),
                    FieldLabel::Url | FieldLabel::ImageUrl => {
                        assert!(value.starts_with("https://"), "{value}");
                    }
                    FieldLabel::Uuid => assert_eq!(value.len(), 36, "{value}"),
                    FieldLabel::Boolean => {
                        assert!(value == "true" || value == "false", "{value}");
                    }
                    FieldLabel::IpAddress => {
                        assert_eq!(value.split('.').count(), 4, "{value}");
                    }
                    _ => assert!(!value.is_empty(), "{:?} produced nothing", example.label),
                }
            }
        }
    }

    #[test]
    fn a_field_carries_more_than_one_sample_sometimes() {
        let corpus = generate_corpus(30, 11);
        assert!(
            corpus.examples.iter().any(|e| e.values.len() > 1),
            "multi-sample fields are where agreement features mean anything"
        );
    }
}
