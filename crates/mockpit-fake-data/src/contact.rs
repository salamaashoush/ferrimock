//! Contact information generators

use fake::Fake;
use fake::faker::internet::en::*;
use fake::faker::phone_number::en::*;

/// Generate a random safe email address
pub fn fake_email() -> String {
  SafeEmail().fake()
}

/// Generate a random free email address (gmail, yahoo, etc.)
pub fn fake_free_email() -> String {
  FreeEmail().fake()
}

/// Generate a random phone number
pub fn fake_phone() -> String {
  PhoneNumber().fake()
}

/// Generate a random cell phone number
pub fn fake_cell_phone() -> String {
  CellNumber().fake()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fake_email() {
    let email = fake_email();
    assert!(email.contains('@'));
    assert!(email.contains('.'));
  }

  #[test]
  fn test_fake_free_email() {
    let email = fake_free_email();
    assert!(email.contains('@'));
    assert!(email.contains('.'));
  }

  #[test]
  fn test_fake_phone() {
    let phone = fake_phone();
    assert!(!phone.is_empty());
  }
}
