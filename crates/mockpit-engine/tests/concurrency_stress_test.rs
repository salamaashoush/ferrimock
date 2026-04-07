#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Concurrency stress tests for PersistenceStore
//!
//! These tests verify that the persistence store handles high-concurrency scenarios
//! without race conditions, data corruption, or deadlocks.

use mockpit_core::PersistenceStore;
use serde_json::json;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn test_high_concurrency_increments() {
    const NUM_THREADS: usize = 20;
    const INCREMENTS_PER_THREAD: usize = 1000;

    let store = Arc::new(PersistenceStore::new());
    let mut handles = vec![];

    // Spawn many threads all incrementing the same counter
    for _ in 0..NUM_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for _ in 0..INCREMENTS_PER_THREAD {
                store_clone.increment("shared_counter".to_string());
            }
        });
        handles.push(handle);
    }

    // Wait for all threads to complete
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify final count is exactly correct (no lost updates)
    let expected = i64::try_from(NUM_THREADS * INCREMENTS_PER_THREAD).unwrap();
    let actual = store.get("shared_counter").unwrap().as_i64().unwrap();

    assert_eq!(
        actual,
        expected,
        "Expected {} increments but got {}. Lost {} updates!",
        expected,
        actual,
        expected - actual
    );
}

#[test]
fn test_mixed_operations_concurrent() {
    const NUM_THREADS: usize = 10;
    const OPERATIONS_PER_THREAD: usize = 100;

    let store = Arc::new(PersistenceStore::new());
    let mut handles = vec![];

    // Initialize some data
    for i in 0..10 {
        store.set(format!("counter_{i}"), json!(0));
    }

    // Spawn threads doing mixed operations
    for thread_id in 0..NUM_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for i in 0..OPERATIONS_PER_THREAD {
                let key = format!("counter_{}", i % 10);

                // Mix of operations
                match (thread_id + i) % 5 {
                    0 => {
                        store_clone.increment(key);
                    }
                    1 => {
                        store_clone.decrement(key);
                    }
                    2 => {
                        store_clone.set(key, json!(i));
                    }
                    3 => {
                        let _ = store_clone.get(&key);
                    }
                    4 => {
                        let _ = store_clone.exists(&key);
                    }
                    5.. => {}
                }
            }
        });
        handles.push(handle);
    }

    // Wait for completion
    for handle in handles {
        handle.join().unwrap();
    }

    // Verify all keys still exist (no corruption)
    for i in 0..10 {
        let key = format!("counter_{i}");
        assert!(
            store.exists(&key),
            "Key {key} should still exist after concurrent ops"
        );
    }
}

#[test]
fn test_concurrent_get_set_same_key() {
    const NUM_THREADS: usize = 50;
    const ITERATIONS: usize = 100;

    let store = Arc::new(PersistenceStore::new());
    store.set("shared_key".to_string(), json!(0));

    let mut handles = vec![];

    for thread_id in 0..NUM_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for i in 0..ITERATIONS {
                // Read current value
                let _current = store_clone
                    .get("shared_key")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                // Write new value based on thread_id and iteration
                let new_value = i64::try_from(thread_id * 1000 + i).unwrap();
                store_clone.set("shared_key".to_string(), json!(new_value));

                // Small random delay to increase contention
                if i % 10 == 0 {
                    thread::sleep(Duration::from_micros(10));
                }
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Should complete without deadlock or panic
    assert!(store.exists("shared_key"));
    println!("Final value: {:?}", store.get("shared_key"));
}

#[test]
fn test_concurrent_delete_and_recreate() {
    const NUM_THREADS: usize = 20;
    const ITERATIONS: usize = 50;

    let store = Arc::new(PersistenceStore::new());
    let mut handles = vec![];

    for thread_id in 0..NUM_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for i in 0..ITERATIONS {
                let key = format!("volatile_key_{}", i % 5);

                // Try to delete (may or may not succeed if another thread recreated it)
                let _ = store_clone.delete(&key);

                // Immediately recreate with thread-specific key to avoid conflicts
                let thread_key = format!("{key}_{thread_id}");
                store_clone.set(thread_key.clone(), json!({"thread": thread_id, "iter": i}));

                // Verify our thread's key exists
                assert!(store_clone.exists(&thread_key));
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Verify we have many keys (should be NUM_THREADS * 5)
    let keys = store.keys();
    let expected_min = NUM_THREADS * 5;
    assert!(
        keys.len() >= expected_min,
        "Expected at least {} keys, got {}",
        expected_min,
        keys.len()
    );
}

#[test]
fn test_concurrent_keys_operation() {
    const NUM_WRITER_THREADS: usize = 10;
    const NUM_READER_THREADS: usize = 10;
    const OPERATIONS: usize = 100;

    let store = Arc::new(PersistenceStore::new());
    let mut handles = vec![];

    // Writer threads constantly adding/removing keys
    for thread_id in 0..NUM_WRITER_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for i in 0..OPERATIONS {
                let key = format!("key_{thread_id}_{i}");
                store_clone.set(key.clone(), json!(i));

                if i % 2 == 0 {
                    store_clone.delete(&key);
                }
            }
        });
        handles.push(handle);
    }

    // Reader threads constantly calling keys()
    for _ in 0..NUM_READER_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for _ in 0..OPERATIONS {
                let keys = store_clone.keys();
                // Just verify it doesn't crash or deadlock
                let _len = keys.len();
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Should complete without errors
    let final_keys = store.keys();
    println!("Final key count: {}", final_keys.len());
}

#[test]
fn test_concurrent_increment_decrement_balance() {
    const NUM_INCREMENT_THREADS: usize = 10;
    const NUM_DECREMENT_THREADS: usize = 10;
    const OPERATIONS_PER_THREAD: usize = 500;

    let store = Arc::new(PersistenceStore::new());
    store.set("balance".to_string(), json!(0));

    let mut handles = vec![];

    // Incrementing threads
    for _ in 0..NUM_INCREMENT_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for _ in 0..OPERATIONS_PER_THREAD {
                store_clone.increment("balance".to_string());
            }
        });
        handles.push(handle);
    }

    // Decrementing threads
    for _ in 0..NUM_DECREMENT_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for _ in 0..OPERATIONS_PER_THREAD {
                store_clone.decrement("balance".to_string());
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Final balance should be 0 (equal increments and decrements)
    let final_balance = store.get("balance").unwrap().as_i64().unwrap();
    assert_eq!(
        final_balance, 0,
        "Balance should be 0 but was {final_balance}. Race condition detected!"
    );
}

#[test]
fn test_no_deadlock_with_clear() {
    const NUM_THREADS: usize = 20;
    const ITERATIONS: usize = 50;

    let store = Arc::new(PersistenceStore::new());
    let mut handles = vec![];

    for thread_id in 0..NUM_THREADS {
        let store_clone = Arc::clone(&store);
        let handle = thread::spawn(move || {
            for i in 0..ITERATIONS {
                if thread_id == 0 && i % 10 == 0 {
                    // One thread periodically clears
                    store_clone.clear();
                } else {
                    // Others keep adding data
                    store_clone.set(format!("key_{thread_id}_{i}"), json!({"value": i}));
                    let _ = store_clone.keys();
                }
            }
        });
        handles.push(handle);
    }

    // Should complete without deadlock
    for handle in handles {
        handle.join().expect("Thread should not panic");
    }

    println!("Test completed successfully - no deadlock");
}
