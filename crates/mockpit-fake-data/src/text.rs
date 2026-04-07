//! Text and content generators

use fake::Fake;
use fake::faker::lorem::en::*;

/// Generate random words (count specified)
pub fn fake_words(count: usize) -> String {
  let words: Vec<String> = Words(count..count + 1).fake();
  words.join(" ")
}

/// Generate a random sentence with specified word count (default: 5)
pub fn fake_sentence(word_count: usize) -> String {
  let count = word_count.max(1);
  let words: Vec<String> = Words(count..count + 1).fake();
  words.join(" ")
}

/// Generate a random paragraph with specified sentence count (default: 3)
pub fn fake_paragraph(sentence_count: usize) -> String {
  let count = sentence_count.max(1);
  let paragraph: Vec<String> = Sentences(count..count + 1).fake();
  paragraph.join(" ")
}

/// Generate a random word
pub fn fake_word() -> String {
  Word().fake()
}

/// Generate a slug (URL-friendly string)
pub fn fake_slug() -> String {
  let words: Vec<String> = Words(3..5).fake();
  words.join("-").to_lowercase()
}

/// Generate a random alphanumeric string of specified length
/// Useful for codes, references, and other unknown string patterns
pub fn fake_alphanumeric(length: usize) -> String {
  use rand::seq::IndexedRandom;
  const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
  let mut rng = rand::rng();

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
