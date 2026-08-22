//! Template access to the machines a collection declared.
//!
//! Reading a machine and moving it are different things and are spelled
//! differently, because a `GET` that advances a lifecycle is the bug every
//! poll-counter scenario has: `machine_state` never moves anything, and
//! `machine_fire` is the only thing that does.

use std::sync::{Arc, OnceLock, RwLock};

use tera::{Kwargs, State, TeraResult};

use crate::core::machine::{Fired, Machines};

/// The machines every template shares.
///
/// A `RwLock` rather than a `OnceLock`, unlike the persistence store beside it:
/// machines arrive when a collection declaring them is read, and a hot reload
/// replaces them. A store that could only ever be set once would pin whichever
/// collection happened to load first.
static MACHINES: OnceLock<RwLock<Arc<Machines>>> = OnceLock::new();

fn machines() -> &'static RwLock<Arc<Machines>> {
    MACHINES.get_or_init(|| RwLock::new(Arc::new(Machines::new([]))))
}

/// Install the machines templates read. Replaces whatever was there.
pub fn set_global_machines(declared: Arc<Machines>) {
    if let Ok(mut held) = machines().write() {
        *held = declared;
    }
}

#[must_use]
pub fn get_global_machines() -> Arc<Machines> {
    machines()
        .read()
        .map_or_else(|_| Arc::new(Machines::new([])), |held| Arc::clone(&held))
}

fn instance(kwargs: &Kwargs) -> TeraResult<(String, String)> {
    let machine = kwargs.must_get::<&str>("machine")?.to_string();
    // Keyless is a machine with one instance, which is what a rate limiter or
    // a circuit breaker is: state about the service, not about a resource.
    let key = kwargs
        .get::<&str>("key")?
        .map_or_else(String::new, ToString::to_string);
    Ok((machine, key))
}

fn unknown(machine: &str) -> tera::Error {
    let known = get_global_machines()
        .names()
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    tera::Error::message(if known.is_empty() {
        format!("no machine `{machine}` — nothing declares a `machines:` block")
    } else {
        format!("no machine `{machine}` — this collection declares: {known}")
    })
}

pub fn register_all_functions(tera: &mut tera::Tera) {
    // machine_state(machine, key=None) — where an instance is, without moving it
    tera.register_function(
        "machine_state",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<String> {
            let (machine, key) = instance(&kwargs)?;
            get_global_machines()
                .state_of(&machine, &key)
                .map(|state| state.to_string())
                .ok_or_else(|| unknown(&machine))
        },
    );

    // machine_can(machine, event, key=None) — whether the edge exists from here
    tera.register_function(
        "machine_can",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<bool> {
            let (machine, key) = instance(&kwargs)?;
            let event = kwargs.must_get::<&str>("event")?;
            let machines = get_global_machines();
            let declared = machines.get(&machine).ok_or_else(|| unknown(&machine))?;
            let at = machines
                .state_of(&machine, &key)
                .ok_or_else(|| unknown(&machine))?;
            Ok(declared.edge(at.as_str(), event).is_some())
        },
    );

    // machine_fire(machine, event, key=None) — move, and answer where it landed
    tera.register_function(
        "machine_fire",
        |kwargs: Kwargs, _: &State<'_>| -> TeraResult<String> {
            let (machine, key) = instance(&kwargs)?;
            let event = kwargs.must_get::<&str>("event")?;
            match get_global_machines().fire(&machine, &key, event, |_| true) {
                Fired::Moved { to, .. } => Ok(to.to_string()),
                // A refused move is the mock's answer, not a render failure: a
                // template branching on it is how a 409 gets written.
                Fired::NoEdge { from } => Err(tera::Error::message(format!(
                    "`{machine}` has no `{event}` out of `{from}`"
                ))),
                Fired::Refused { from, guard } => Err(tera::Error::message(format!(
                    "`{machine}` refused `{event}` out of `{from}`: guard `{guard}`"
                ))),
                Fired::NoSuchMachine => Err(unknown(&machine)),
            }
        },
    );

    // machine_reset() — put every instance back, for a test that shares a process
    tera.register_function(
        "machine_reset",
        |_: Kwargs, _: &State<'_>| -> TeraResult<String> {
            get_global_machines().reset();
            Ok(String::new())
        },
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::core::machine::{Edge, Machine, State as MachineState};
    use lean_string::LeanString;

    fn install() {
        let state = |name: &str, on: &[(&str, &str)]| MachineState {
            name: LeanString::from(name),
            weight: 1.0,
            empty: Vec::new(),
            on: on
                .iter()
                .map(|(event, target)| Edge {
                    event: LeanString::from(*event),
                    target: LeanString::from(*target),
                    guard: None,
                })
                .collect(),
        };
        set_global_machines(Arc::new(Machines::new([(
            LeanString::from("order"),
            Machine::new(vec![
                state("created", &[("pay", "paid")]),
                state("paid", &[("ship", "shipped")]),
                state("shipped", &[]),
            ]),
        )])));
    }

    fn render(body: &str) -> String {
        let mut tera = tera::Tera::default();
        super::register_all_functions(&mut tera);
        tera.add_raw_template("t", body).expect("template");
        tera.render("t", &tera::Context::new()).expect("render")
    }

    /// Reading is not moving. Every poll-counter scenario gets this wrong, and
    /// a `GET` that advances a lifecycle is a mock lying about a safe method.
    #[test]
    fn reading_a_machine_does_not_move_it_and_firing_does() {
        install();
        get_global_machines().reset();

        assert_eq!(
            render(r#"{{ machine_state(machine="order", key="7") }}"#),
            "created"
        );
        assert_eq!(
            render(r#"{{ machine_state(machine="order", key="7") }}"#),
            "created",
            "reading twice is still where it started"
        );

        assert_eq!(
            render(r#"{{ machine_fire(machine="order", key="7", event="pay") }}"#),
            "paid"
        );
        assert_eq!(
            render(r#"{{ machine_state(machine="order", key="7") }}"#),
            "paid"
        );
        // And a different key is a different instance.
        assert_eq!(
            render(r#"{{ machine_state(machine="order", key="8") }}"#),
            "created"
        );
    }

    #[test]
    fn an_edge_that_does_not_exist_says_so_and_names_what_does() {
        install();
        get_global_machines().reset();

        let mut tera = tera::Tera::default();
        super::register_all_functions(&mut tera);
        tera.add_raw_template(
            "t",
            r#"{{ machine_fire(machine="order", key="1", event="deliver") }}"#,
        )
        .expect("template");
        let failed = tera
            .render("t", &tera::Context::new())
            .expect_err("no `deliver` out of `created`");
        assert!(
            format!("{failed:?}").contains("deliver"),
            "the error names the event: {failed:?}"
        );

        assert_eq!(
            render(r#"{{ machine_can(machine="order", key="1", event="pay") }}"#),
            "true"
        );
        assert_eq!(
            render(r#"{{ machine_can(machine="order", key="1", event="deliver") }}"#),
            "false"
        );
    }

    /// A machine nobody declared is a typo, and the error is worth more if it
    /// says what was declared instead.
    #[test]
    fn an_undeclared_machine_names_the_ones_that_exist() {
        install();
        let mut tera = tera::Tera::default();
        super::register_all_functions(&mut tera);
        tera.add_raw_template("t", r#"{{ machine_state(machine="nosuch") }}"#)
            .expect("template");
        let failed = tera
            .render("t", &tera::Context::new())
            .expect_err("`nosuch` is not declared");
        let shown = format!("{failed:?}");
        assert!(shown.contains("nosuch"), "{shown}");
        assert!(shown.contains("order"), "it names what does exist: {shown}");
    }
}
