//! Internet and networking generators

use fake::Fake;
use fake::faker::internet::en::*;
use rand::RngExt;
use rand::seq::IndexedRandom;
use uuid::Uuid;

/// Generate a random URL
pub fn fake_url() -> String {
  let domain: String = DomainSuffix().fake();
  format!("https://example.{domain}")
}

/// Generate a random domain name
pub fn fake_domain() -> String {
  DomainSuffix().fake()
}

/// Generate a random IPv4 address
pub fn fake_ipv4() -> String {
  IPv4().fake()
}

/// Generate a random IPv6 address
pub fn fake_ipv6() -> String {
  IPv6().fake()
}

/// Generate a random MAC address
pub fn fake_mac_address() -> String {
  MACAddress().fake()
}

/// Generate a random user agent string
pub fn fake_user_agent() -> String {
  UserAgent().fake()
}

/// Generate a random color hex code
pub fn fake_color() -> String {
  format!("#{:06x}", rand::rng().random_range(0..0x00FF_FFFF))
}

/// Generate a pagination URL with page parameter
pub fn fake_pagination_url() -> String {
  let page = rand::rng().random_range(1..=100);
  let limit = rand::rng().random_range(10..=50);
  format!("https://api.example.com/v1/items?page={page}&limit={limit}")
}

/// Generate a pagination URL with offset parameter
pub fn fake_pagination_url_offset() -> String {
  let offset = rand::rng().random_range(0..=1000);
  let limit = rand::rng().random_range(10..=50);
  format!("https://api.example.com/v1/items?offset={offset}&limit={limit}")
}

/// Generate a search/filter URL with query parameters
pub fn fake_search_url() -> String {
  let queries = ["status=active", "type=user", "sort=desc", "filter=new"];
  let query = queries.choose(&mut rand::rng()).copied().unwrap_or("status=active");
  format!("https://api.example.com/v1/search?q={query}")
}

/// Generate a file download URL
pub fn fake_file_download_url() -> String {
  let file_id = Uuid::new_v4().to_string().replace('-', "");
  let token = Uuid::new_v4().to_string().replace('-', "");
  let expires = chrono::Utc::now().timestamp() + 3600;
  format!(
    "https://cdn.example.com/files/{}/download?token={}&expires={}",
    file_id.get(..16).unwrap_or(&file_id),
    token.get(..32).unwrap_or(&token),
    expires
  )
}

/// Generate a versioned API endpoint URL
pub fn fake_api_url() -> String {
  let version = rand::rng().random_range(1..=3);
  let resources = ["users", "documents", "files", "projects", "tasks"];
  let resource = resources.choose(&mut rand::rng()).copied().unwrap_or("users");
  let id = rand::rng().random_range(1..=10000);
  format!("https://api.example.com/v{version}/{resource}/{id}")
}

/// Generate a webhook callback URL
pub fn fake_webhook_url() -> String {
  let event_types = ["payment", "user.created", "document.signed", "file.uploaded"];
  let event = event_types.choose(&mut rand::rng()).copied().unwrap_or("payment");
  format!("https://webhooks.example.com/callbacks/{event}")
}

/// Generate a relative API endpoint path
pub fn fake_api_endpoint() -> String {
  let version = rand::rng().random_range(1..=3);
  let resources = ["users", "documents", "files", "projects", "tasks", "teams"];
  let resource = resources.choose(&mut rand::rng()).copied().unwrap_or("users");
  let id = Uuid::new_v4().to_string();
  format!("/api/v{version}/{resource}/{id}")
}

/// Generate a REST resource path
pub fn fake_resource_path() -> String {
  let resources = ["users", "posts", "comments", "files", "projects"];
  let resource = resources.choose(&mut rand::rng()).copied().unwrap_or("users");
  let id = rand::rng().random_range(1..=10000);
  format!("/{resource}/{id}")
}

/// Generate a user agent string
pub fn fake_user_agent_modern() -> String {
  let browsers = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
  ];
  browsers
    .choose(&mut rand::rng())
    .copied()
    .unwrap_or(
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    )
    .to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fake_url() {
    let url = fake_url();
    assert!(url.starts_with("https://"));
  }

  #[test]
  fn test_fake_domain() {
    let domain = fake_domain();
    assert!(!domain.is_empty());
  }

  #[test]
  fn test_fake_ipv4() {
    let ip = fake_ipv4();
    assert_eq!(ip.split('.').count(), 4);
  }

  #[test]
  fn test_fake_ipv6() {
    let ip = fake_ipv6();
    assert!(ip.contains(':'));
  }

  #[test]
  fn test_fake_mac_address() {
    let mac = fake_mac_address();
    assert!(!mac.is_empty());
  }

  #[test]
  fn test_fake_user_agent() {
    let ua = fake_user_agent();
    assert!(!ua.is_empty());
  }

  #[test]
  fn test_fake_color() {
    let color = fake_color();
    assert!(color.starts_with('#'));
    assert_eq!(color.len(), 7);
  }

  #[test]
  fn test_fake_pagination_url() {
    let url = fake_pagination_url();
    assert!(url.contains("page="));
    assert!(url.contains("limit="));
    assert!(url.starts_with("https://"));
  }

  #[test]
  fn test_fake_api_endpoint() {
    let endpoint = fake_api_endpoint();
    assert!(endpoint.starts_with("/api/v"));
    assert!(endpoint.split('/').count() >= 4);
  }

  #[test]
  fn test_fake_user_agent_modern() {
    let ua = fake_user_agent_modern();
    assert!(ua.contains("Mozilla"));
    assert!(ua.contains("AppleWebKit"));
  }
}
