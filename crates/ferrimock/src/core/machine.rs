//! What something is doing now, and what it is allowed to do next.
//!
//! A machine is named states and the edges between them. Nothing here knows
//! about entities, fields or requests: a record's `status` column, a mocked
//! order's progress across three routes, and a websocket session are the same
//! shape, and they were three separate mechanisms because this one had no name.
//!
//! Edges are optional, and their absence means something specific. A machine
//! that declares none is *monotone* — its states are in order and a move to an
//! earlier one is refused — which is what a lifecycle read off a `world.states`
//! block has always meant. A machine that declares them is a graph, and only
//! the moves it names are allowed. The distinction matters beyond enforcement:
//! a declared edge set can be counted, so "this transition was never taken" is
//! answerable, and an ordering cannot be, because its edges are implied rather
//! than written.

use std::sync::Arc;

use lean_string::LeanString;
use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value as JsonValue;

use crate::core::PersistenceStore;
use crate::fake_data::distribution;

/// A set of named states, and the moves between them.
#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    states: Vec<State>,
    /// Moves available from every state, unless a state names the same event
    /// itself.
    ///
    /// This is what hierarchy is usually reached for: `cancel` working from
    /// anywhere active, without repeating the edge on every state. Nesting buys
    /// the same concision at the cost of transition resolution up a tree and
    /// entry/exit ordering, which is also what makes "which states were never
    /// reached" hard to answer — and answering that is the reason the edges are
    /// declared at all.
    on: Vec<Edge>,
}

/// One state, and what being in it means for everything else.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub name: LeanString,
    /// How much of a generated population sits here.
    ///
    /// Only a census reads this. A machine driving requests has one instance
    /// per key and no population to distribute.
    pub weight: f64,
    /// Fields that hold nothing while this is the state. An order that has not
    /// shipped has no `shipped_at`, and a payload carrying one is a
    /// contradiction rather than an unlikely value.
    pub empty: Vec<LeanString>,
    /// The moves this state names. Empty everywhere means the machine is
    /// ordered rather than drawn.
    pub on: Vec<Edge>,
    /// Moves that happen on their own once an instance has sat here long
    /// enough. A job that finishes after five seconds is closer to what a real
    /// one does than a job that finishes after three polls.
    pub after: Vec<Timer>,
}

/// A move that needs no event, only time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer {
    pub after: std::time::Duration,
    pub target: LeanString,
}

/// One named move out of a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    pub event: LeanString,
    pub target: LeanString,
    /// A condition resolved outside this module, by name. A guard is opaque
    /// here on purpose: the *edge* stays visible to anything counting them
    /// even when whether it fires is someone else's question.
    pub guard: Option<LeanString>,
}

/// What a machine says about one attempted move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Move {
    Allowed,
    /// The target names no state this machine has.
    NoSuchState,
    /// The move is backwards through an ordered machine.
    Backward,
    /// The move is not an edge this machine declares.
    Undeclared,
}

impl Machine {
    #[must_use]
    pub fn new(states: Vec<State>) -> Self {
        Self {
            states,
            on: Vec::new(),
        }
    }

    /// The same machine, with moves available from anywhere.
    #[must_use]
    pub fn with_global(mut self, on: Vec<Edge>) -> Self {
        self.on = on;
        self
    }

    #[must_use]
    pub fn global(&self) -> &[Edge] {
        &self.on
    }

    #[must_use]
    pub fn states(&self) -> &[State] {
        &self.states
    }

    #[must_use]
    pub fn position_of(&self, state: &str) -> Option<usize> {
        self.states.iter().position(|held| held.name == state)
    }

    #[must_use]
    pub fn state(&self, at: usize) -> Option<&State> {
        self.states.get(at)
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&State> {
        self.states.iter().find(|held| held.name == name)
    }

    /// Where an instance of this machine starts.
    ///
    /// The first state, because a machine is written in the order it happens.
    /// A population drawn for an entity does not use this — it draws by weight,
    /// since a census is a snapshot of things at every stage rather than a
    /// cohort that all began at once.
    #[must_use]
    pub fn initial(&self) -> Option<&State> {
        self.states.first()
    }

    /// The edge one event takes out of one state.
    ///
    /// A state's own edge wins over a machine-wide one of the same name, so a
    /// state that has to handle `cancel` differently just says so.
    #[must_use]
    pub fn edge(&self, from: &str, event: &str) -> Option<&Edge> {
        let state = self.get(from)?;
        state
            .on
            .iter()
            .find(|edge| edge.event == event)
            .or_else(|| self.on.iter().find(|edge| edge.event == event))
    }

    /// Every move out of a state: its own, plus the machine-wide ones it does
    /// not shadow. This is what coverage counts.
    pub fn edges_from<'a>(&'a self, from: &'a str) -> impl Iterator<Item = &'a Edge> + 'a {
        let own = self.get(from).map_or(&[][..], |state| state.on.as_slice());
        own.iter().chain(
            self.on
                .iter()
                .filter(move |wide| !own.iter().any(|edge| edge.event == wide.event)),
        )
    }

    /// Whether anything names an edge, which is what separates a graph from an
    /// ordering.
    #[must_use]
    pub fn is_drawn(&self) -> bool {
        !self.on.is_empty()
            || self
                .states
                .iter()
                .any(|state| !state.on.is_empty() || !state.after.is_empty())
    }

    /// Whether moving to `to` is allowed, from `from` or from nowhere.
    ///
    /// A record that holds no state yet can be written into any of them: it is
    /// arriving, not moving.
    #[must_use]
    pub fn allows(&self, from: Option<&str>, to: &str) -> Move {
        let Some(to_at) = self.position_of(to) else {
            return Move::NoSuchState;
        };
        let Some(from) = from else {
            return Move::Allowed;
        };
        let Some(from_at) = self.position_of(from) else {
            return Move::Allowed;
        };
        if self.is_drawn() {
            let declared = self
                .state(from_at)
                .map(|state| state.name.clone())
                .is_some_and(|name| {
                    self.edges_from(name.as_str()).any(|edge| edge.target == to)
                        || self
                            .get(name.as_str())
                            .is_some_and(|state| state.after.iter().any(|t| t.target == to))
                });
            return if declared {
                Move::Allowed
            } else {
                Move::Undeclared
            };
        }
        if to_at < from_at {
            Move::Backward
        } else {
            Move::Allowed
        }
    }

    /// Which state one draw lands in, given what each weighs.
    ///
    /// The weights are the caller's, not a prior: a lifecycle's shape is domain
    /// knowledge, and guessing at it from declaration order is the one thing
    /// that reliably gets it wrong.
    #[must_use]
    pub fn weighted(&self, derived: u64) -> Option<&State> {
        let total: f64 = self.states.iter().map(|state| state.weight.max(0.0)).sum();
        if total <= 0.0 {
            return self.states.first();
        }
        let target = distribution::unit(derived) * total;
        let mut carried = 0.0;
        for state in &self.states {
            carried += state.weight.max(0.0);
            if carried >= target {
                return Some(state);
            }
        }
        self.states.last()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn ordered() -> Machine {
        Machine::new(
            ["draft", "paid", "shipped", "delivered"]
                .into_iter()
                .map(|name| State {
                    name: name.into(),
                    weight: 1.0,
                    empty: Vec::new(),
                    on: Vec::new(),
                    after: Vec::new(),
                })
                .collect(),
        )
    }

    fn edge(event: &str, target: &str) -> Edge {
        Edge {
            event: event.into(),
            target: target.into(),
            guard: None,
        }
    }

    /// A machine that names no edge is the ordering it is written in, which is
    /// what every `world.states` block written so far means.
    #[test]
    fn an_ordered_machine_refuses_only_the_moves_that_go_back() {
        let machine = ordered();
        assert!(!machine.is_drawn());
        assert_eq!(machine.allows(Some("draft"), "shipped"), Move::Allowed);
        assert_eq!(machine.allows(Some("paid"), "paid"), Move::Allowed);
        assert_eq!(machine.allows(Some("shipped"), "draft"), Move::Backward);
        assert_eq!(machine.allows(Some("draft"), "refunded"), Move::NoSuchState);
        // Arriving is not moving.
        assert_eq!(machine.allows(None, "delivered"), Move::Allowed);
    }

    /// One declared edge makes the whole machine a graph, and the ordering
    /// stops meaning anything — which is the point, because an order cannot
    /// express `paid → refunded` beside `paid → shipped`.
    #[test]
    fn a_drawn_machine_allows_what_it_names_and_nothing_else() {
        let mut machine = ordered();
        machine.states[1].on = vec![edge("ship", "shipped"), edge("refund", "draft")];
        assert!(machine.is_drawn());

        assert_eq!(machine.allows(Some("paid"), "shipped"), Move::Allowed);
        // Backwards, and allowed, because it was declared. A refund is a real
        // move that an ordering cannot describe.
        assert_eq!(machine.allows(Some("paid"), "draft"), Move::Allowed);
        // Forwards, and refused, because nothing names it.
        assert_eq!(machine.allows(Some("paid"), "delivered"), Move::Undeclared);
    }

    fn order() -> Machines {
        let state = |name: &str, on: &[(&str, &str)]| State {
            name: name.into(),
            weight: 1.0,
            empty: Vec::new(),
            on: on.iter().map(|(e, t)| edge(e, t)).collect(),
            after: Vec::new(),
        };
        Machines::new([(
            "order".into(),
            Machine::new(vec![
                state("created", &[("pay", "paid"), ("cancel", "cancelled")]),
                state("paid", &[("ship", "shipped"), ("refund", "created")]),
                state("shipped", &[("deliver", "delivered")]),
                state("delivered", &[]),
                state("cancelled", &[]),
            ]),
        )])
    }

    /// An instance does not exist until something asks for it, and it starts
    /// where the machine is written to start.
    #[test]
    fn an_instance_starts_at_the_first_state_and_moves_on_its_edges() {
        let machines = order();
        assert_eq!(machines.state_of("order", "42").as_deref(), Some("created"));

        assert_eq!(
            machines.fire("order", "42", "pay", |_| true),
            Fired::Moved {
                from: "created".into(),
                to: "paid".into()
            }
        );
        assert_eq!(machines.state_of("order", "42").as_deref(), Some("paid"));

        // Keys are separate instances of one machine, which is the whole point
        // of keying them.
        assert_eq!(machines.state_of("order", "43").as_deref(), Some("created"));

        // An event nothing leads out on is refused rather than ignored.
        assert_eq!(
            machines.fire("order", "42", "deliver", |_| true),
            Fired::NoEdge {
                from: "paid".into()
            }
        );

        machines.reset();
        assert_eq!(machines.state_of("order", "42").as_deref(), Some("created"));
    }

    /// A guard is a name, and whether it holds is the caller's answer. The edge
    /// stays declared either way, which is what keeps it countable.
    #[test]
    fn a_guard_is_answered_by_the_caller_and_the_edge_survives_a_no() {
        let machines = Machines::new([(
            "gate".into(),
            Machine::new(vec![
                State {
                    name: "shut".into(),
                    weight: 1.0,
                    empty: Vec::new(),
                    on: vec![Edge {
                        event: "open".into(),
                        target: "wide".into(),
                        guard: Some("has_key".into()),
                    }],
                    after: Vec::new(),
                },
                State {
                    name: "wide".into(),
                    weight: 1.0,
                    empty: Vec::new(),
                    on: Vec::new(),
                    after: Vec::new(),
                },
            ]),
        )]);

        assert_eq!(
            machines.fire("gate", "1", "open", |_| false),
            Fired::Refused {
                from: "shut".into(),
                guard: "has_key".into()
            }
        );
        assert_eq!(machines.state_of("gate", "1").as_deref(), Some("shut"));
        assert!(matches!(
            machines.fire("gate", "1", "open", |guard| guard == "has_key"),
            Fired::Moved { .. }
        ));
    }

    /// The reason to declare edges at all: what a run never exercised is a
    /// question with an answer.
    #[test]
    fn what_a_run_never_reached_is_answerable() {
        let machines = order();
        machines.fire("order", "1", "pay", |_| true);
        machines.fire("order", "1", "ship", |_| true);

        let missing = machines.unreached();
        let state = |name: &str| (LeanString::from("order"), LeanString::from(name));
        assert!(
            !missing.states.contains(&state("shipped")),
            "it was reached"
        );
        assert!(
            missing.states.contains(&state("cancelled")),
            "nothing cancelled anything: {missing:?}"
        );
        assert!(
            missing
                .edges
                .contains(&("order".into(), "created".into(), "cancel".into()))
        );
        assert!(
            !missing
                .edges
                .contains(&("order".into(), "created".into(), "pay".into()))
        );
    }

    /// The concision hierarchy is usually reached for, without the hierarchy:
    /// `cancel` works from anywhere, and a state that needs it to mean
    /// something else just says so.
    #[test]
    fn a_machine_wide_edge_works_from_anywhere_and_a_state_can_shadow_it() {
        let plain = |name: &str, on: &[(&str, &str)]| State {
            name: name.into(),
            weight: 1.0,
            empty: Vec::new(),
            on: on.iter().map(|(e, t)| edge(e, t)).collect(),
            after: Vec::new(),
        };
        let machine = Machine::new(vec![
            plain("created", &[("pay", "paid")]),
            plain("paid", &[("ship", "shipped")]),
            // Shipping is past the point of a plain cancel.
            plain("shipped", &[("cancel", "returned")]),
            plain("cancelled", &[]),
            plain("returned", &[]),
        ])
        .with_global(vec![edge("cancel", "cancelled")]);

        assert_eq!(
            machine.edge("created", "cancel").map(|e| e.target.as_str()),
            Some("cancelled")
        );
        assert_eq!(
            machine.edge("paid", "cancel").map(|e| e.target.as_str()),
            Some("cancelled")
        );
        assert_eq!(
            machine.edge("shipped", "cancel").map(|e| e.target.as_str()),
            Some("returned"),
            "a state's own edge wins"
        );
        // And coverage counts a state's own edges plus the wide ones it does
        // not shadow, never both spellings of the same event.
        let from_shipped: Vec<&str> = machine
            .edges_from("shipped")
            .map(|e| e.event.as_str())
            .collect();
        assert_eq!(from_shipped, vec!["cancel"]);
        let from_paid: Vec<&str> = machine
            .edges_from("paid")
            .map(|e| e.event.as_str())
            .collect();
        assert_eq!(from_paid, vec!["ship", "cancel"]);
    }

    /// A job that finishes after five seconds, rather than after three polls.
    #[test]
    fn a_timer_moves_an_instance_without_anything_asking_it_to() {
        let timed = |name: &str, after: &[(u64, &str)]| State {
            name: name.into(),
            weight: 1.0,
            empty: Vec::new(),
            on: Vec::new(),
            after: after
                .iter()
                .map(|(secs, target)| Timer {
                    after: std::time::Duration::from_secs(*secs),
                    target: (*target).into(),
                })
                .collect(),
        };
        let machines = Machines::new([(
            "job".into(),
            Machine::new(vec![
                timed("queued", &[(5, "running")]),
                timed("running", &[(10, "done")]),
                timed("done", &[]),
            ]),
        )]);

        // The first read starts the clock; nothing has elapsed yet.
        assert_eq!(machines.state_at("job", "1", 0).as_deref(), Some("queued"));
        assert_eq!(
            machines.state_at("job", "1", 4_999).as_deref(),
            Some("queued"),
            "a second short is still queued"
        );
        assert_eq!(
            machines.state_at("job", "1", 5_000).as_deref(),
            Some("running")
        );
        assert_eq!(
            machines.state_at("job", "1", 9_000).as_deref(),
            Some("running"),
            "the second timer runs from when it arrived, not from zero"
        );
        assert_eq!(
            machines.state_at("job", "1", 15_000).as_deref(),
            Some("done")
        );

        // A read long after everything walks the whole chain at once.
        assert_eq!(machines.state_at("job", "2", 0).as_deref(), Some("queued"));
        assert_eq!(
            machines.state_at("job", "2", 1_000_000).as_deref(),
            Some("done")
        );

        // Waiting is counted like any other move, so a timer nothing waited for
        // is a gap coverage can report.
        let missing = machines.unreached();
        assert!(
            !missing
                .edges
                .contains(&("job".into(), "queued".into(), WAITED.into())),
            "that timer fired: {missing:?}"
        );
    }

    #[test]
    fn weights_decide_where_a_draw_lands() {
        let machine = Machine::new(vec![
            State {
                name: "rare".into(),
                weight: 1.0,
                empty: Vec::new(),
                on: Vec::new(),
                after: Vec::new(),
            },
            State {
                name: "common".into(),
                weight: 99.0,
                empty: Vec::new(),
                on: Vec::new(),
                after: Vec::new(),
            },
        ]);
        let common = (0..1000_u64)
            .filter(|draw| {
                machine
                    .weighted(draw.wrapping_mul(0x9E37_79B9_7F4A_7C15))
                    .is_some_and(|state| state.name == "common")
            })
            .count();
        assert!(common > 900, "99 against 1 landed common {common} times");
    }
}

/// What firing an event did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fired {
    Moved {
        from: LeanString,
        to: LeanString,
    },
    /// Nothing leads out of here on that event.
    NoEdge {
        from: LeanString,
    },
    /// The edge is there and its guard said no.
    Refused {
        from: LeanString,
        guard: LeanString,
    },
    NoSuchMachine,
}

/// What a run has actually exercised.
///
/// Counted rather than derived, because the point of declaring edges is that
/// they can be counted: a state nothing reached and an edge nothing took are
/// answerable questions here and are not answerable at all when the edges are
/// implied by an ordering.
#[derive(Debug, Clone, Default)]
pub struct Seen {
    pub states: FxHashSet<(LeanString, LeanString)>,
    pub edges: FxHashSet<(LeanString, LeanString, LeanString)>,
}

/// The machines something declared, and where each instance of them is.
///
/// Instances live in a [`PersistenceStore`] rather than in a census: a census
/// is a known population derived from a seed, and a machine instance does not
/// exist until a request asks for one, with no meaningful count and a key the
/// caller chose.
#[derive(Debug)]
pub struct Machines {
    /// Behind a lock and merged rather than replaced, because a directory holds
    /// several collections and each may declare its own: installing one
    /// collection's machines by overwriting the map made whichever loaded last
    /// the only one that existed. Instances live beside this and outlive a
    /// redeclaration, so a hot reload does not put every order back at the
    /// start.
    declared: RwLock<FxHashMap<LeanString, Arc<Machine>>>,
    instances: PersistenceStore,
    seen: RwLock<Seen>,
}

impl Machines {
    #[must_use]
    pub fn new(declared: impl IntoIterator<Item = (LeanString, Machine)>) -> Self {
        Self {
            declared: RwLock::new(
                declared
                    .into_iter()
                    .map(|(name, machine)| (name, Arc::new(machine)))
                    .collect(),
            ),
            instances: PersistenceStore::new(),
            seen: RwLock::new(Seen::default()),
        }
    }

    /// Add one machine, replacing any of the same name. Instances are untouched.
    pub fn declare(&self, name: impl Into<LeanString>, machine: Machine) {
        self.declared.write().insert(name.into(), Arc::new(machine));
    }

    /// Drop every declaration, keeping instances. What a reload does before it
    /// reads the collections again, so a machine deleted from a file stops
    /// existing.
    pub fn forget_declarations(&self) {
        self.declared.write().clear();
    }

    #[must_use]
    pub fn get(&self, machine: &str) -> Option<Arc<Machine>> {
        self.declared.read().get(machine).map(Arc::clone)
    }

    #[must_use]
    pub fn names(&self) -> Vec<LeanString> {
        let mut names: Vec<LeanString> = self.declared.read().keys().cloned().collect();
        names.sort_unstable();
        names
    }

    /// A machine name is barred from holding the separator, so splitting an
    /// instance key at the first one is unambiguous.
    fn slot(machine: &str, key: &str) -> String {
        format!("{machine}#{key}")
    }

    /// Where one instance is, which is its initial state until it moves.
    ///
    /// A stored value the machine no longer has — a state renamed under a
    /// running world — reads as unstarted rather than as an error, because the
    /// declaration is the truth and the store is a cache of where things got to.
    #[must_use]
    pub fn state_of(&self, machine: &str, key: &str) -> Option<LeanString> {
        self.state_at(machine, key, now_millis())
    }

    /// [`Self::state_of`] against a stated clock.
    ///
    /// Public because a timer that only fires against wall-clock time is a
    /// timer nothing can test, and because replaying a recording wants the
    /// recording's clock rather than this one.
    #[must_use]
    pub fn state_at(&self, machine: &str, key: &str, now: u64) -> Option<LeanString> {
        /// A chain of timers can fire on one read; this stops a cycle of them
        /// from spinning.
        const CHAIN: usize = 32;

        let declared = self.get(machine)?;
        let (held, arrived) = self.held(machine, key);
        let mut at = held
            .filter(|held| declared.get(held.as_str()).is_some())
            .or_else(|| declared.initial().map(|state| state.name.clone()))?;
        // An instance that has never been observed starts its clock now, and
        // that has to be written down: a timer measured from a fresh `now` on
        // every read is a timer that never elapses.
        let mut since = arrived.unwrap_or_else(|| {
            self.remember(machine, key, &at, now);
            now
        });

        for _ in 0..CHAIN {
            let waited = now.saturating_sub(since);
            let Some(timer) = declared.get(at.as_str()).and_then(|state| {
                state
                    .after
                    .iter()
                    .filter(|timer| millis_of(timer.after) <= waited)
                    // The soonest, because a state naming two means the earlier
                    // one already happened.
                    .min_by_key(|timer| timer.after)
            }) else {
                break;
            };
            let to = timer.target.clone();
            since = since.saturating_add(millis_of(timer.after));
            self.remember(machine, key, &to, since);
            self.took(machine, &at, &LeanString::from(WAITED));
            at = to;
        }

        self.saw_state(machine, &at);
        Some(at)
    }

    fn held(&self, machine: &str, key: &str) -> (Option<LeanString>, Option<u64>) {
        match self.instances.get(&Self::slot(machine, key)) {
            // A bare string is an instance stored before timers existed.
            Some(JsonValue::String(state)) => (Some(LeanString::from(state.as_str())), None),
            Some(JsonValue::Object(held)) => (
                held.get("state")
                    .and_then(JsonValue::as_str)
                    .map(LeanString::from),
                held.get("since").and_then(JsonValue::as_u64),
            ),
            _ => (None, None),
        }
    }

    fn remember(&self, machine: &str, key: &str, state: &LeanString, since: u64) {
        self.instances.set(
            Self::slot(machine, key),
            serde_json::json!({ "state": state.as_str(), "since": since }),
        );
    }

    /// Move one instance along the edge an event names.
    ///
    /// Whether a guard holds is answered by the caller: a guard is a name here
    /// and resolving it is somebody else's job, which is what keeps the edge
    /// countable even when the condition is opaque.
    pub fn fire(
        &self,
        machine: &str,
        key: &str,
        event: &str,
        allows: impl Fn(&str) -> bool,
    ) -> Fired {
        let Some(declared) = self.get(machine) else {
            return Fired::NoSuchMachine;
        };
        let Some(from) = self.state_of(machine, key) else {
            return Fired::NoSuchMachine;
        };
        let Some(edge) = declared.edge(from.as_str(), event) else {
            return Fired::NoEdge { from };
        };
        if let Some(guard) = &edge.guard
            && !allows(guard.as_str())
        {
            return Fired::Refused {
                from,
                guard: guard.clone(),
            };
        }
        let to = edge.target.clone();
        self.remember(machine, key, &to, now_millis());
        self.took(machine, &from, &edge.event);
        self.saw_state(machine, &to);
        Fired::Moved { from, to }
    }

    /// Put every instance back where it started. Instances are shared, so they
    /// leak between tests exactly the way world writes do.
    pub fn reset(&self) {
        self.instances.clear();
        *self.seen.write() = Seen::default();
    }

    #[must_use]
    pub fn seen(&self) -> Seen {
        self.seen.read().clone()
    }

    /// States and edges a run declared and never exercised.
    #[must_use]
    pub fn unreached(&self) -> Seen {
        let seen = self.seen.read();
        let mut missing = Seen::default();
        for (name, machine) in self.declared.read().iter() {
            for state in machine.states() {
                if !seen.states.contains(&(name.clone(), state.name.clone())) {
                    missing.states.insert((name.clone(), state.name.clone()));
                }
                for edge in machine.edges_from(state.name.as_str()) {
                    let taken = (name.clone(), state.name.clone(), edge.event.clone());
                    if !seen.edges.contains(&taken) {
                        missing.edges.insert(taken);
                    }
                }
                if !state.after.is_empty() {
                    let waited = (name.clone(), state.name.clone(), LeanString::from(WAITED));
                    if !seen.edges.contains(&waited) {
                        missing.edges.insert(waited);
                    }
                }
            }
        }
        missing
    }

    fn took(&self, machine: &str, from: &LeanString, event: &LeanString) {
        self.seen
            .write()
            .edges
            .insert((LeanString::from(machine), from.clone(), event.clone()));
    }

    fn saw_state(&self, machine: &str, state: &LeanString) {
        self.seen
            .write()
            .states
            .insert((LeanString::from(machine), state.clone()));
    }
}

/// The event name a timed move is counted under, so waiting is coverable the
/// same way an event is.
pub const WAITED: &str = "<after>";

fn millis_of(span: std::time::Duration) -> u64 {
    u64::try_from(span.as_millis()).unwrap_or(u64::MAX)
}

/// Wall-clock milliseconds, or zero before the epoch, which cannot happen.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}
