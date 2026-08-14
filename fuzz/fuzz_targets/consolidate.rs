//! Consolidation must survive any collection, however strange.
//!
//! Recordings come from real traffic, and real traffic is not well behaved:
//! empty paths, duplicate ids, bodies that are JSON only by accident. None of it
//! may panic, hang, or make consolidation produce more mocks than it was given.

#![no_main]

use arbitrary::Arbitrary;
use ferrimock::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use ferrimock::consolidator::MockConsolidator;
use libfuzzer_sys::fuzz_target;
use rustc_hash::FxHashMap;
use serde_json::Value as JsonValue;

/// Deepest nesting a generated body reaches. Recursion here is driven by the
/// fuzzer's input, so it needs a hard stop that does not depend on that input
/// running out.
const MAX_DEPTH: u8 = 4;
/// Widest a generated collection gets, so a single case stays fast enough for
/// the fuzzer to make progress.
const MAX_MOCKS: usize = 24;

#[derive(Arbitrary, Debug)]
enum FuzzJson {
    Null,
    Bool(bool),
    Int(i32),
    Float(f32),
    Text(String),
    List(Vec<FuzzJson>),
    Map(Vec<(String, FuzzJson)>),
}

impl FuzzJson {
    fn to_json(&self, depth: u8) -> JsonValue {
        if depth >= MAX_DEPTH {
            return JsonValue::Null;
        }
        match self {
            Self::Null => JsonValue::Null,
            Self::Bool(value) => JsonValue::Bool(*value),
            Self::Int(value) => JsonValue::from(*value),
            Self::Float(value) => serde_json::Number::from_f64(f64::from(*value))
                .map_or(JsonValue::Null, JsonValue::Number),
            Self::Text(value) => JsonValue::String(value.clone()),
            Self::List(items) => JsonValue::Array(
                items
                    .iter()
                    .take(8)
                    .map(|item| item.to_json(depth + 1))
                    .collect(),
            ),
            Self::Map(fields) => JsonValue::Object(
                fields
                    .iter()
                    .take(8)
                    .map(|(key, value)| (key.clone(), value.to_json(depth + 1)))
                    .collect(),
            ),
        }
    }
}

#[derive(Arbitrary, Debug)]
enum FuzzMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl FuzzMethod {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Arbitrary, Debug)]
struct FuzzMock {
    method: FuzzMethod,
    url: String,
    status: u16,
    body: FuzzJson,
    priority: u16,
    enabled: bool,
    once: bool,
}

#[derive(Arbitrary, Debug)]
struct FuzzCollection {
    mocks: Vec<FuzzMock>,
}

impl FuzzCollection {
    fn into_collection(self) -> MockCollectionConfig {
        let mocks = self
            .mocks
            .into_iter()
            .take(MAX_MOCKS)
            .enumerate()
            .map(|(index, mock)| MockConfig {
                id: format!("fuzz-{index}").as_str().into(),
                description: None,
                priority: u32::from(mock.priority),
                enabled: mock.enabled,
                once: mock.once,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec![mock.method.as_str().to_string()],
                    urls: vec![mock.url],
                    ..Default::default()
                }),
                request: None,
                response_config: Some(ReturnConfig::Structured {
                    status: Some(mock.status),
                    headers: FxHashMap::default(),
                    body: Some(mock.body.to_json(0).to_string()),
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
            })
            .collect();

        MockCollectionConfig {
            name: None,
            description: None,
            enabled: true,
            vars: None,
            mocks,
        }
    }
}

fuzz_target!(|input: FuzzCollection| {
    let collection = input.into_collection();
    let original = collection.mocks.len();

    let mut consolidator = MockConsolidator::new();
    let Ok(consolidated) = consolidator.consolidate(collection) else {
        return;
    };

    assert!(
        consolidated.mocks.len() <= original,
        "consolidation grew {original} mocks into {}",
        consolidated.mocks.len()
    );

    // Every surviving mock must be traceable to what it stands in for,
    // otherwise nothing downstream can tell a merge from a mix-up.
    let provenance = consolidator.provenance();
    for mock in &consolidated.mocks {
        assert!(
            !provenance.origins(mock.id.as_str()).is_empty(),
            "mock {} has no lineage",
            mock.id
        );
    }
});
