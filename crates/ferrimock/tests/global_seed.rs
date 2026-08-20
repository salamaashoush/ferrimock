//! The process-wide seeded stream.
//!
//! These assertions are about a singleton: what a draw returns depends on every
//! draw that happened before it, anywhere in the process. Inside the unit-test
//! binary that is not assertable — the ~180 `fake_data` tests that draw without
//! installing a scope run on other threads, and any one of them landing between
//! the two draws below moves the stream. `#[serial]` does not help: it
//! serialises against other serial tests, not against the whole harness.
//!
//! An integration test file is its own process, and nothing else in this one
//! draws. So the stream is genuinely private here, and the property can be
//! asserted rather than approximated.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use ferrimock::fake_data::rng::{reset_streams, scope, set_global_seed};
use ferrimock::fake_data::{fake_name, fake_uuid};

#[test]
#[serial_test::serial]
fn a_global_seed_makes_unscoped_draws_reproducible() {
    let run = || {
        set_global_seed(Some(99));
        (0..5).map(|_| fake_name()).collect::<Vec<_>>()
    };

    let first = run();
    assert_eq!(first, run(), "the same seed replays the same names");

    set_global_seed(Some(100));
    let other = (0..5).map(|_| fake_name()).collect::<Vec<_>>();
    assert_ne!(first, other, "a different seed is a different world");

    set_global_seed(None);
}

#[test]
#[serial_test::serial]
fn reset_streams_replays_without_reseeding() {
    set_global_seed(Some(11));

    let first = fake_uuid();
    reset_streams();
    assert_eq!(
        first,
        fake_uuid(),
        "resetting restarts the process-wide stream"
    );

    // The other half of what `reset_streams` restarts: the per-stream ordinal
    // counters a scope draws its stream from.
    let scoped = {
        let _scope = scope("reset-streams-replay");
        fake_uuid()
    };
    reset_streams();
    let again = {
        let _scope = scope("reset-streams-replay");
        fake_uuid()
    };
    assert_eq!(scoped, again, "and the ordinal a named stream is at");

    set_global_seed(None);
}
