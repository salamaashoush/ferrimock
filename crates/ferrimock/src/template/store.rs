//! Template persistence store functions
//!
//! Provides global persistence store and Tera function registration for
//! store operations (get, set, incr, decr, etc.)

use crate::core::PersistenceStore;
use std::sync::{Arc, OnceLock};
use tera::{Kwargs, State, TeraResult, Value as TeraValue};

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
pub fn set_global_persistence_store(
    store: Arc<PersistenceStore>,
) -> Result<(), Arc<PersistenceStore>> {
    GLOBAL_PERSISTENCE_STORE.set(store)
}

/// Get a clone of the global persistence store
pub fn get_global_persistence_store() -> Arc<PersistenceStore> {
    Arc::clone(get_persistence_store())
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
    tera.register_function(
        "store_get",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let key = kwargs.must_get::<&str>("key")?;
            Ok(get_persistence_store()
                .get(key)
                .map_or_else(TeraValue::none, super::convert::to_tera))
        },
    );

    // store_set(key, value, ttl_seconds=None)
    tera.register_function(
        "store_set",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<String> {
            let key = kwargs.must_get::<&str>("key")?;
            let value = super::convert::to_json(kwargs.must_get::<&TeraValue>("value")?);

            get_persistence_store().set_with_ttl(key.to_string(), value, ttl_from(&kwargs)?);

            // Return empty string for cleaner template syntax
            Ok(String::new())
        },
    );

    // store_incr(key)
    tera.register_function(
        "store_incr",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<i64> {
            let key = kwargs.must_get::<&str>("key")?;
            Ok(get_persistence_store().increment(key.to_string()))
        },
    );

    // store_decr(key)
    tera.register_function(
        "store_decr",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<i64> {
            let key = kwargs.must_get::<&str>("key")?;
            Ok(get_persistence_store().decrement(key.to_string()))
        },
    );

    // store_has(key)
    tera.register_function(
        "store_has",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<bool> {
            let key = kwargs.must_get::<&str>("key")?;
            Ok(get_persistence_store().exists(key))
        },
    );

    // store_del(key)
    tera.register_function(
        "store_del",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<String> {
            let key = kwargs.must_get::<&str>("key")?;
            get_persistence_store().delete(key);
            Ok(String::new())
        },
    );

    // store_clear()
    tera.register_function("store_clear", |_: Kwargs, _: &State<'_>| -> String {
        get_persistence_store().clear();
        String::new()
    });

    // store_keys()
    tera.register_function(
        "store_keys",
        |_: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            Ok(super::convert::to_tera(serde_json::json!(
                get_persistence_store().keys()
            )))
        },
    );

    // store_set_nx(key, value, ttl_seconds=None)
    tera.register_function(
        "store_set_nx",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<bool> {
            let key = kwargs.must_get::<&str>("key")?;
            let value = super::convert::to_json(kwargs.must_get::<&TeraValue>("value")?);

            Ok(get_persistence_store().set_nx_with_ttl(key.to_string(), value, ttl_from(&kwargs)?))
        },
    );

    // store_get_or_set(key, default, ttl_seconds=None)
    tera.register_function(
        "store_get_or_set",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let key = kwargs.must_get::<&str>("key")?;
            let default = kwargs.must_get::<&TeraValue>("default")?;

            if let Some(value) = get_persistence_store().get(key) {
                return Ok(super::convert::to_tera(value));
            }

            get_persistence_store().set_with_ttl(
                key.to_string(),
                super::convert::to_json(default),
                ttl_from(&kwargs)?,
            );
            Ok(default.clone())
        },
    );

    // store_ttl(key)
    tera.register_function(
        "store_ttl",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<TeraValue> {
            let key = kwargs.must_get::<&str>("key")?;
            Ok(get_persistence_store()
                .ttl_seconds(key)
                .map_or_else(TeraValue::none, TeraValue::from))
        },
    );
}

fn ttl_from(kwargs: &Kwargs) -> TeraResult<Option<std::time::Duration>> {
    Ok(kwargs
        .get::<u64>("ttl_seconds")?
        .map(std::time::Duration::from_secs))
}
