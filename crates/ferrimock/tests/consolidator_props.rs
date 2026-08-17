//! Properties consolidation must hold over generated traffic.
//!
//! The scenario tests in `consolidator_fidelity.rs` cover cases someone thought
//! of. These cover the ones nobody did: a grammar generates a small synthetic
//! API -- resources, id shapes, response structures, statuses -- and every
//! recording it produces is put through consolidation and replayed.
//!
//! The invariants are deliberately behavioural rather than structural. How the
//! engine chooses to group and template is its business; that every recorded
//! request still gets its own answer back is not.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Utc;
use ferrimock::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use ferrimock::consolidator::{
    ConsolidatorOptions, FidelityOptions, FidelityReport, MockConsolidator,
};
use ferrimock::recorder::{RecordedInteraction, RecordedRequest, RecordedResponse};
use proptest::prelude::*;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::sync::OnceLock;
use std::time::Duration;

// ---------------------------------------------------------------------------
// A small synthetic API
// ---------------------------------------------------------------------------

/// How a handler's instances are addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IdKind {
    Numeric,
    Uuid,
    Slug,
}

impl IdKind {
    fn render(self, instance: u8) -> String {
        match self {
            Self::Numeric => (1000 + u32::from(instance)).to_string(),
            // A stable v4-shaped UUID: only the tail varies, which is enough to
            // make instances distinct without a random source.
            Self::Uuid => format!("550e8400-e29b-41d4-a716-{instance:012x}"),
            Self::Slug => format!("item-{instance}"),
        }
    }
}

/// One field in a handler's response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldKind {
    /// Echoes the instance id.
    Id,
    /// A string that differs per instance.
    VaryingString,
    /// A number that differs per instance.
    VaryingNumber,
    /// The same value in every response.
    Constant,
    /// A nested object carrying both a constant and a varying field.
    Nested,
    /// An array of like-shaped objects, of a length that varies per instance.
    List,
    /// The same array in every response.
    ConstantList,
    /// An array whose elements all share one discriminator value.
    UniformList,
    /// The same nested object in every response.
    ConstantNested,
    /// A field that is null in every response.
    Null,
}

impl FieldKind {
    fn render(self, id: &str, instance: u8) -> JsonValue {
        match self {
            Self::Id => JsonValue::String(id.to_string()),
            Self::VaryingString => JsonValue::String(format!("value-{instance}")),
            Self::VaryingNumber => JsonValue::from(u32::from(instance) * 7 + 3),
            Self::Constant => JsonValue::String("fixed".to_string()),
            Self::Nested => {
                let mut nested = JsonMap::new();
                nested.insert("kind".to_string(), JsonValue::String("inner".to_string()));
                nested.insert("seq".to_string(), JsonValue::from(u32::from(instance)));
                JsonValue::Object(nested)
            }
            Self::List => {
                let items: Vec<JsonValue> = (0..=u32::from(instance) % 3)
                    .map(|n| {
                        let mut item = JsonMap::new();
                        item.insert("id".to_string(), JsonValue::from(n));
                        item.insert("label".to_string(), JsonValue::String(format!("l{n}")));
                        JsonValue::Object(item)
                    })
                    .collect();
                JsonValue::Array(items)
            }
            Self::ConstantList => serde_json::json!([
                {"code": "alpha", "rank": 1},
                {"code": "beta", "rank": 2}
            ]),
            Self::UniformList => {
                // Every element agrees on `kind`; only `n` moves. A template that
                // invents `kind` has changed what the endpoint says about itself.
                let items: Vec<JsonValue> = (0..=u32::from(instance) % 3)
                    .map(|n| {
                        let mut item = JsonMap::new();
                        item.insert("kind".to_string(), JsonValue::String("entry".to_string()));
                        item.insert("n".to_string(), JsonValue::from(n));
                        JsonValue::Object(item)
                    })
                    .collect();
                JsonValue::Array(items)
            }
            Self::ConstantNested => serde_json::json!({"scheme": "v1", "locked": true}),
            Self::Null => JsonValue::Null,
        }
    }
}

/// The kind of endpoint a handler is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `GET /v2/{resource}/{id}` -- one instance per id.
    Detail,
    /// `GET /v2/{resource}?offset=N&limit=2` -- a page of a collection.
    Paginated,
    /// `POST /v2/{resource}` -- distinguishable only by its request body.
    Search,
}

/// One endpoint of the synthetic API.
#[derive(Debug, Clone)]
struct Handler {
    resource: String,
    shape: Shape,
    id_kind: IdKind,
    status: u16,
    fields: Vec<(String, FieldKind)>,
}

/// Total entries the synthetic collections claim to hold.
const COLLECTION_SIZE: u32 = 6;
/// Entries a page of a synthetic collection returns.
const PAGE_SIZE: u32 = 2;

impl Handler {
    fn method(&self) -> &'static str {
        match self.shape {
            Shape::Detail | Shape::Paginated => "GET",
            Shape::Search => "POST",
        }
    }

    fn uri(&self, instance: u8) -> String {
        match self.shape {
            Shape::Detail => {
                format!("/v2/{}/{}", self.resource, self.id_kind.render(instance))
            }
            Shape::Paginated | Shape::Search => format!("/v2/{}", self.resource),
        }
    }

    fn query(&self, instance: u8) -> Option<String> {
        match self.shape {
            Shape::Paginated => Some(format!(
                "offset={}&limit={PAGE_SIZE}",
                u32::from(instance) % 3 * PAGE_SIZE
            )),
            Shape::Detail | Shape::Search => None,
        }
    }

    fn request_body(&self, instance: u8) -> Option<String> {
        match self.shape {
            Shape::Search => {
                Some(serde_json::json!({ "query": format!("term-{instance}") }).to_string())
            }
            Shape::Detail | Shape::Paginated => None,
        }
    }

    fn body(&self, instance: u8) -> String {
        let id = self.id_kind.render(instance);
        let mut body = JsonMap::new();

        if self.shape == Shape::Paginated {
            let offset = u32::from(instance) % 3 * PAGE_SIZE;
            body.insert("total".to_string(), JsonValue::from(COLLECTION_SIZE));
            body.insert("offset".to_string(), JsonValue::from(offset));
            body.insert("limit".to_string(), JsonValue::from(PAGE_SIZE));
            let entries: Vec<JsonValue> = (0..PAGE_SIZE)
                .map(|n| {
                    serde_json::json!({
                        "type": "entry",
                        "id": (offset + n).to_string(),
                    })
                })
                .collect();
            body.insert("items".to_string(), JsonValue::Array(entries));
        }

        for (name, kind) in &self.fields {
            body.insert(name.clone(), kind.render(&id, instance));
        }
        JsonValue::Object(body).to_string()
    }
}

/// A recording of a synthetic API being called.
#[derive(Debug, Clone)]
struct Traffic {
    handlers: Vec<Handler>,
    /// `(handler index, instance)` in the order they were called.
    calls: Vec<(usize, u8)>,
}

impl Traffic {
    fn interactions(&self) -> Vec<RecordedInteraction> {
        self.calls
            .iter()
            .enumerate()
            .filter_map(|(index, (handler, instance))| {
                let handler = self.handlers.get(*handler)?;
                Some(RecordedInteraction {
                    id: format!("i{index}"),
                    timestamp: Utc::now(),
                    request: RecordedRequest {
                        method: handler.method().to_string(),
                        uri: handler.uri(*instance),
                        query: handler.query(*instance),
                        headers: vec![("accept".to_string(), "application/json".to_string())],
                        body: handler.request_body(*instance),
                    },
                    response: RecordedResponse {
                        status: handler.status,
                        headers: vec![("content-type".to_string(), "application/json".to_string())],
                        body: handler.body(*instance),
                    },
                    duration: Duration::from_millis(3),
                })
            })
            .collect()
    }
}

fn arb_field_kind() -> impl Strategy<Value = FieldKind> {
    prop_oneof![
        Just(FieldKind::Id),
        Just(FieldKind::VaryingString),
        Just(FieldKind::VaryingNumber),
        Just(FieldKind::Constant),
        Just(FieldKind::Nested),
        Just(FieldKind::List),
        Just(FieldKind::ConstantList),
        Just(FieldKind::UniformList),
        Just(FieldKind::ConstantNested),
        Just(FieldKind::Null),
    ]
}

fn arb_handler(index: usize) -> impl Strategy<Value = Handler> {
    (
        prop_oneof![
            Just(Shape::Detail),
            Just(Shape::Paginated),
            Just(Shape::Search)
        ],
        prop_oneof![
            Just(IdKind::Numeric),
            Just(IdKind::Uuid),
            Just(IdKind::Slug)
        ],
        prop_oneof![Just(200u16), Just(201), Just(404), Just(500)],
        prop::collection::vec(arb_field_kind(), 1..5),
    )
        .prop_map(move |(shape, id_kind, status, kinds)| {
            let fields = kinds
                .into_iter()
                .enumerate()
                .map(|(position, kind)| {
                    // `id` keeps its conventional name so the engine's
                    // path-id binding is exercised; the rest are numbered so no
                    // two fields of one handler collide.
                    let name = if kind == FieldKind::Id {
                        "id".to_string()
                    } else {
                        format!("field_{position}")
                    };
                    (name, kind)
                })
                .collect::<Vec<_>>();
            Handler {
                // Resources are numbered, so two handlers never claim one path.
                resource: format!("res{index}"),
                shape,
                id_kind,
                status,
                fields,
            }
        })
}

fn arb_traffic() -> impl Strategy<Value = Traffic> {
    (1usize..4).prop_flat_map(|handler_count| {
        let handlers = (0..handler_count).map(arb_handler).collect::<Vec<_>>();
        (
            handlers,
            prop::collection::vec((0..handler_count, 0u8..8), 1..24),
        )
            .prop_map(|(handlers, calls)| Traffic { handlers, calls })
    })
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
    })
}

fn recorded_collection(interactions: &[RecordedInteraction]) -> MockCollectionConfig {
    let request_bodies: Vec<Option<String>> = interactions
        .iter()
        .map(|it| it.request.body.clone())
        .collect();
    let mut mocks: Vec<MockConfig> = interactions
        .iter()
        .enumerate()
        .map(|(index, it)| MockConfig {
            id: format!("rec-{}", index + 1).as_str().into(),
            description: None,
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            vars: None,
            match_config: Some(MatchConfig {
                methods: vec![it.request.method.clone()],
                urls: vec![match &it.request.query {
                    Some(query) => format!("{}?{}", it.request.uri, query),
                    None => it.request.uri.clone(),
                }],
                ..Default::default()
            }),
            request: None,
            response_config: Some(ReturnConfig::Structured {
                status: Some(it.response.status),
                headers: it.response.headers.iter().cloned().collect(),
                body: Some(it.response.body.clone()),
                template: None,
                file: None,
                template_file: None,
                json: Box::new(JsonValue::Null),
            }),
            patch: None,
            delay: None,
            network_error: None,
            sse: None,
            ws: None,
            serve: None,
        })
        .collect();

    // Same-URL requests that differed only in their body are indistinguishable
    // until something from the body is pinned -- which is what the real
    // recorders do before handing a collection to the consolidator.
    ferrimock::config::discriminate_by_request_body(&mut mocks, &request_bodies);

    MockCollectionConfig {
        name: Some("generated".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
        world: None,
    }
}

fn consolidate(interactions: &[RecordedInteraction]) -> (MockCollectionConfig, FidelityReport) {
    let original = recorded_collection(interactions);
    let mut consolidator = MockConsolidator::with_options(ConsolidatorOptions::default());
    runtime().block_on(async {
        consolidator
            .consolidate_verified(
                interactions,
                original,
                &FidelityOptions {
                    reset_persistence: true,
                    ..FidelityOptions::default()
                },
            )
            .await
            .expect("verified consolidation runs")
    })
}

fn describe(report: &FidelityReport) -> String {
    format!(
        "matched {}/{}, lineage {}, status {}, shape {}, constants {} | \
         unmatched {:?} cross-talk {:?} status {:?} shape {:?} constants {:?} render {:?}",
        report.score.matched,
        report.score.total,
        report.score.no_cross_talk,
        report.score.status_exact,
        report.score.shape_equal,
        report.score.constants_held,
        report.unmatched,
        report.cross_talk,
        report.status_mismatch,
        report.shape_mismatch,
        report.constant_drift,
        report.render_errors,
    )
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// The recording has to replay against its own unconsolidated mocks. If this
    /// fails the generator is producing traffic the recorder cannot express, and
    /// every number measured after it is meaningless.
    #[test]
    fn a_generated_recording_replays_against_itself(traffic in arb_traffic()) {
        let interactions = traffic.interactions();
        let (_, report) = consolidate(&interactions);

        prop_assert_eq!(
            report.baseline.behavioral,
            report.baseline.total,
            "the recording does not replay before consolidation: {}",
            describe(&report)
        );
    }

    /// Consolidation may merge, but it may never drop a request on the floor.
    #[test]
    fn every_recorded_request_still_gets_an_answer(traffic in arb_traffic()) {
        let interactions = traffic.interactions();
        let (_, report) = consolidate(&interactions);

        prop_assert_eq!(
            report.score.matched,
            report.score.total,
            "consolidation orphaned requests: {}",
            describe(&report)
        );
    }

    /// A request must be answered by a mock descended from its own recording.
    /// This is what an over-broad pattern breaks, and nothing else detects it.
    #[test]
    fn no_request_is_answered_by_a_foreign_lineage(traffic in arb_traffic()) {
        let interactions = traffic.interactions();
        let (_, report) = consolidate(&interactions);

        prop_assert_eq!(
            report.score.no_cross_talk,
            report.score.total,
            "cross-talk: {}",
            describe(&report)
        );
    }

    /// Merging responses must not change what any of them said.
    #[test]
    fn consolidation_preserves_recorded_behaviour(traffic in arb_traffic()) {
        let interactions = traffic.interactions();
        let (_, report) = consolidate(&interactions);

        prop_assert_eq!(
            report.score.behavioral,
            report.score.total,
            "consolidation changed behaviour: {}",
            describe(&report)
        );
    }

    /// Consolidation compresses. It must never produce more mocks than it was
    /// given.
    #[test]
    fn consolidation_never_grows_a_collection(traffic in arb_traffic()) {
        let interactions = traffic.interactions();
        let (consolidated, _) = consolidate(&interactions);

        prop_assert!(
            consolidated.mocks.len() <= interactions.len(),
            "{} recordings became {} mocks",
            interactions.len(),
            consolidated.mocks.len()
        );
    }

    /// Consolidating an already-consolidated collection must find nothing more
    /// to do. A second pass that keeps merging is a sign the first pass left
    /// the collection in a shape it does not recognise as its own output.
    #[test]
    fn consolidation_is_idempotent(traffic in arb_traffic()) {
        let interactions = traffic.interactions();
        let (once, _) = consolidate(&interactions);

        let mut consolidator = MockConsolidator::new();
        let twice = consolidator
            .consolidate(once.clone())
            .expect("second pass runs");

        prop_assert_eq!(
            twice.mocks.len(),
            once.mocks.len(),
            "a second pass changed {} mocks into {}",
            once.mocks.len(),
            twice.mocks.len()
        );
    }

    /// The order requests happened in must not change how many mocks come out.
    /// Recording order is an accident of the session, not a property of the API.
    #[test]
    fn the_order_of_a_recording_does_not_change_its_shape(traffic in arb_traffic()) {
        let mut interactions = traffic.interactions();
        let (forward, _) = consolidate(&interactions);

        interactions.reverse();
        let (backward, _) = consolidate(&interactions);

        prop_assert_eq!(
            forward.mocks.len(),
            backward.mocks.len(),
            "reversing the recording changed the mock count"
        );
    }
}
