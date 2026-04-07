//! Finance and commerce generators

use fake::Fake;
use fake::faker::creditcard::en::*;
use fake::faker::currency::en::*;
use rand::RngExt;

/// Generate a random credit card number
pub fn fake_credit_card() -> String {
  CreditCardNumber().fake()
}

/// Generate a random currency code (USD, EUR, GBP, etc.)
pub fn fake_currency_code() -> String {
  CurrencyCode().fake()
}

/// Generate a random currency name
pub fn fake_currency_name() -> String {
  CurrencyName().fake()
}

/// Generate a random currency symbol
pub fn fake_currency_symbol() -> String {
  CurrencySymbol().fake()
}

/// Generate a random price between min and max
pub fn fake_price(min: f64, max: f64) -> f64 {
  rand::rng().random_range(min..=max)
}

/// Generate a random amount with 2 decimal places
pub fn fake_amount() -> String {
  format!("{:.2}", fake_price(1.0, 9999.99))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fake_credit_card() {
    let card = fake_credit_card();
    assert!(!card.is_empty());
  }

  #[test]
  fn test_fake_currency_code() {
    let code = fake_currency_code();
    assert_eq!(code.len(), 3);
  }

  #[test]
  fn test_fake_currency_name() {
    let name = fake_currency_name();
    assert!(!name.is_empty());
  }

  #[test]
  fn test_fake_price() {
    let price = fake_price(10.0, 100.0);
    assert!((10.0..=100.0).contains(&price));
  }

  #[test]
  fn test_fake_amount() {
    let amount = fake_amount();
    assert!(amount.contains('.'));
    let parts: Vec<&str> = amount.split('.').collect();
    assert_eq!(parts[1].len(), 2);
  }
}
