#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Properties a machine has to hold whatever it is asked.
//!
//! The timer bug was a property violation and nothing else: an instance never
//! wrote down when it arrived, so every read measured the wait from a fresh
//! `now` and no timer ever elapsed. A single example test happened to catch it,
//! but only because the example read twice at different clocks. Monotonicity is
//! the general statement of that, and it does not depend on picking the right
//! example.

use std::time::Duration;

use ferrimock::core::machine::{Machine, Machines, State, Timer};
use proptest::prelude::*;

/// A chain: each state moves to the next after a second, the last is terminal.
fn chain(length: usize) -> Machines {
    let states = (0..length)
        .map(|at| State {
            name: format!("s{at}").into(),
            weight: 1.0,
            empty: Vec::new(),
            on: Vec::new(),
            after: if at + 1 < length {
                vec![Timer {
                    after: Duration::from_secs(1),
                    target: format!("s{}", at + 1).into(),
                }]
            } else {
                Vec::new()
            },
        })
        .collect();
    Machines::new([("chain".into(), Machine::new(states))])
}

proptest! {
    /// Time only moves an instance forward. A read at a later clock can never
    /// answer an earlier state than a read at an earlier one, which is exactly
    /// what the un-persisted arrival broke.
    #[test]
    fn a_later_clock_never_answers_an_earlier_state(
        length in 2usize..8,
        mut clocks in proptest::collection::vec(0u64..20_000, 1..12),
    ) {
        let machines = chain(length);
        clocks.sort_unstable();

        let position = |name: &str| name.trim_start_matches('s').parse::<usize>().unwrap_or(0);
        let mut furthest = 0;
        for now in clocks {
            let at = machines.state_at("chain", "1", now).expect("a state");
            let reached = position(&at);
            prop_assert!(
                reached >= furthest,
                "went back from s{furthest} to {at} at {now}ms"
            );
            furthest = reached;
        }
    }

    /// Reading is not moving, however often it is done at one instant.
    #[test]
    fn reading_at_one_instant_is_idempotent(
        length in 2usize..8,
        now in 0u64..20_000,
        reads in 2usize..10,
    ) {
        let machines = chain(length);
        let first = machines.state_at("chain", "1", now).expect("a state");
        for _ in 1..reads {
            prop_assert_eq!(
                machines.state_at("chain", "1", now).expect("a state"),
                first.clone()
            );
        }
    }

    /// A key is an instance, and walking one leaves the others alone.
    ///
    /// Note what this pins down: an instance's clock starts when it is first
    /// *observed*, not at some epoch, because there is no other honest answer
    /// for something nothing has ever asked about. So `fresh` sits at the start
    /// however late it is first read, and `walked` moves only because it was
    /// read early and read again later.
    #[test]
    fn one_key_cannot_be_seen_through_another(
        length in 3usize..8,
        later in 2_000u64..20_000,
    ) {
        let machines = chain(length);

        // Observed at zero, so its clock starts at zero.
        let start = machines.state_at("chain", "walked", 0).expect("a state");
        prop_assert_eq!(start.as_str(), "s0");
        let walked = machines.state_at("chain", "walked", later).expect("a state");
        prop_assert!(walked.as_str() != "s0", "it should have moved by {later}ms");

        // First observed now, so nothing has elapsed for it.
        let fresh = machines.state_at("chain", "fresh", later).expect("a state");
        prop_assert_eq!(fresh.as_str(), "s0");
        // And observing it did not disturb the other one.
        prop_assert_eq!(
            machines.state_at("chain", "walked", later).expect("a state"),
            walked
        );
    }
}
