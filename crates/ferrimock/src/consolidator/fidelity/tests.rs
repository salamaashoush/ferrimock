use super::*;
use crate::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use crate::consolidator::MockConsolidator;
use chrono::Utc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn interaction(
    id: &str,
    method: &str,
    uri: &str,
    query: Option<&str>,
    status: u16,
    body: &str,
) -> RecordedInteraction {
    RecordedInteraction {
        id: id.to_string(),
        timestamp: Utc::now(),
        request: RecordedRequest {
            method: method.to_string(),
            uri: uri.to_string(),
            query: query.map(str::to_string),
            headers: vec![("accept".to_string(), "application/json".to_string())],
            body: None,
        },
        response: crate::recorder::RecordedResponse {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.to_string(),
        },
        duration: Duration::from_millis(5),
    }
}

fn get(id: &str, uri: &str, body: &str) -> RecordedInteraction {
    interaction(id, "GET", uri, None, 200, body)
}

/// Mirrors what `MockRecorder` writes out: one exact-URL mock per interaction.
fn recorded_collection(interactions: &[RecordedInteraction]) -> MockCollectionConfig {
    let mocks = interactions
        .iter()
        .enumerate()
        .map(|(index, it)| {
            let url = match &it.request.query {
                Some(query) => format!("{}?{}", it.request.uri, query),
                None => it.request.uri.clone(),
            };
            MockConfig {
                id: format!("rec-{}", index + 1).as_str().into(),
                description: None,
                priority: 100,
                enabled: true,
                once: false,
                scope: None,
                vars: None,
                match_config: Some(MatchConfig {
                    methods: vec![it.request.method.clone()],
                    urls: vec![url],
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
                    json: Box::new(serde_json::Value::Null),
                }),
                patch: None,
                delay: None,
                network_error: None,
                sse: None,
                ws: None,
            }
        })
        .collect();

    MockCollectionConfig {
        name: Some("test recording".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    }
}

fn options() -> FidelityOptions {
    FidelityOptions {
        reset_persistence: true,
        ..FidelityOptions::default()
    }
}

// ---------------------------------------------------------------------------
// Baseline: the recording must replay against its own unconsolidated collection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unconsolidated_recording_replays_exactly() {
    let interactions = vec![
        get("i1", "/api/users/1", r#"{"id":1,"name":"Ann","role":"admin"}"#),
        get("i2", "/api/users/2", r#"{"id":2,"name":"Bob","role":"admin"}"#),
        get("i3", "/api/users/3", r#"{"id":3,"name":"Cid","role":"admin"}"#),
    ];
    let collection = recorded_collection(&interactions);

    let mut provenance = Provenance::new();
    for mock in &collection.mocks {
        provenance.record_identity(mock.id.clone());
    }

    let report = verify(
        &interactions,
        &collection,
        &collection,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(report.score.total, 3);
    assert_eq!(report.score.matched, 3, "unmatched: {:?}", report.unmatched);
    assert_eq!(report.score.value_equal, 3);
    assert_eq!(report.score.behavioral, 3);
    assert!(report.baseline_unmatched.is_empty());
    assert!((report.score.behavioral_ratio() - 1.0).abs() < f64::EPSILON);
}

// ---------------------------------------------------------------------------
// The measurement consolidation never had
// ---------------------------------------------------------------------------

#[tokio::test]
async fn consolidating_a_uniform_group_preserves_behaviour() {
    let interactions = vec![
        get("i1", "/api/users/1", r#"{"id":1,"name":"Ann","role":"admin"}"#),
        get("i2", "/api/users/2", r#"{"id":2,"name":"Bob","role":"admin"}"#),
        get("i3", "/api/users/3", r#"{"id":3,"name":"Cid","role":"admin"}"#),
    ];
    let original = recorded_collection(&interactions);

    let mut consolidator = MockConsolidator::new();
    let (consolidated, report) = consolidator
        .consolidate_verified(&interactions, original, &options())
        .await
        .expect("verified consolidation runs");

    assert!(
        consolidated.mocks.len() < 3,
        "three same-shaped users should collapse, got {} mocks",
        consolidated.mocks.len()
    );
    assert_eq!(
        report.score.matched, 3,
        "every recorded request must still be answerable; unmatched: {:?}",
        report.unmatched
    );
    assert_eq!(
        report.score.no_cross_talk, 3,
        "requests answered from a foreign lineage: {:?}",
        report.cross_talk
    );
    assert_eq!(
        report.score.status_exact, 3,
        "status divergences: {:?}",
        report.status_mismatch
    );
    assert_eq!(
        report.score.shape_equal, 3,
        "shape divergences: {:?}",
        report.shape_mismatch
    );
    assert_eq!(
        report.score.constants_held, 3,
        "constant drift: {:?}",
        report.constant_drift
    );
    assert!(report.render_errors.is_empty());
}

#[tokio::test]
async fn the_baseline_separates_recorder_faults_from_consolidator_faults() {
    let interactions = vec![
        get("i1", "/api/items/1", r#"{"id":1,"kind":"crate"}"#),
        get("i2", "/api/items/2", r#"{"id":2,"kind":"crate"}"#),
        get("i3", "/api/items/3", r#"{"id":3,"kind":"crate"}"#),
    ];
    let original = recorded_collection(&interactions);

    let mut consolidator = MockConsolidator::new();
    let (_, report) = consolidator
        .consolidate_verified(&interactions, original, &options())
        .await
        .expect("verified consolidation runs");

    assert_eq!(
        report.baseline.behavioral, 3,
        "the unconsolidated recording must replay perfectly, else the delta is meaningless"
    );
    assert!(
        report.behavioral_delta() <= 0.0,
        "consolidation cannot score above its own input"
    );
}

// ---------------------------------------------------------------------------
// Each level fails on its own
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_request_nothing_answers_is_reported_unmatched() {
    let interactions = vec![get("i1", "/api/users/1", r#"{"id":1}"#)];
    let original = recorded_collection(&interactions);
    let empty = MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![],
    };

    let report = verify(
        &interactions,
        &original,
        &empty,
        &Provenance::new(),
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(report.score.matched, 0);
    assert_eq!(report.score.behavioral, 0);
    assert_eq!(report.unmatched.len(), 1);
    assert_eq!(report.unmatched[0].target, "/api/users/1");
    assert_eq!(report.baseline.behavioral, 1, "the original still answers");
}

#[tokio::test]
async fn answering_from_a_foreign_lineage_is_cross_talk() {
    let interactions = vec![
        get("i1", "/api/users/1", r#"{"id":1,"name":"Ann"}"#),
        get("i2", "/api/orders/1", r#"{"id":1,"total":42}"#),
    ];
    let original = recorded_collection(&interactions);

    // A catch-all that swallows both paths, claiming lineage from only the first.
    let mut catch_all = original.mocks[0].clone();
    catch_all.id = "greedy".into();
    if let Some(match_config) = catch_all.match_config.as_mut() {
        match_config.urls = vec!["/api/**".to_string()];
    }
    let consolidated = MockCollectionConfig {
        mocks: vec![catch_all],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("greedy", ["rec-1"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(report.score.matched, 2, "the catch-all answers both");
    assert_eq!(
        report.score.no_cross_talk, 1,
        "only the user request has a lineage claim on the catch-all"
    );
    assert_eq!(report.cross_talk.len(), 1);
    assert_eq!(report.cross_talk[0].matched_mock, "greedy");
    assert_eq!(report.cross_talk[0].expected_origin, "rec-2");
    assert_eq!(
        report.score.behavioral, 1,
        "cross-talk alone must sink the interaction"
    );
}

#[tokio::test]
async fn a_dropped_field_is_a_shape_divergence_not_a_status_one() {
    let interactions = vec![get("i1", "/api/users/1", r#"{"id":1,"name":"Ann"}"#)];
    let original = recorded_collection(&interactions);

    let mut lossy = original.mocks[0].clone();
    lossy.id = "lossy".into();
    lossy.response_config = Some(ReturnConfig::Structured {
        status: Some(200),
        headers: rustc_hash::FxHashMap::default(),
        body: Some(r#"{"id":1}"#.to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    });
    let consolidated = MockCollectionConfig {
        mocks: vec![lossy],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("lossy", ["rec-1"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(report.score.status_exact, 1);
    assert_eq!(report.score.shape_equal, 0);
    assert_eq!(report.shape_mismatch.len(), 1);
    assert!(
        report.shape_mismatch[0].detail.contains("name"),
        "detail should name the dropped field, got {:?}",
        report.shape_mismatch[0].detail
    );
    assert_eq!(report.score.behavioral, 0);
}

#[tokio::test]
async fn a_changed_status_is_a_status_divergence() {
    let interactions = vec![get("i1", "/api/users/1", r#"{"id":1}"#)];
    let original = recorded_collection(&interactions);

    let mut wrong_status = original.mocks[0].clone();
    wrong_status.id = "wrong-status".into();
    wrong_status.response_config = Some(ReturnConfig::Structured {
        status: Some(201),
        headers: rustc_hash::FxHashMap::default(),
        body: Some(r#"{"id":1}"#.to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    });
    let consolidated = MockCollectionConfig {
        mocks: vec![wrong_status],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("wrong-status", ["rec-1"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(report.score.status_exact, 0);
    assert_eq!(report.score.shape_equal, 1, "the body was untouched");
    assert_eq!(report.status_mismatch.len(), 1);
    assert!(report.status_mismatch[0].detail.contains("201"));
}

#[tokio::test]
async fn a_value_the_group_never_varied_must_not_start_varying() {
    let interactions = vec![
        get("i1", "/api/users/1", r#"{"id":1,"role":"admin"}"#),
        get("i2", "/api/users/2", r#"{"id":2,"role":"admin"}"#),
    ];
    let original = recorded_collection(&interactions);

    // One mock now answers for both, but invented a role neither recording had.
    let mut drifted = original.mocks[0].clone();
    drifted.id = "drifted".into();
    if let Some(match_config) = drifted.match_config.as_mut() {
        match_config.urls = vec!["/api/users/:id".to_string()];
    }
    drifted.response_config = Some(ReturnConfig::Structured {
        status: Some(200),
        headers: rustc_hash::FxHashMap::default(),
        body: Some(r#"{"id":1,"role":"guest"}"#.to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    });
    let consolidated = MockCollectionConfig {
        mocks: vec![drifted],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("drifted", ["rec-1", "rec-2"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(report.score.shape_equal, 2, "shape is intact; only a value moved");
    assert_eq!(report.score.constants_held, 0);
    assert!(
        report.constant_drift[0].detail.contains("/role"),
        "drift should point at the field, got {:?}",
        report.constant_drift[0].detail
    );
    // `id` varied across the group, so it is not a constant and must not be
    // reported even though the replay returns 1 for both.
    assert!(
        !report.constant_drift[0].detail.contains("/id"),
        "a field the group varied is not a constant: {:?}",
        report.constant_drift[0].detail
    );
}

#[tokio::test]
async fn a_value_every_list_element_agreed_on_must_not_be_invented() {
    // Lists of different lengths, but every entry in every one is a "file".
    let interactions = vec![
        get("i1", "/api/folders/1", r#"{"entries":[{"type":"file","id":1}]}"#),
        get(
            "i2",
            "/api/folders/2",
            r#"{"entries":[{"type":"file","id":2},{"type":"file","id":3}]}"#,
        ),
    ];
    let original = recorded_collection(&interactions);

    let mut inventive = original.mocks[0].clone();
    inventive.id = "inventive".into();
    if let Some(match_config) = inventive.match_config.as_mut() {
        match_config.urls = vec!["/api/folders/:id".to_string()];
    }
    inventive.response_config = Some(ReturnConfig::Structured {
        status: Some(200),
        headers: rustc_hash::FxHashMap::default(),
        // The list kept its shape and the endpoint still answers, but it now
        // claims to hold a folder.
        body: Some(r#"{"entries":[{"type":"folder","id":9}]}"#.to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    });
    let consolidated = MockCollectionConfig {
        mocks: vec![inventive],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("inventive", ["rec-1", "rec-2"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(
        report.score.shape_equal, 2,
        "the shape is intact; only what the entries say changed"
    );
    assert_eq!(report.score.constants_held, 0);
    assert!(
        report.constant_drift[0].detail.contains("/entries[]/type"),
        "drift should name the element field, got {:?}",
        report.constant_drift[0].detail
    );
    assert_eq!(report.score.behavioral, 0);
}

#[tokio::test]
async fn an_element_field_the_group_varied_is_free_to_vary_on_replay() {
    // `type` is a real discriminator here, so a replay is not bound to any one
    // value for it.
    let interactions = vec![
        get("i1", "/api/folders/1", r#"{"entries":[{"type":"file","id":1}]}"#),
        get("i2", "/api/folders/2", r#"{"entries":[{"type":"folder","id":2}]}"#),
    ];
    let original = recorded_collection(&interactions);

    let mut merged = original.mocks[0].clone();
    merged.id = "merged".into();
    if let Some(match_config) = merged.match_config.as_mut() {
        match_config.urls = vec!["/api/folders/:id".to_string()];
    }
    merged.response_config = Some(ReturnConfig::Structured {
        status: Some(200),
        headers: rustc_hash::FxHashMap::default(),
        body: Some(r#"{"entries":[{"type":"folder","id":7}]}"#.to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    });
    let consolidated = MockCollectionConfig {
        mocks: vec![merged],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("merged", ["rec-1", "rec-2"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(
        report.score.constants_held, 2,
        "nothing was constant to hold: {:?}",
        report.constant_drift
    );
}

#[tokio::test]
async fn an_element_field_only_some_entries_carried_is_optional_not_constant() {
    let interactions = vec![
        get(
            "i1",
            "/api/folders/1",
            r#"{"entries":[{"type":"file"},{"type":"file","note":"x"}]}"#,
        ),
        get("i2", "/api/folders/2", r#"{"entries":[{"type":"file"}]}"#),
    ];
    let original = recorded_collection(&interactions);

    let mut merged = original.mocks[0].clone();
    merged.id = "merged".into();
    if let Some(match_config) = merged.match_config.as_mut() {
        match_config.urls = vec!["/api/folders/:id".to_string()];
    }
    merged.response_config = Some(ReturnConfig::Structured {
        status: Some(200),
        headers: rustc_hash::FxHashMap::default(),
        body: Some(r#"{"entries":[{"type":"file"}]}"#.to_string()),
        template: None,
        file: None,
        template_file: None,
        json: Box::new(serde_json::Value::Null),
    });
    let consolidated = MockCollectionConfig {
        mocks: vec![merged],
        ..original.clone()
    };

    let mut provenance = Provenance::new();
    provenance.record("merged", ["rec-1", "rec-2"]);

    let report = verify(
        &interactions,
        &original,
        &consolidated,
        &provenance,
        &options(),
    )
    .await
    .expect("verification runs");

    assert_eq!(
        report.score.constants_held, 2,
        "`note` appeared in one entry of three, so dropping it is not drift: {:?}",
        report.constant_drift
    );
}

// ---------------------------------------------------------------------------
// Shape comparison unit tests
// ---------------------------------------------------------------------------

fn shape(recorded: &str, replayed: &str, options: &FidelityOptions) -> Vec<String> {
    let recorded: JsonValue = serde_json::from_str(recorded).unwrap();
    let replayed: JsonValue = serde_json::from_str(replayed).unwrap();
    compare_shape(&recorded, &replayed, options)
}

#[test]
fn differing_values_of_the_same_kind_are_not_a_shape_change() {
    assert!(
        shape(
            r#"{"id":1,"name":"Ann"}"#,
            r#"{"id":9,"name":"Zed"}"#,
            &FidelityOptions::default()
        )
        .is_empty()
    );
}

#[test]
fn a_kind_change_is_reported_with_its_pointer() {
    let divergences = shape(
        r#"{"user":{"id":1}}"#,
        r#"{"user":{"id":"1"}}"#,
        &FidelityOptions::default(),
    );
    assert_eq!(divergences.len(), 1);
    assert!(divergences[0].contains("/user/id"));
    assert!(divergences[0].contains("integer"));
    assert!(divergences[0].contains("string"));
}

#[test]
fn integers_and_floats_separate_only_under_strict_numbers() {
    let strict = FidelityOptions::default();
    assert_eq!(shape(r#"{"n":1}"#, r#"{"n":1.5}"#, &strict).len(), 1);

    let relaxed = FidelityOptions {
        strict_numbers: false,
        ..FidelityOptions::default()
    };
    assert!(shape(r#"{"n":1}"#, r#"{"n":1.5}"#, &relaxed).is_empty());
}

#[test]
fn an_emptied_array_is_reported_even_when_length_is_not_strict() {
    let relaxed = FidelityOptions::default();
    let divergences = shape(r#"{"items":[1,2,3]}"#, r#"{"items":[]}"#, &relaxed);
    assert_eq!(divergences.len(), 1);
    assert!(divergences[0].contains("replayed none"));

    // A shorter-but-populated array is fine unless length is strict.
    assert!(shape(r#"{"items":[1,2,3]}"#, r#"{"items":[7]}"#, &relaxed).is_empty());
    let strict = FidelityOptions {
        strict_array_len: true,
        ..FidelityOptions::default()
    };
    assert_eq!(shape(r#"{"items":[1,2,3]}"#, r#"{"items":[7]}"#, &strict).len(), 1);
}

#[test]
fn invented_and_dropped_keys_are_reported_separately() {
    let divergences = shape(
        r#"{"a":1,"b":2}"#,
        r#"{"a":1,"c":3}"#,
        &FidelityOptions::default(),
    );
    assert_eq!(divergences.len(), 2);
    assert!(divergences.iter().any(|d| d.contains("dropped b")));
    assert!(divergences.iter().any(|d| d.contains("invented c")));
}

#[test]
fn null_tolerance_is_opt_in() {
    let strict = FidelityOptions::default();
    assert_eq!(shape(r#"{"a":null}"#, r#"{"a":"x"}"#, &strict).len(), 1);

    let relaxed = FidelityOptions {
        strict_null: false,
        ..FidelityOptions::default()
    };
    assert!(shape(r#"{"a":null}"#, r#"{"a":"x"}"#, &relaxed).is_empty());
}

#[test]
fn only_the_head_of_a_long_array_is_probed() {
    let recorded = format!("[{}]", vec!["1"; 100].join(","));
    let mut replayed_items = vec!["1"; 100];
    replayed_items[50] = "\"boom\"";
    let replayed = format!("[{}]", replayed_items.join(","));

    let options = FidelityOptions::default();
    assert!(
        shape(&recorded, &replayed, &options).is_empty(),
        "the default 8-element probe does not reach index 50"
    );

    let deep = FidelityOptions {
        array_probe: 100,
        ..FidelityOptions::default()
    };
    assert_eq!(shape(&recorded, &replayed, &deep).len(), 1);
}

// ---------------------------------------------------------------------------
// Leaf flattening and target parsing
// ---------------------------------------------------------------------------

#[test]
fn leaves_are_addressed_by_json_pointer() {
    let value: JsonValue =
        serde_json::from_str(r#"{"a":{"b":"deep"},"c":true}"#).unwrap();
    let leaves = flatten_leaves(&value, 64);

    assert_eq!(leaves.get("/a/b"), Some(&serde_json::json!("deep")));
    assert_eq!(leaves.get("/c"), Some(&serde_json::json!(true)));
    assert_eq!(leaves.len(), 2);
}

#[test]
fn an_array_is_one_leaf_not_one_per_element() {
    // Two recordings can agree on element 0 while disagreeing on length, so
    // positions are never addressed. The list as a whole is, because a list that
    // never changed is as constant as any scalar.
    let value: JsonValue =
        serde_json::from_str(r#"{"items":[{"label":"l0"}],"kind":"page"}"#).unwrap();
    let leaves = flatten_leaves(&value, 64);

    assert_eq!(leaves.get("/kind"), Some(&serde_json::json!("page")));
    assert_eq!(
        leaves.get("/items"),
        Some(&serde_json::json!([{"label":"l0"}]))
    );
    assert!(leaves.keys().all(|pointer| !pointer.starts_with("/items/")));
}

#[test]
fn leaf_collection_stops_at_the_budget() {
    let value: JsonValue = serde_json::from_str(
        r#"{"a":"1","b":"2","c":"3","d":"4","e":"5","f":"6"}"#,
    )
    .unwrap();
    assert_eq!(flatten_leaves(&value, 4).len(), 4);
}

#[test]
fn a_scalar_body_flattens_to_the_root_pointer() {
    let value: JsonValue = serde_json::json!("hello");
    let leaves = flatten_leaves(&value, 8);
    assert_eq!(leaves.get("/"), Some(&serde_json::json!("hello")));
}

#[test]
fn a_target_splits_the_same_way_however_it_was_recorded() {
    let bare = RecordedRequest {
        method: "GET".to_string(),
        uri: "/api/users".to_string(),
        query: Some("limit=10".to_string()),
        headers: vec![],
        body: None,
    };
    assert_eq!(
        split_target(&bare),
        ("/api/users".to_string(), Some("limit=10".to_string()))
    );

    let embedded = RecordedRequest {
        uri: "/api/users?limit=10".to_string(),
        query: None,
        ..bare.clone()
    };
    assert_eq!(
        split_target(&embedded),
        ("/api/users".to_string(), Some("limit=10".to_string()))
    );

    let absolute = RecordedRequest {
        uri: "https://api.example.com/api/users?limit=10".to_string(),
        query: None,
        ..bare.clone()
    };
    assert_eq!(
        split_target(&absolute),
        ("/api/users".to_string(), Some("limit=10".to_string()))
    );

    let no_query = RecordedRequest {
        uri: "/api/users".to_string(),
        query: None,
        ..bare.clone()
    };
    assert_eq!(split_target(&no_query), ("/api/users".to_string(), None));

    // An explicit empty query must not shadow one embedded in the uri.
    let empty_query = RecordedRequest {
        uri: "/api/users?limit=10".to_string(),
        query: Some(String::new()),
        ..bare
    };
    assert_eq!(
        split_target(&empty_query),
        ("/api/users".to_string(), Some("limit=10".to_string()))
    );
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

#[test]
fn an_empty_recording_scores_one_rather_than_dividing_by_zero() {
    let score = FidelityScore::default();
    assert!((score.behavioral_ratio() - 1.0).abs() < f64::EPSILON);
    assert!((score.matched_ratio() - 1.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn examples_are_capped_but_counts_are_not() {
    let interactions: Vec<_> = (1..=10)
        .map(|n| get(&format!("i{n}"), &format!("/api/users/{n}"), r#"{"id":1}"#))
        .collect();
    let original = recorded_collection(&interactions);
    let empty = MockCollectionConfig {
        name: None,
        description: None,
        enabled: true,
        vars: None,
        mocks: vec![],
    };

    let report = verify(
        &interactions,
        &original,
        &empty,
        &Provenance::new(),
        &FidelityOptions {
            max_examples: 3,
            ..options()
        },
    )
    .await
    .expect("verification runs");

    assert_eq!(report.unmatched.len(), 3, "examples are capped");
    assert_eq!(report.score.matched, 0);
    assert_eq!(report.score.total, 10, "counts are exact");
    assert!(report.examples_capped);
}

// ---------------------------------------------------------------------------
// Threshold gate
// ---------------------------------------------------------------------------

#[test]
fn passes_compares_against_the_behavioural_ratio() {
    let report = FidelityReport {
        score: FidelityScore {
            total: 4,
            behavioral: 3,
            ..FidelityScore::default()
        },
        baseline: FidelityScore {
            total: 4,
            behavioral: 4,
            ..FidelityScore::default()
        },
        unmatched: vec![],
        baseline_unmatched: vec![],
        cross_talk: vec![],
        status_mismatch: vec![],
        shape_mismatch: vec![],
        constant_drift: vec![],
        render_errors: vec![],
        examples_capped: false,
    };

    assert!(report.passes(0.75));
    assert!(!report.passes(0.8));
    assert!((report.behavioral_delta() + 0.25).abs() < f64::EPSILON);
}
