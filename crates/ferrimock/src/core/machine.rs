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
