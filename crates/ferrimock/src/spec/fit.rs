//! Fitting a world's parameters to a recording.
//!
//! Realism is agreement with an empirical distribution, so the highest-fidelity
//! world is one whose parameters were *measured* rather than guessed. Every
//! default in the value layer is a defensible prior and none of them knows what
//! this API's `status` field actually holds, how many folders a real account
//! has, or which of a schema's four states carries most of the mass.
//!
//! What comes out is an ordinary overrides file: reviewable, diffable,
//! committable, and applied through the same `FieldRules` a hand-written one
//! is. Nothing here reaches into the store — a fit that could only be applied
//! by a private path would be a second configuration surface.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as JsonValue;

use crate::core::world::model::{EntityGraph, EntityType};
use crate::recorder::RecordedInteraction;

/// How much of an entity's field set an object has to carry before it is
/// taken to *be* one.
const RECOGNISABLE: f64 = 0.6;

/// A field with more distinct values than this is not a closed set.
const CLOSED_SET: usize = 12;

/// How many observations a claim about a field needs.
const ENOUGH: usize = 8;

/// How many slots a fitted weighting is written across.
///
/// `one_of` picks uniformly over its vector without deduplicating, so a weight
/// is written by repeating a value — which is how a hand-written override
/// already expresses one, and which keeps the output a thing a person can read
/// and change.
const SLOTS: usize = 20;

/// What a recording says about one field.
#[derive(Debug, Clone, PartialEq)]
pub enum Fitted {
    /// A closed set, weighted the way the recording weighted it.
    OneOf(Vec<String>),
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
    },
}

/// One state of a lifecycle a recording revealed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FittedState {
    pub name: String,
    pub weight: usize,
    pub empty: Vec<String>,
}

/// Everything a recording says about a world.
#[derive(Debug, Clone, Default)]
pub struct Fit {
    /// Distinct instances of each entity the recording showed.
    pub counts: BTreeMap<String, usize>,
    /// `Entity.field` to what it holds.
    pub fields: BTreeMap<String, Fitted>,
    /// `Entity.field` to the lifecycle it turned out to be.
    pub states: BTreeMap<String, Vec<FittedState>>,
    /// How often each nullable-looking field was absent or null, which no
    /// override can express yet and which is reported rather than emitted.
    pub missing: BTreeMap<String, (usize, usize)>,
    /// Responses read, and objects recognised as an entity.
    pub read: usize,
    pub recognised: usize,
}

/// What one entity's records looked like across a recording.
#[derive(Debug, Default)]
struct Seen {
    keys: BTreeSet<String>,
    records: Vec<serde_json::Map<String, JsonValue>>,
}

/// Read every response and say what the world would have to be to have
/// produced it.
#[must_use]
pub fn fit(graph: &EntityGraph, interactions: &[RecordedInteraction]) -> Fit {
    let mut fit = Fit::default();
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();

    for interaction in interactions {
        let Ok(body) = serde_json::from_str::<JsonValue>(&interaction.response.body) else {
            continue;
        };
        fit.read += 1;
        for object in objects_in(&body) {
            let Some(entity) = recognise(graph, object) else {
                continue;
            };
            fit.recognised += 1;
            let held = seen.entry(entity.name.to_string()).or_default();
            if let Some(key) = key_of(entity, object) {
                held.keys.insert(key);
            }
            held.records.push(object.clone());
        }
    }

    for (name, held) in seen {
        if !held.keys.is_empty() {
            fit.counts.insert(name.clone(), held.keys.len());
        }
        let Some(entity) = graph.get(&name) else {
            continue;
        };
        fit_fields(entity, &held.records, &mut fit);
    }
    fit
}

/// Every JSON object a response carries, however it wrapped them.
fn objects_in(body: &JsonValue) -> Vec<&serde_json::Map<String, JsonValue>> {
    let mut found = Vec::new();
    let mut pending = vec![body];
    while let Some(value) = pending.pop() {
        match value {
            JsonValue::Object(object) => {
                found.push(object);
                // An envelope holds the records rather than being one, and a
                // record holds embedded objects; both are worth looking into.
                pending.extend(object.values());
            }
            JsonValue::Array(items) => pending.extend(items.iter()),
            _ => {}
        }
    }
    found
}

/// Which entity an object is, if it is one.
fn recognise<'a>(
    graph: &'a EntityGraph,
    object: &serde_json::Map<String, JsonValue>,
) -> Option<&'a EntityType> {
    graph
        .entities()
        .filter(|entity| {
            entity
                .key
                .iter()
                .all(|part| object.contains_key(part.field.as_str()))
        })
        .filter_map(|entity| {
            let declared = entity.fields.len();
            if declared == 0 {
                return None;
            }
            let held = entity
                .fields
                .iter()
                .filter(|field| object.contains_key(field.name.as_str()))
                .count();
            #[allow(
                clippy::cast_precision_loss,
                reason = "a schema's field count, far below the f64 mantissa"
            )]
            let share = held as f64 / declared as f64;
            (share >= RECOGNISABLE).then_some((entity, held))
        })
        // The most specific reading wins: two entities sharing `id` and `name`
        // are told apart by whichever one the object carries more of.
        .max_by_key(|(_, held)| *held)
        .map(|(entity, _)| entity)
}

fn key_of(entity: &EntityType, object: &serde_json::Map<String, JsonValue>) -> Option<String> {
    let parts: Vec<String> = entity
        .key
        .iter()
        .filter_map(|part| object.get(part.field.as_str()))
        .map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_string)
        })
        .collect();
    (parts.len() == entity.key.len()).then(|| parts.join("/"))
}

fn fit_fields(entity: &EntityType, records: &[serde_json::Map<String, JsonValue>], fit: &mut Fit) {
    for field in entity.value_fields() {
        let name = field.name.as_str();
        let target = format!("{}.{name}", entity.name);
        let held: Vec<&JsonValue> = records
            .iter()
            .filter_map(|record| record.get(name))
            .collect();

        let missing = records.len() - held.len();
        let nulled = held.iter().filter(|value| value.is_null()).count();
        if missing > 0 || nulled > 0 {
            fit.missing.insert(target.clone(), (missing, nulled));
        }

        let present: Vec<&JsonValue> = held.into_iter().filter(|value| !value.is_null()).collect();
        if present.len() < ENOUGH {
            continue;
        }
        if let Some(fitted) = fit_field(&present) {
            fit.fields.insert(target.clone(), fitted);
        }
        if let Some(states) = fit_states(entity, records, name) {
            fit.states.insert(target, states);
        }
    }
}

fn fit_field(present: &[&JsonValue]) -> Option<Fitted> {
    if present.iter().all(|value| value.is_i64()) {
        let numbers: Vec<i64> = present.iter().filter_map(|value| value.as_i64()).collect();
        return Some(Fitted::Int {
            min: numbers.iter().copied().min()?,
            max: numbers.iter().copied().max()?,
        });
    }
    if present.iter().all(|value| value.is_number()) {
        let numbers: Vec<f64> = present.iter().filter_map(|value| value.as_f64()).collect();
        return Some(Fitted::Float {
            min: numbers.iter().copied().fold(f64::MAX, f64::min),
            max: numbers.iter().copied().fold(f64::MIN, f64::max),
        });
    }

    let texts: Vec<&str> = present.iter().filter_map(|value| value.as_str()).collect();
    if texts.len() != present.len() {
        return None;
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for text in &texts {
        *counts.entry(text).or_insert(0) += 1;
    }
    if counts.len() > CLOSED_SET || counts.len() < 2 {
        return None;
    }
    Some(Fitted::OneOf(weighted(&counts, texts.len())))
}

/// A closed set written out with each member repeated in proportion to how
/// often the recording held it.
fn weighted(counts: &BTreeMap<&str, usize>, total: usize) -> Vec<String> {
    let mut written = Vec::new();
    for (value, held) in counts {
        let slots = (held * SLOTS).div_ceil(total.max(1)).max(1);
        for _ in 0..slots {
            written.push((*value).to_string());
        }
    }
    written
}

/// Whether a field turned out to be a lifecycle rather than a set of words.
///
/// The evidence is other fields going empty: if every record whose `status` is
/// `draft` has no `shipped_at`, that is not a correlation the recording
/// happened to show, it is what `draft` means. The order comes out of the same
/// evidence — a state that empties more of the record is earlier in the life
/// of one.
fn fit_states(
    entity: &EntityType,
    records: &[serde_json::Map<String, JsonValue>],
    field: &str,
) -> Option<Vec<FittedState>> {
    let siblings: Vec<&str> = entity
        .value_fields()
        .map(|held| held.name.as_str())
        .filter(|held| *held != field)
        .collect();

    let mut by_state: BTreeMap<&str, Vec<&serde_json::Map<String, JsonValue>>> = BTreeMap::new();
    for record in records {
        let Some(state) = record.get(field).and_then(JsonValue::as_str) else {
            continue;
        };
        by_state.entry(state).or_default().push(record);
    }
    if by_state.len() < 2 || by_state.len() > CLOSED_SET {
        return None;
    }

    let mut states: Vec<FittedState> = by_state
        .iter()
        .map(|(state, held)| FittedState {
            name: (*state).to_string(),
            weight: held.len(),
            empty: siblings
                .iter()
                .filter(|sibling| {
                    held.iter()
                        .all(|record| record.get(**sibling).is_none_or(serde_json::Value::is_null))
                })
                .map(|sibling| (*sibling).to_string())
                .collect(),
        })
        .collect();

    // Nothing empties: an ordinary set of words, already fitted as `one_of`.
    if states.iter().all(|state| state.empty.is_empty()) {
        return None;
    }
    states.sort_by(|a, b| {
        b.empty
            .len()
            .cmp(&a.empty.len())
            .then_with(|| a.name.cmp(&b.name))
    });
    Some(states)
}

/// The fit as a `world:` block a collection can carry.
#[must_use]
pub fn to_yaml(fit: &Fit) -> String {
    use std::fmt::Write as _;

    let mut written = String::from("# Fitted from a recording by `ferrimock world fit`.\n");
    written.push_str("# Every value here was measured; edit anything that reads wrong.\nworld:\n");

    if !fit.counts.is_empty() {
        written.push_str("  counts:\n");
        for (entity, count) in &fit.counts {
            let _ = writeln!(written, "    {entity}: {count}");
        }
    }

    let fitted: Vec<(&String, &Fitted)> = fit
        .fields
        .iter()
        .filter(|(target, _)| !fit.states.contains_key(*target))
        .collect();
    if !fitted.is_empty() {
        written.push_str("  fields:\n");
        for (target, held) in fitted {
            match held {
                Fitted::OneOf(values) => {
                    let quoted: Vec<String> =
                        values.iter().map(|value| format!("{value:?}")).collect();
                    let _ = writeln!(
                        written,
                        "    {target}: {{ one_of: [{}] }}",
                        quoted.join(", ")
                    );
                }
                Fitted::Int { min, max } => {
                    let _ = writeln!(
                        written,
                        "    {target}: {{ int: {{ min: {min}, max: {max} }} }}"
                    );
                }
                Fitted::Float { min, max } => {
                    let _ = writeln!(
                        written,
                        "    {target}: {{ float: {{ min: {min}, max: {max} }} }}"
                    );
                }
            }
        }
    }

    if !fit.states.is_empty() {
        written.push_str("  states:\n");
        for (target, states) in &fit.states {
            let _ = writeln!(written, "    {target}:");
            for state in states {
                let empty = if state.empty.is_empty() {
                    String::new()
                } else {
                    format!(", empty: [{}]", state.empty.join(", "))
                };
                let _ = writeln!(
                    written,
                    "      - {{ name: {}, weight: {}{empty} }}",
                    state.name, state.weight
                );
            }
        }
    }
    written
}

#[cfg(test)]
mod tests;
