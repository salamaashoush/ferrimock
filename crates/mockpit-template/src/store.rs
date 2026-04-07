//! Template persistence store functions
//!
//! Provides global persistence store and Tera function registration for
//! store operations (get, set, incr, decr, etc.)

// Tera library callbacks require std::collections::HashMap - cannot use FxHashMap
#![allow(clippy::disallowed_types)]

use mockpit_core::PersistenceStore;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

// ============================================================================
// GLOBAL PERSISTENCE STORE
// ============================================================================

// Global shared persistence store (thread-safe, shared across all requests)
static GLOBAL_PERSISTENCE_STORE: OnceLock<Arc<PersistenceStore>> = OnceLock::new();

/// Get or initialize the global persistence store
fn get_persistence_store() -> &'static Arc<PersistenceStore> {
  GLOBAL_PERSISTENCE_STORE.get_or_init(|| Arc::new(PersistenceStore::new()))
}

/// Set the global persistence store
/// This should be called once during initialization before any templates are rendered
pub fn set_global_persistence_store(store: Arc<PersistenceStore>) -> Result<(), Arc<PersistenceStore>> {
  GLOBAL_PERSISTENCE_STORE.set(store)
}

/// Get a clone of the global persistence store
pub fn get_global_persistence_store() -> Arc<PersistenceStore> {
  get_persistence_store().clone()
}

// ============================================================================
// TERA REGISTRATION HELPER
// ============================================================================

/// Register all persistence store functions with a Tera instance
///
/// This function registers all store-related functions (store_get, store_set, etc.)
/// that can be used in templates for stateful mock scenarios.
pub fn register_all_functions(tera: &mut tera::Tera) {
  // store_get(key) - supports dot notation for namespaces
  tera.register_function("store_get", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_get requires 'key' parameter"))?;

    Ok(get_persistence_store().get(key).unwrap_or(Value::Null))
  });

  // store_set(key, value, ttl_seconds=None)
  tera.register_function("store_set", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_set requires 'key' parameter"))?;
    let value = args
      .get("value")
      .ok_or_else(|| tera::Error::msg("store_set requires 'value' parameter"))?;

    let ttl_seconds = args.get("ttl_seconds").and_then(|v| v.as_u64());
    let ttl = ttl_seconds.map(std::time::Duration::from_secs);

    get_persistence_store().set_with_ttl(key.to_string(), value.clone(), ttl);

    // Return empty string for cleaner template syntax
    Ok(Value::String(String::new()))
  });

  // store_incr(key)
  tera.register_function("store_incr", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_incr requires 'key' parameter"))?;

    Ok(Value::Number(get_persistence_store().increment(key.to_string()).into()))
  });

  // store_decr(key)
  tera.register_function("store_decr", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_decr requires 'key' parameter"))?;

    Ok(Value::Number(get_persistence_store().decrement(key.to_string()).into()))
  });

  // store_has(key)
  tera.register_function("store_has", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_has requires 'key' parameter"))?;

    Ok(Value::Bool(get_persistence_store().exists(key)))
  });

  // store_del(key)
  tera.register_function("store_del", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_del requires 'key' parameter"))?;

    get_persistence_store().delete(key);
    Ok(Value::String(String::new()))
  });

  // store_clear()
  tera.register_function("store_clear", |_args: &HashMap<String, Value>| -> tera::Result<Value> {
    get_persistence_store().clear();
    Ok(Value::String(String::new()))
  });

  // store_keys()
  tera.register_function("store_keys", |_args: &HashMap<String, Value>| -> tera::Result<Value> {
    let keys = get_persistence_store()
      .keys()
      .into_iter()
      .map(Value::String)
      .collect::<Vec<Value>>();

    Ok(Value::Array(keys))
  });

  // store_set_nx(key, value, ttl_seconds=None)
  tera.register_function("store_set_nx", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_set_nx requires 'key' parameter"))?;
    let value = args
      .get("value")
      .ok_or_else(|| tera::Error::msg("store_set_nx requires 'value' parameter"))?;

    let ttl_seconds = args.get("ttl_seconds").and_then(|v| v.as_u64());
    let ttl = ttl_seconds.map(std::time::Duration::from_secs);

    let was_set = get_persistence_store().set_nx_with_ttl(key.to_string(), value.clone(), ttl);

    Ok(Value::Bool(was_set))
  });

  // store_get_or_set(key, default, ttl_seconds=None)
  tera.register_function(
    "store_get_or_set",
    |args: &HashMap<String, Value>| -> tera::Result<Value> {
      let key = args
        .get("key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| tera::Error::msg("store_get_or_set requires 'key' parameter"))?;
      let default = args
        .get("default")
        .ok_or_else(|| tera::Error::msg("store_get_or_set requires 'default' parameter"))?;

      let ttl_seconds = args.get("ttl_seconds").and_then(|v| v.as_u64());
      let ttl = ttl_seconds.map(std::time::Duration::from_secs);

      // Try to get existing value
      if let Some(value) = get_persistence_store().get(key) {
        Ok(value)
      } else {
        // Set the default value and return it
        get_persistence_store().set_with_ttl(key.to_string(), default.clone(), ttl);
        Ok(default.clone())
      }
    },
  );

  // store_ttl(key)
  tera.register_function("store_ttl", |args: &HashMap<String, Value>| -> tera::Result<Value> {
    let key = args
      .get("key")
      .and_then(|v| v.as_str())
      .ok_or_else(|| tera::Error::msg("store_ttl requires 'key' parameter"))?;

    let ttl_secs = get_persistence_store().ttl_seconds(key);

    Ok(ttl_secs.map(|s| Value::Number(s.into())).unwrap_or(Value::Null))
  });
}
