//! Persistence store for cross-request state management
//!
//! Provides a thread-safe in-memory key-value store that allows templates and scripts
//! to maintain state across multiple mock requests. This enables powerful scenarios like:
//! - Stateful workflows and multi-step processes
//! - Request counters and rate limiting simulation
//! - Paginated responses with state tracking
//! - Session simulation
//! - A/B testing with persistent state

use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Entry in persistence store with optional TTL
#[derive(Debug, Clone)]
struct PersistenceEntry {
    /// The stored value
    value: Value,
    /// When this entry was created
    created_at: Instant,
    /// Optional time-to-live for automatic expiration
    ttl: Option<Duration>,
}

impl PersistenceEntry {
    /// Check if this entry has expired based on its TTL
    fn is_expired(&self) -> bool {
        if let Some(ttl) = self.ttl {
            self.created_at.elapsed() > ttl
        } else {
            false
        }
    }
}

/// Thread-safe persistence store for cross-request state
///
/// This store uses DashMap for lock-free concurrent access, making it safe
/// to use from multiple threads without explicit locking.
#[derive(Debug, Clone)]
pub struct PersistenceStore {
    /// Internal storage using DashMap for thread-safe access
    data: Arc<DashMap<String, PersistenceEntry>>,
}

impl PersistenceStore {
    /// Create a new empty persistence store
    pub fn new() -> Self {
        Self {
            data: Arc::new(DashMap::new()),
        }
    }

    /// Set a value (replaces existing value if present)
    pub fn set(&self, key: String, value: Value) {
        self.set_with_ttl(key, value, None);
    }

    /// Set a value with a time-to-live duration
    ///
    /// The value will automatically expire and be removed after the TTL elapses.
    pub fn set_with_ttl(&self, key: String, value: Value, ttl: Option<Duration>) {
        let entry = PersistenceEntry {
            value,
            created_at: Instant::now(),
            ttl,
        };
        self.data.insert(key, entry);
    }

    /// Get a value from the store
    ///
    /// Returns `None` if the key doesn't exist or if the value has expired.
    pub fn get(&self, key: &str) -> Option<Value> {
        let entry = self.data.get(key)?;

        // Check if expired
        if entry.is_expired() {
            drop(entry); // Release read lock
            self.data.remove(key); // Remove expired entry
            return None;
        }

        Some(entry.value.clone())
    }

    /// Increment a numeric counter atomically
    ///
    /// If the key doesn't exist, it will be created with an initial value of 1.
    /// If the existing value is not a number, it will be treated as 0.
    pub fn increment(&self, key: String) -> i64 {
        self.data
            .entry(key)
            .and_modify(|entry| {
                if let Some(num) = entry.value.as_i64() {
                    entry.value = Value::Number(num.wrapping_add(1).into());
                } else {
                    entry.value = Value::Number(1.into());
                }
                entry.created_at = Instant::now();
            })
            .or_insert_with(|| PersistenceEntry {
                value: Value::Number(1.into()),
                created_at: Instant::now(),
                ttl: None,
            })
            .value
            .as_i64()
            .unwrap_or(1)
    }

    /// Decrement a numeric counter atomically
    ///
    /// If the key doesn't exist, it will be created with an initial value of -1.
    /// If the existing value is not a number, it will be treated as 0.
    pub fn decrement(&self, key: String) -> i64 {
        self.data
            .entry(key)
            .and_modify(|entry| {
                if let Some(num) = entry.value.as_i64() {
                    entry.value = Value::Number(num.wrapping_sub(1).into());
                } else {
                    entry.value = Value::Number((-1).into());
                }
                entry.created_at = Instant::now();
            })
            .or_insert_with(|| PersistenceEntry {
                value: Value::Number((-1).into()),
                created_at: Instant::now(),
                ttl: None,
            })
            .value
            .as_i64()
            .unwrap_or(-1)
    }

    /// Delete a key from the store
    ///
    /// Returns `true` if the key existed and was deleted, `false` otherwise.
    pub fn delete(&self, key: &str) -> bool {
        self.data.remove(key).is_some()
    }

    /// Check if a key exists and has not expired
    pub fn exists(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Clear all data from the store
    pub fn clear(&self) {
        self.data.clear();
    }

    /// Get all keys currently in the store (excluding expired entries)
    pub fn keys(&self) -> Vec<String> {
        self.data
            .iter()
            .filter(|entry| !entry.is_expired())
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get the number of entries in the store (excluding expired entries)
    pub fn len(&self) -> usize {
        self.keys().len()
    }

    /// Check if the store is empty (has no non-expired entries)
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clean up expired entries and return the count of removed entries
    pub fn cleanup_expired(&self) -> usize {
        let expired_keys: Vec<String> = self
            .data
            .iter()
            .filter_map(|entry| {
                if entry.is_expired() {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();

        let count = expired_keys.len();
        for key in expired_keys {
            self.data.remove(&key);
        }
        count
    }

    /// Get the time-to-live (TTL) remaining for a key
    ///
    /// Returns `None` if the key doesn't exist, has no TTL, or has expired.
    pub fn ttl(&self, key: &str) -> Option<Duration> {
        let entry = self.data.get(key)?;

        if entry.is_expired() {
            return None;
        }

        entry.ttl.and_then(|ttl| {
            let elapsed = entry.created_at.elapsed();
            if elapsed < ttl {
                ttl.checked_sub(elapsed)
            } else {
                None
            }
        })
    }

    /// Get TTL remaining in seconds (convenience method for Tera)
    ///
    /// Returns `None` if the key doesn't exist, has no TTL, or has expired.
    pub fn ttl_seconds(&self, key: &str) -> Option<u64> {
        self.ttl(key).map(|d| d.as_secs())
    }

    /// Set a value only if the key does not exist (SET if Not eXists)
    ///
    /// Returns `true` if the value was set, `false` if the key already exists.
    pub fn set_nx(&self, key: String, value: Value) -> bool {
        self.set_nx_with_ttl(key, value, None)
    }

    /// Set a value only if the key does not exist, with optional TTL
    ///
    /// Returns `true` if the value was set, `false` if the key already exists.
    pub fn set_nx_with_ttl(&self, key: String, value: Value, ttl: Option<Duration>) -> bool {
        let entry = PersistenceEntry {
            value,
            created_at: Instant::now(),
            ttl,
        };

        // Check if key exists and is not expired
        if let Some(existing) = self.data.get(&key) {
            if !existing.is_expired() {
                return false;
            }
            // If expired, remove it and continue with insert
            drop(existing);
            self.data.remove(&key);
        }

        self.data.insert(key, entry);
        true
    }
}

impl Default for PersistenceStore {
    fn default() -> Self {
        Self::new()
    }
}
