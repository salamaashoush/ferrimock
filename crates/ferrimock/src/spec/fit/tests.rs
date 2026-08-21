#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::get_unwrap
)]

use super::*;
use crate::core::world::model::{
    CompositeKey, EntityType, FieldDef, Provenance, Rule, Scalar, ScalarKind, ValueSpec,
};
use crate::recorder::{RecordedRequest, RecordedResponse};

fn field(name: &str) -> FieldDef {
    FieldDef::new(
        name,
        ValueSpec::Scalar(Scalar::new(ScalarKind::String)),
        true,
    )
}

fn graph_of(name: &str, fields: &[&str]) -> EntityGraph {
    let mut entity = EntityType::new(
        name,
        CompositeKey::single("id"),
        Provenance::new(Rule::GraphQLSchema, name),
    );
    for held in fields {
        entity = entity.with_field(field(held));
    }
    let mut graph = EntityGraph::new();
    graph.insert(entity);
    graph
}

fn recorded(bodies: &[JsonValue]) -> Vec<RecordedInteraction> {
    bodies
        .iter()
        .enumerate()
        .map(|(at, body)| RecordedInteraction {
            id: at.to_string(),
            timestamp: chrono::Utc::now(),
            request: RecordedRequest {
                method: "GET".to_string(),
                uri: "/orders".to_string(),
                query: None,
                headers: Vec::new(),
                body: None,
            },
            response: RecordedResponse {
                status: 200,
                headers: Vec::new(),
                body: body.to_string(),
            },
            duration: std::time::Duration::from_millis(1),
        })
        .collect()
}

fn orders(count: usize) -> Vec<JsonValue> {
    let states = ["draft", "paid", "paid", "shipped", "delivered"];
    (0..count)
        .map(|at| {
            let state = states[at % states.len()];
            let mut order = serde_json::json!({
                "id": format!("o-{at}"),
                "status": state,
                "total": i64::try_from(at % 40).unwrap_or(0) + 5,
                "paid_at": JsonValue::Null,
                "shipped_at": JsonValue::Null,
                "delivered_at": JsonValue::Null,
            });
            let stamp = JsonValue::from("2026-01-01T00:00:00Z");
            if state != "draft" {
                order["paid_at"] = stamp.clone();
            }
            if state == "shipped" || state == "delivered" {
                order["shipped_at"] = stamp.clone();
            }
            if state == "delivered" {
                order["delivered_at"] = stamp;
            }
            order
        })
        .collect()
}

fn order_graph() -> EntityGraph {
    graph_of(
        "Order",
        &[
            "id",
            "status",
            "total",
            "paid_at",
            "shipped_at",
            "delivered_at",
        ],
    )
}

#[test]
fn a_recording_says_how_many_of_each_thing_there_are() {
    let bodies: Vec<JsonValue> = vec![JsonValue::Array(orders(30))];
    let held = fit(&order_graph(), &recorded(&bodies));

    assert_eq!(held.counts.get("Order"), Some(&30));
    assert_eq!(held.read, 1);
    assert_eq!(held.recognised, 30);
}

/// A record wrapped in an envelope is still a record.
#[test]
fn a_record_is_found_however_the_response_wrapped_it() {
    let bodies = vec![serde_json::json!({
        "entries": orders(20),
        "total_count": 20,
        "limit": 20,
    })];
    let held = fit(&order_graph(), &recorded(&bodies));
    assert_eq!(held.counts.get("Order"), Some(&20));
}

/// The weights are the recording's. `one_of` picks uniformly without
/// deduplicating, so a weight is a repeated value — which is how a
/// hand-written override already expresses one.
#[test]
fn a_closed_set_comes_back_weighted_the_way_it_was_seen() {
    let bodies = vec![JsonValue::Array(orders(50))];
    let held = fit(&order_graph(), &recorded(&bodies));

    // `status` turned out to be a lifecycle, so it is emitted as one; `total`
    // is a range.
    assert!(matches!(
        held.fields.get("Order.total"),
        Some(Fitted::Int { min: 5, .. })
    ));

    let states = held.states.get("Order.status").expect("a lifecycle");
    let named: Vec<&str> = states.iter().map(|state| state.name.as_str()).collect();
    assert_eq!(named, ["draft", "paid", "shipped", "delivered"]);

    let paid = states.iter().find(|state| state.name == "paid").unwrap();
    let draft = states.iter().find(|state| state.name == "draft").unwrap();
    assert!(
        paid.weight > draft.weight,
        "the weights are the recording's: {states:?}"
    );
}

/// The evidence for a lifecycle is other fields going empty: every record
/// whose status is `draft` has no `shipped_at`, and that is what `draft`
/// *means* rather than something the recording happened to show.
#[test]
fn a_status_that_empties_the_record_is_read_as_a_lifecycle() {
    let bodies = vec![JsonValue::Array(orders(50))];
    let held = fit(&order_graph(), &recorded(&bodies));
    let states = held.states.get("Order.status").unwrap();

    let empty_of = |name: &str| {
        states
            .iter()
            .find(|state| state.name == name)
            .map(|state| state.empty.clone())
            .unwrap()
    };
    assert_eq!(empty_of("draft"), ["paid_at", "shipped_at", "delivered_at"]);
    assert_eq!(empty_of("paid"), ["shipped_at", "delivered_at"]);
    assert_eq!(empty_of("shipped"), ["delivered_at"]);
    assert!(empty_of("delivered").is_empty());
}

#[test]
fn a_set_of_words_that_implies_nothing_stays_a_set_of_words() {
    let bodies = vec![JsonValue::Array(
        (0..40)
            .map(|at| {
                let colour = ["red", "red", "red", "blue"][at % 4];
                serde_json::json!({ "id": format!("t-{at}"), "colour": colour })
            })
            .collect::<Vec<_>>(),
    )];
    let held = fit(&graph_of("Tag", &["id", "colour"]), &recorded(&bodies));

    assert!(held.states.is_empty(), "nothing was implied");
    let Some(Fitted::OneOf(values)) = held.fields.get("Tag.colour") else {
        panic!("a closed set: {:?}", held.fields)
    };
    let reds = values.iter().filter(|value| *value == "red").count();
    let blues = values.iter().filter(|value| *value == "blue").count();
    assert!(reds > blues * 2, "{values:?}");
}

#[test]
fn what_was_missing_is_measured_even_where_no_override_can_say_it() {
    let bodies = vec![JsonValue::Array(orders(50))];
    let held = fit(&order_graph(), &recorded(&bodies));
    let (absent, nulled) = held.missing.get("Order.delivered_at").copied().unwrap();
    assert_eq!(absent, 0);
    assert!(nulled > 0, "most orders have not been delivered");
}

/// The output is an ordinary overrides file, applied through the same
/// `FieldRules` a hand-written one is — not a private path back into the
/// store, which would be a second configuration surface.
#[test]
fn the_output_is_a_world_block_a_collection_can_carry() {
    #[derive(serde::Deserialize)]
    struct Carried {
        world: crate::config::WorldConfig,
    }

    let bodies = vec![JsonValue::Array(orders(50))];
    let held = fit(&order_graph(), &recorded(&bodies));
    let written = to_yaml(&held);

    let carried: Carried =
        serde_yaml_ng::from_str(&written).unwrap_or_else(|e| panic!("{e}: {written}"));
    assert_eq!(
        carried
            .world
            .counts
            .as_ref()
            .and_then(|counts| counts.get("Order")),
        Some(&50)
    );

    let states = carried
        .world
        .states
        .as_ref()
        .and_then(|states| states.get("Order.status"))
        .expect("the lifecycle survives the round trip");
    assert_eq!(states.len(), 4);
    assert_eq!(states[0].name, "draft");
    assert_eq!(states[0].empty, ["paid_at", "shipped_at", "delivered_at"]);

    // And it is a rule the graph will actually take.
    assert!(carried.world.field_rules().is_ok());
}

#[test]
fn an_object_that_is_not_an_entity_is_left_alone() {
    let bodies = vec![serde_json::json!({ "meta": { "page": 1, "of": 4 } })];
    let held = fit(&order_graph(), &recorded(&bodies));
    assert!(held.counts.is_empty());
    assert_eq!(held.recognised, 0);
}
