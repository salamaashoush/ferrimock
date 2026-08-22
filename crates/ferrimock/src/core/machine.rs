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
}

/// One named move out of a state.
#[derive(Debug, Clone, PartialEq)]
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
        Self { states }
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
    #[must_use]
    pub fn edge(&self, from: &str, event: &str) -> Option<&Edge> {
        self.get(from)?.on.iter().find(|edge| edge.event == event)
    }

    /// Whether any state names an edge, which is what separates a graph from
    /// an ordering.
    #[must_use]
    pub fn is_drawn(&self) -> bool {
        self.states.iter().any(|state| !state.on.is_empty())
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
                .is_some_and(|state| state.on.iter().any(|edge| edge.target == to));
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
                },
                State {
                    name: "wide".into(),
                    weight: 1.0,
                    empty: Vec::new(),
                    on: Vec::new(),
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

    #[test]
    fn weights_decide_where_a_draw_lands() {
        let machine = Machine::new(vec![
            State {
                name: "rare".into(),
                weight: 1.0,
                empty: Vec::new(),
                on: Vec::new(),
            },
            State {
                name: "common".into(),
                weight: 99.0,
                empty: Vec::new(),
                on: Vec::new(),
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
    declared: FxHashMap<LeanString, Machine>,
    instances: PersistenceStore,
    seen: RwLock<Seen>,
}

impl Machines {
    #[must_use]
    pub fn new(declared: impl IntoIterator<Item = (LeanString, Machine)>) -> Self {
        Self {
            declared: declared.into_iter().collect(),
            instances: PersistenceStore::new(),
            seen: RwLock::new(Seen::default()),
        }
    }

    #[must_use]
    pub fn get(&self, machine: &str) -> Option<&Machine> {
        self.declared.get(machine)
    }

    #[must_use]
    pub fn names(&self) -> Vec<&LeanString> {
        self.declared.keys().collect()
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
        let declared = self.declared.get(machine)?;
        let held = self
            .instances
            .get(&Self::slot(machine, key))
            .and_then(|value| value.as_str().map(LeanString::from))
            .filter(|held| declared.get(held.as_str()).is_some());
        let at = held.or_else(|| declared.initial().map(|state| state.name.clone()))?;
        self.saw_state(machine, &at);
        Some(at)
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
        let Some(declared) = self.declared.get(machine) else {
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
        self.instances
            .set(Self::slot(machine, key), JsonValue::String(to.to_string()));
        {
            let mut seen = self.seen.write();
            seen.edges
                .insert((LeanString::from(machine), from.clone(), edge.event.clone()));
            seen.states.insert((LeanString::from(machine), to.clone()));
        }
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
        for (name, machine) in &self.declared {
            for state in machine.states() {
                if !seen.states.contains(&(name.clone(), state.name.clone())) {
                    missing.states.insert((name.clone(), state.name.clone()));
                }
                for edge in &state.on {
                    let taken = (name.clone(), state.name.clone(), edge.event.clone());
                    if !seen.edges.contains(&taken) {
                        missing.edges.insert(taken);
                    }
                }
            }
        }
        missing
    }

    fn saw_state(&self, machine: &str, state: &LeanString) {
        self.seen
            .write()
            .states
            .insert((LeanString::from(machine), state.clone()));
    }
}
