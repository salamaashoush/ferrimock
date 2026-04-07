//! Scope management for test isolation
//!
//! Scopes provide a way to group mocks together and delete them as a unit.
//! This is essential for test isolation - each test can create a scope,
//! add mocks to it, and automatically clean up when done.

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use lean_string::LeanString;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Information about a scope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeInfo {
    /// Unique scope identifier
    pub id: LeanString,
    /// When the scope was created
    pub created_at: DateTime<Utc>,
    /// When the scope will expire (if TTL is set)
    pub expires_at: Option<DateTime<Utc>>,
    /// Number of mocks in this scope
    pub mock_count: usize,
}

/// Internal scope data
#[derive(Debug, Clone)]
struct ScopeData {
    id: LeanString,
    created_at: DateTime<Utc>,
    expires_at: Option<DateTime<Utc>>,
}

/// Manager for scope lifecycle and TTL cleanup
#[derive(Clone)]
pub struct ScopeManager {
    /// Active scopes
    scopes: Arc<DashMap<LeanString, ScopeData>>,
}

impl ScopeManager {
    /// Create a new scope manager
    pub fn new() -> Self {
        Self {
            scopes: Arc::new(DashMap::new()),
        }
    }

    /// Create a new scope with optional TTL
    pub fn create_scope(&self, id: LeanString, ttl: Option<Duration>) -> Result<ScopeInfo, String> {
        // Check if scope already exists
        if self.scopes.contains_key(&id) {
            return Err(format!("Scope '{id}' already exists"));
        }

        let created_at = Utc::now();
        let expires_at = ttl.and_then(|duration| {
            chrono::Duration::from_std(duration)
                .ok()
                .map(|chrono_dur| created_at + chrono_dur)
        });

        let scope_data = ScopeData {
            id: id.clone(),
            created_at,
            expires_at,
        };

        self.scopes.insert(id.clone(), scope_data);

        Ok(ScopeInfo {
            id,
            created_at,
            expires_at,
            mock_count: 0, // Will be filled by registry
        })
    }

    /// Delete a scope
    pub fn delete_scope(&self, scope_id: &str) -> Result<(), String> {
        if self.scopes.remove(scope_id).is_some() {
            Ok(())
        } else {
            Err(format!("Scope '{scope_id}' not found"))
        }
    }

    /// Check if a scope exists
    pub fn exists(&self, scope_id: &str) -> bool {
        self.scopes.contains_key(scope_id)
    }

    /// Get scope information (without mock count)
    pub fn get_scope(&self, scope_id: &str) -> Option<ScopeInfo> {
        self.scopes.get(scope_id).map(|entry| {
            let data = entry.value();
            ScopeInfo {
                id: data.id.clone(),
                created_at: data.created_at,
                expires_at: data.expires_at,
                mock_count: 0, // Will be filled by registry
            }
        })
    }

    /// Get all scope IDs
    pub fn list_scopes(&self) -> Vec<LeanString> {
        self.scopes
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Clean up expired scopes
    /// Returns list of expired scope IDs that should have their mocks deleted
    pub fn cleanup_expired(&self) -> Vec<LeanString> {
        let now = Utc::now();
        let mut expired = Vec::new();

        // Find expired scopes
        for entry in self.scopes.iter() {
            let data = entry.value();
            if let Some(expires_at) = data.expires_at
                && now >= expires_at
            {
                expired.push(data.id.clone());
            }
        }

        // Remove expired scopes
        for scope_id in &expired {
            self.scopes.remove(scope_id);
        }

        expired
    }

    /// Get number of scopes
    pub fn count(&self) -> usize {
        self.scopes.len()
    }
}

impl Default for ScopeManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn test_create_scope() {
        let manager = ScopeManager::new();

        let info = manager.create_scope("test-scope".into(), None).unwrap();
        assert_eq!(info.id, "test-scope");
        assert!(info.expires_at.is_none());
        assert_eq!(manager.count(), 1);
    }

    #[test]
    fn test_create_scope_with_ttl() {
        let manager = ScopeManager::new();

        let ttl = Duration::from_hours(1);
        let info = manager
            .create_scope("test-scope".into(), Some(ttl))
            .unwrap();

        assert_eq!(info.id, "test-scope");
        assert!(info.expires_at.is_some());

        let expected_expiry = info.created_at + chrono::Duration::seconds(3600);
        let actual_expiry = info.expires_at.unwrap();

        // Allow 1 second difference for test execution time
        let diff = (expected_expiry - actual_expiry).num_seconds().abs();
        assert!(diff <= 1);
    }

    #[test]
    fn test_duplicate_scope_id() {
        let manager = ScopeManager::new();

        manager.create_scope("test-scope".into(), None).unwrap();
        let result = manager.create_scope("test-scope".into(), None);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_delete_scope() {
        let manager = ScopeManager::new();

        manager.create_scope("test-scope".into(), None).unwrap();
        assert_eq!(manager.count(), 1);

        manager.delete_scope("test-scope").unwrap();
        assert_eq!(manager.count(), 0);
    }

    #[test]
    fn test_delete_nonexistent_scope() {
        let manager = ScopeManager::new();

        let result = manager.delete_scope("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_scope_exists() {
        let manager = ScopeManager::new();

        assert!(!manager.exists("test-scope"));

        manager.create_scope("test-scope".into(), None).unwrap();
        assert!(manager.exists("test-scope"));

        manager.delete_scope("test-scope").unwrap();
        assert!(!manager.exists("test-scope"));
    }

    #[test]
    fn test_get_scope() {
        let manager = ScopeManager::new();

        assert!(manager.get_scope("test-scope").is_none());

        manager.create_scope("test-scope".into(), None).unwrap();

        let info = manager.get_scope("test-scope").unwrap();
        assert_eq!(info.id, "test-scope");
        assert!(info.expires_at.is_none());
    }

    #[test]
    fn test_list_scopes() {
        let manager = ScopeManager::new();

        assert_eq!(manager.list_scopes().len(), 0);

        manager.create_scope("scope1".into(), None).unwrap();
        manager.create_scope("scope2".into(), None).unwrap();
        manager.create_scope("scope3".into(), None).unwrap();

        let scopes = manager.list_scopes();
        assert_eq!(scopes.len(), 3);
        assert!(scopes.contains(&LeanString::from("scope1")));
        assert!(scopes.contains(&LeanString::from("scope2")));
        assert!(scopes.contains(&LeanString::from("scope3")));
    }

    #[test]
    fn test_cleanup_expired() {
        let manager = ScopeManager::new();

        // Create scope with very short TTL (1 nanosecond - will be immediately expired)
        manager
            .create_scope("expired".into(), Some(Duration::from_nanos(1)))
            .unwrap();

        // Create scope without TTL
        manager.create_scope("permanent".into(), None).unwrap();

        // Sleep to ensure expiry
        std::thread::sleep(Duration::from_millis(10));

        let expired = manager.cleanup_expired();

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], "expired");
        assert_eq!(manager.count(), 1);
        assert!(manager.exists("permanent"));
        assert!(!manager.exists("expired"));
    }

    #[test]
    fn test_cleanup_no_expired() {
        let manager = ScopeManager::new();

        manager.create_scope("scope1".into(), None).unwrap();
        manager
            .create_scope("scope2".into(), Some(Duration::from_hours(1)))
            .unwrap();

        let expired = manager.cleanup_expired();

        assert_eq!(expired.len(), 0);
        assert_eq!(manager.count(), 2);
    }
}
