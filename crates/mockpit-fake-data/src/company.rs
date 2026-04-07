//! Company and job-related generators

use fake::Fake;
use fake::faker::company::en::*;
use fake::faker::job::en::*;

/// Generate a random company name
pub fn fake_company() -> String {
  CompanyName().fake()
}

/// Generate a random company suffix (Inc., LLC, etc.)
pub fn fake_company_suffix() -> String {
  CompanySuffix().fake()
}

/// Generate a random profession/job title
pub fn fake_job_title() -> String {
  Profession().fake()
}

/// Generate a random industry name
pub fn fake_industry() -> String {
  Industry().fake()
}

/// Generate a random job field (Engineering, Marketing, etc.)
pub fn fake_job_field() -> String {
  Field().fake()
}

/// Generate a random job position (Manager, Director, etc.)
pub fn fake_job_position() -> String {
  Position().fake()
}

/// Generate a random job seniority (Junior, Senior, Lead, etc.)
pub fn fake_job_seniority() -> String {
  Seniority().fake()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fake_company() {
    let company = fake_company();
    assert!(!company.is_empty());
  }

  #[test]
  fn test_fake_company_suffix() {
    let suffix = fake_company_suffix();
    assert!(!suffix.is_empty());
  }

  #[test]
  fn test_fake_job_title() {
    let title = fake_job_title();
    assert!(!title.is_empty());
  }

  #[test]
  fn test_fake_industry() {
    let industry = fake_industry();
    assert!(!industry.is_empty());
  }
}
