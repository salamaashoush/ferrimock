//! Text and content generators

use super::rng::rng;
use fake::Fake;
use fake::faker::lorem::en::*;

/// Generate random words (count specified)
pub fn fake_words(count: usize) -> String {
    let words: Vec<String> = Words(count..count + 1).fake_with_rng(&mut rng());
    words.join(" ")
}

/// Generate a random sentence with specified word count (default: 5)
pub fn fake_sentence(word_count: usize) -> String {
    let count = word_count.max(1);
    let words: Vec<String> = Words(count..count + 1).fake_with_rng(&mut rng());
    words.join(" ")
}

/// Generate a random paragraph with specified sentence count (default: 3)
pub fn fake_paragraph(sentence_count: usize) -> String {
    let count = sentence_count.max(1);
    let paragraph: Vec<String> = Sentences(count..count + 1).fake_with_rng(&mut rng());
    paragraph.join(" ")
}

/// Generate a random word
pub fn fake_word() -> String {
    Word().fake_with_rng(&mut rng())
}

/// Tokens a field whose name ends in `status`, `type`, `role` or `level`
/// actually holds.
///
/// Answering a `status` with `perferendis` is not a distribution problem: the
/// value does not mean what the field name says, and a client switching on it
/// breaks on the first record. The field name is read again here rather than
/// carried on the shape, because the shape says *that* the value is a short
/// token and this says *which* closed set it comes from — a `role` and a
/// `status` are both tokens and share no vocabulary at all.
#[must_use]
pub fn token_vocabulary(field_name: &str) -> &'static [&'static str] {
    const LIFECYCLE: [&str; 16] = [
        "active",
        "inactive",
        "pending",
        "draft",
        "published",
        "archived",
        "deleted",
        "expired",
        "approved",
        "rejected",
        "queued",
        "running",
        "completed",
        "failed",
        "cancelled",
        "suspended",
    ];
    const CATEGORY: [&str; 12] = [
        "standard",
        "custom",
        "internal",
        "external",
        "default",
        "manual",
        "automatic",
        "primary",
        "secondary",
        "shared",
        "private",
        "public",
    ];
    const ROLES: [&str; 8] = [
        "owner",
        "admin",
        "editor",
        "viewer",
        "member",
        "guest",
        "contributor",
        "reviewer",
    ];
    const LEVELS: [&str; 8] = [
        "low", "medium", "high", "critical", "info", "warning", "error", "debug",
    ];

    let leaf = field_name.rsplit('.').next().unwrap_or(field_name);
    let lowered = leaf.to_ascii_lowercase().replace(['_', '-'], "");
    if lowered.ends_with("role") {
        return &ROLES;
    }
    if lowered.ends_with("level") {
        return &LEVELS;
    }
    if ["type", "kind", "mode"]
        .iter()
        .any(|s| lowered.ends_with(s))
    {
        return &CATEGORY;
    }
    &LIFECYCLE
}

/// The stems a generated slug is built from.
///
/// Real words, because a slug reaches a URL and a person reads it there.
/// `perferendis-non-adipisci` is the thing that makes a mocked screen look
/// mocked.
#[must_use]
pub fn slug_stems() -> &'static [&'static str] {
    const STEMS: [&str; 48] = [
        "north",
        "summit",
        "harbor",
        "atlas",
        "beacon",
        "cobalt",
        "delta",
        "ember",
        "forge",
        "granite",
        "haven",
        "ironwood",
        "juniper",
        "kestrel",
        "lantern",
        "meridian",
        "nimbus",
        "orchard",
        "pioneer",
        "quarry",
        "ridgeline",
        "sable",
        "tundra",
        "umber",
        "vantage",
        "wayfarer",
        "yardarm",
        "zephyr",
        "anchor",
        "bridge",
        "canyon",
        "drift",
        "estuary",
        "foundry",
        "glacier",
        "hollow",
        "inlet",
        "junction",
        "keystone",
        "lattice",
        "mesa",
        "northgate",
        "outpost",
        "pinnacle",
        "quartz",
        "reef",
        "spire",
        "trailhead",
    ];
    &STEMS
}

/// Generate a slug (URL-friendly string)
pub fn fake_slug() -> String {
    use rand::RngExt as _;
    use rand::seq::IndexedRandom;

    let stems = slug_stems();
    let mut source = rng();
    let parts = source.random_range(2..=3);
    // Without replacement: `north-north` is not a slug anyone wrote.
    let written: Vec<&str> = stems.sample(&mut source, parts).copied().collect();
    let joined = written.join("-");
    // A real slug collides, and a real service disambiguates it.
    if source.random_range(0..4) == 0 {
        return format!("{joined}-{}", source.random_range(2..99));
    }
    joined
}

/// Generate a random alphanumeric string of specified length
/// Useful for codes, references, and other unknown string patterns
pub fn fake_alphanumeric(length: usize) -> String {
    use rand::seq::IndexedRandom;
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rng();

    (0..length)
        .map(|_| *CHARSET.choose(&mut rng).unwrap_or(&b'a') as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_words() {
        let words = fake_words(5);
        assert!(!words.is_empty());
        assert_eq!(words.split_whitespace().count(), 5);
    }

    #[test]
    fn test_fake_sentence() {
        let sentence = fake_sentence(5);
        assert!(!sentence.is_empty());
    }

    #[test]
    fn test_fake_paragraph() {
        let paragraph = fake_paragraph(3);
        assert!(!paragraph.is_empty());
    }

    #[test]
    fn test_fake_word() {
        let word = fake_word();
        assert!(!word.is_empty());
    }

    #[test]
    fn test_fake_slug() {
        let slug = fake_slug();
        assert!(slug.contains('-'));
        assert_eq!(slug, slug.to_lowercase());
        assert!(!slug.contains(' '));
    }

    #[test]
    fn test_fake_alphanumeric() {
        let code = fake_alphanumeric(10);
        assert_eq!(code.len(), 10);
        assert!(code.chars().all(|c| c.is_ascii_alphanumeric()));

        let short = fake_alphanumeric(6);
        assert_eq!(short.len(), 6);
    }
}
