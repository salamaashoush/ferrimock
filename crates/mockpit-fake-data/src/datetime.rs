//! Date and time generators

use fake::Fake;
use fake::faker::chrono::en::*;
use rand::RngExt;
use rand::seq::IndexedRandom;

/// Generate a random date in RFC3339 format
pub fn fake_date() -> String {
  DateTime().fake::<chrono::DateTime<chrono::Utc>>().to_rfc3339()
}

/// Generate a random time string
pub fn fake_time() -> String {
  Time().fake()
}

/// Generate an ISO date (date only, no time)
pub fn fake_iso_date() -> String {
  let year = rand::rng().random_range(2020..=2025);
  let month = rand::rng().random_range(1..=12);
  let day = rand::rng().random_range(1..=28);
  format!("{year:04}-{month:02}-{day:02}")
}

/// Generate a Unix timestamp
pub fn fake_unix_timestamp() -> i64 {
  rand::rng().random_range(1_640_000_000..=1_900_000_000)
}

/// Generate a relative time string
pub fn fake_relative_time() -> String {
  let times = [
    "2 hours ago",
    "1 day ago",
    "3 days ago",
    "1 week ago",
    "2 weeks ago",
    "1 month ago",
  ];
  times
    .choose(&mut rand::rng())
    .copied()
    .unwrap_or("1 day ago")
    .to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fake_date() {
    let date = fake_date();
    assert!(!date.is_empty());
    assert!(date.contains('T'));
  }

  #[test]
  fn test_fake_time() {
    let time = fake_time();
    assert!(!time.is_empty());
  }

  #[test]
  fn test_fake_iso_date() {
    let date = fake_iso_date();
    assert!(date.contains('-'));
    let parts: Vec<&str> = date.split('-').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0].len(), 4);
    assert_eq!(parts[1].len(), 2);
    assert_eq!(parts[2].len(), 2);
  }

  #[test]
  fn test_fake_unix_timestamp() {
    let timestamp = fake_unix_timestamp();
    assert!(timestamp >= 1_640_000_000);
    assert!(timestamp <= 1_900_000_000);
  }

  #[test]
  fn test_fake_relative_time() {
    let time = fake_relative_time();
    assert!(!time.is_empty());
    assert!(time.contains("ago"));
  }
}
