//! Identity and personal data generators

use fake::Fake;
use fake::faker::name::en::*;

/// Generate a random full name
pub fn fake_name() -> String {
  Name().fake()
}

/// Generate a random first name
pub fn fake_first_name() -> String {
  FirstName().fake()
}

/// Generate a random last name
pub fn fake_last_name() -> String {
  LastName().fake()
}

/// Generate a random username
pub fn fake_username() -> String {
  use fake::faker::internet::en::Username;
  Username().fake()
}

/// Generate a random password (8-16 characters)
pub fn fake_password() -> String {
  use fake::faker::internet::en::Password;
  Password(8..16).fake()
}

/// Generate a random title (Mr., Mrs., Dr., etc.)
pub fn fake_title() -> String {
  Title().fake()
}

/// Generate a random suffix (Jr., Sr., III, etc.)
pub fn fake_suffix() -> String {
  Suffix().fake()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fake_name() {
    let name = fake_name();
    assert!(!name.is_empty());
    assert!(name.contains(' '));
  }

  #[test]
  fn test_fake_first_name() {
    let name = fake_first_name();
    assert!(!name.is_empty());
  }

  #[test]
  fn test_fake_last_name() {
    let name = fake_last_name();
    assert!(!name.is_empty());
  }

  #[test]
  fn test_fake_username() {
    let username = fake_username();
    assert!(!username.is_empty());
  }

  #[test]
  fn test_fake_password() {
    let password = fake_password();
    assert!(password.len() >= 8);
    assert!(password.len() <= 16);
  }

  #[test]
  fn test_fake_title() {
    let title = fake_title();
    assert!(!title.is_empty());
  }
}
