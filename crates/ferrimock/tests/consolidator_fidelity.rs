//! Fidelity of consolidation over realistic recorded traffic.
//!
//! Each scenario is a small recording that stresses one thing consolidation has
//! to get right. The assertions state what the engine actually preserves today,
//! so a regression shows up as a failing test and an improvement shows up as an
//! assertion that has to be tightened.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::Utc;
use ferrimock::config::{MatchConfig, MockCollectionConfig, MockConfig, ReturnConfig};
use ferrimock::consolidator::{
    ConsolidatorOptions, FidelityOptions, FidelityReport, MockConsolidator,
};
use ferrimock::recorder::{RecordedInteraction, RecordedRequest, RecordedResponse};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Recorded {
    method: &'static str,
    uri: String,
    query: Option<String>,
    request_body: Option<String>,
    status: u16,
    body: String,
}

fn get(uri: impl Into<String>, body: impl Into<String>) -> Recorded {
    Recorded {
        method: "GET",
        uri: uri.into(),
        query: None,
        request_body: None,
        status: 200,
        body: body.into(),
    }
}

fn get_with_query(
    uri: impl Into<String>,
    query: impl Into<String>,
    body: impl Into<String>,
) -> Recorded {
    Recorded {
        query: Some(query.into()),
        ..get(uri, body)
    }
}

fn post(
    uri: impl Into<String>,
    request_body: impl Into<String>,
    body: impl Into<String>,
) -> Recorded {
    Recorded {
        method: "POST",
        request_body: Some(request_body.into()),
        ..get(uri, body)
    }
}

fn with_status(status: u16, recorded: Recorded) -> Recorded {
    Recorded { status, ..recorded }
}

fn to_interactions(recorded: Vec<Recorded>) -> Vec<RecordedInteraction> {
    recorded
        .into_iter()
        .enumerate()
        .map(|(index, r)| RecordedInteraction {
            id: format!("i{}", index + 1),
            timestamp: Utc::now(),
            request: RecordedRequest {
                method: r.method.to_string(),
                uri: r.uri,
                query: r.query,
                headers: vec![("accept".to_string(), "application/json".to_string())],
                body: r.request_body,
            },
            response: RecordedResponse {
                status: r.status,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: r.body,
            },
            duration: Duration::from_millis(7),
        })
        .collect()
}

/// Mirrors what `MockRecorder` writes: one exact-URL mock per interaction.
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
        name: Some("scenario".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    }
}

async fn run(recorded: Vec<Recorded>) -> (usize, FidelityReport) {
    run_with(recorded, ConsolidatorOptions::default()).await
}

async fn run_with(
    recorded: Vec<Recorded>,
    options: ConsolidatorOptions,
) -> (usize, FidelityReport) {
    let interactions = to_interactions(recorded);
    let original = recorded_collection(&interactions);
    let fidelity = FidelityOptions {
        reset_persistence: true,
        ..FidelityOptions::default()
    };

    let mut consolidator = MockConsolidator::with_options(options);
    let (consolidated, report) = consolidator
        .consolidate_verified(&interactions, original, &fidelity)
        .await
        .expect("verified consolidation runs");

    (consolidated.mocks.len(), report)
}

/// Run the whole pipeline the CLI runs: HAR -> mocks -> consolidate -> verify.
///
/// The hand-built collection above mirrors the recorder, but only the HAR loader
/// exercises request-body discrimination, `once` sequencing and query pinning --
/// the parts that decide whether two recordings of one URL stay selectable.
async fn run_through_har(recorded: Vec<Recorded>) -> (usize, FidelityReport) {
    let interactions = to_interactions(recorded);
    let har = serde_json::json!({
        "log": {
            "version": "1.2",
            "creator": {"name": "test", "version": "1"},
            "entries": interactions.iter().map(|it| {
                let url = match &it.request.query {
                    Some(query) => format!("https://api.example.com{}?{}", it.request.uri, query),
                    None => format!("https://api.example.com{}", it.request.uri),
                };
                let mut request = serde_json::json!({
                    "method": it.request.method,
                    "url": url,
                    "httpVersion": "HTTP/1.1",
                    "cookies": [],
                    "headers": [{"name": "accept", "value": "application/json"}],
                    "queryString": [],
                    "headersSize": -1,
                    "bodySize": 0,
                });
                if let Some(body) = &it.request.body
                    && let Some(request) = request.as_object_mut()
                {
                    request.insert(
                        "postData".to_string(),
                        serde_json::json!({
                            "mimeType": "application/json",
                            "text": body,
                        }),
                    );
                }
                serde_json::json!({
                    "startedDateTime": it.timestamp.to_rfc3339(),
                    "time": 7.0,
                    "request": request,
                    "response": {
                        "status": it.response.status,
                        "statusText": "",
                        "httpVersion": "HTTP/1.1",
                        "cookies": [],
                        "headers": [{"name": "content-type", "value": "application/json"}],
                        "content": {
                            "size": it.response.body.len(),
                            "mimeType": "application/json",
                            "text": it.response.body,
                        },
                        "redirectURL": "",
                        "headersSize": -1,
                        "bodySize": it.response.body.len(),
                    },
                    "cache": {},
                    "timings": {"send": 0.0, "wait": 7.0, "receive": 0.0},
                })
            }).collect::<Vec<_>>(),
        }
    });

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("traffic.har");
    std::fs::write(&path, serde_json::to_string(&har).expect("serialize har")).expect("write har");

    let mocks = ferrimock::config::HarLoader::new()
        .load_from_file(&path)
        .await
        .expect("HAR converts to mocks");
    let original = MockCollectionConfig {
        name: Some("har".to_string()),
        description: None,
        enabled: true,
        vars: None,
        mocks,
    };

    let fidelity = FidelityOptions {
        reset_persistence: true,
        ..FidelityOptions::default()
    };
    let mut consolidator = MockConsolidator::new();
    let (consolidated, report) = consolidator
        .consolidate_verified(&interactions, original, &fidelity)
        .await
        .expect("verified consolidation runs");

    (consolidated.mocks.len(), report)
}

fn explain(report: &FidelityReport) -> String {
    format!(
        "matched {}/{}, lineage {}, status {}, shape {}, constants {}, behavioural {}\n\
         unmatched: {:?}\ncross-talk: {:?}\nstatus: {:?}\nshape: {:?}\nconstants: {:?}\nrender: {:?}",
        report.score.matched,
        report.score.total,
        report.score.no_cross_talk,
        report.score.status_exact,
        report.score.shape_equal,
        report.score.constants_held,
        report.score.behavioral,
        report.unmatched,
        report.cross_talk,
        report.status_mismatch,
        report.shape_mismatch,
        report.constant_drift,
        report.render_errors,
    )
}

/// Every scenario's unconsolidated recording must replay perfectly. Otherwise
/// the consolidated numbers measure the recorder, not the consolidator.
fn assert_baseline_perfect(report: &FidelityReport) {
    assert_eq!(
        report.baseline.behavioral, report.baseline.total,
        "the recording does not replay against its own collection: {}",
        explain(report)
    );
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_uniform_detail_endpoint_collapses_without_loss() {
    let (mocks, report) = run(vec![
        get("/v2/users/1", r#"{"type":"user","id":"1","name":"Ann"}"#),
        get("/v2/users/2", r#"{"type":"user","id":"2","name":"Bob"}"#),
        get("/v2/users/3", r#"{"type":"user","id":"3","name":"Cid"}"#),
        get("/v2/users/4", r#"{"type":"user","id":"4","name":"Dee"}"#),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(mocks, 1, "four same-shaped users should become one pattern");
    assert_eq!(
        report.score.behavioral, 4,
        "consolidation must not change behaviour: {}",
        explain(&report)
    );
}

#[tokio::test]
async fn distinct_resources_under_one_collection_stay_distinct() {
    let (_, report) = run(vec![
        get("/v2/users/1", r#"{"type":"user","id":"1","name":"Ann"}"#),
        get("/v2/users/2", r#"{"type":"user","id":"2","name":"Bob"}"#),
        get("/v2/users/3", r#"{"type":"user","id":"3","name":"Cid"}"#),
        get("/v2/folders/1", r#"{"type":"folder","id":"1","name":"Docs"}"#),
        get("/v2/folders/2", r#"{"type":"folder","id":"2","name":"Pics"}"#),
        get("/v2/folders/3", r#"{"type":"folder","id":"3","name":"Code"}"#),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(
        report.score.no_cross_talk, 6,
        "a user request must never be answered by a folder mock: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.shape_equal, 6,
        "users and folders have different shapes: {}",
        explain(&report)
    );
}

#[tokio::test]
async fn a_paginated_listing_survives_consolidation() {
    let page = |offset: u32, items: &str| {
        get_with_query(
            "/v2/folders/0/items",
            format!("offset={offset}&limit=2"),
            format!(r#"{{"total":6,"offset":{offset},"limit":2,"items":{items}}}"#),
        )
    };

    let (_, report) = run(vec![
        page(0, r#"[{"type":"file","id":"10"},{"type":"file","id":"11"}]"#),
        page(2, r#"[{"type":"file","id":"12"},{"type":"file","id":"13"}]"#),
        page(4, r#"[{"type":"file","id":"14"},{"type":"file","id":"15"}]"#),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(
        report.score.matched, 3,
        "every page must still be answerable: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.shape_equal, 3,
        "a page must keep total/offset/limit/items: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.constants_held, 3,
        "total never varied across the pages: {}",
        explain(&report)
    );
}

#[tokio::test]
async fn mixed_status_codes_in_one_path_shape_are_preserved() {
    let (mocks, report) = run(vec![
        get("/v2/files/1", r#"{"type":"file","id":"1","name":"a.txt"}"#),
        get("/v2/files/2", r#"{"type":"file","id":"2","name":"b.txt"}"#),
        get("/v2/files/3", r#"{"type":"file","id":"3","name":"c.txt"}"#),
        with_status(
            404,
            get("/v2/files/999", r#"{"type":"error","status":404,"code":"not_found"}"#),
        ),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(
        report.score.status_exact, 4,
        "a 404 must not become a 200: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.behavioral, 4,
        "splitting the error out must cost nothing: {}",
        explain(&report)
    );
    assert_eq!(
        mocks, 2,
        "the three files become a pattern and the 404 keeps its exact URL"
    );
}

#[tokio::test]
async fn an_api_version_segment_is_not_an_id() {
    let (mocks, report) = run(vec![
        get("/api/2/users/1", r#"{"v":2,"id":"1"}"#),
        get("/api/2/users/2", r#"{"v":2,"id":"2"}"#),
        get("/api/2/users/3", r#"{"v":2,"id":"3"}"#),
        get("/api/3/users/1", r#"{"v":3,"id":"1","extra":true}"#),
        get("/api/3/users/2", r#"{"v":3,"id":"2","extra":true}"#),
        get("/api/3/users/3", r#"{"v":3,"id":"3","extra":true}"#),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(
        report.score.no_cross_talk, 6,
        "v2 and v3 are different resources: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.shape_equal, 6,
        "v3 carries a field v2 does not: {}",
        explain(&report)
    );
    assert_eq!(
        mocks, 2,
        "each version earns its own pattern, not one shared /api/{{id}}/users/{{id2}}"
    );
}

#[tokio::test]
async fn posts_differing_only_by_request_body_stay_distinguishable() {
    let (_, report) = run(vec![
        post(
            "/v2/search",
            r#"{"query":"invoices"}"#,
            r#"{"total":2,"items":[{"id":"1"},{"id":"2"}]}"#,
        ),
        post(
            "/v2/search",
            r#"{"query":"contracts"}"#,
            r#"{"total":5,"items":[{"id":"3"}]}"#,
        ),
        post(
            "/v2/search",
            r#"{"query":"receipts"}"#,
            r#"{"total":9,"items":[{"id":"4"}]}"#,
        ),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(
        report.score.no_cross_talk, 3,
        "each search body deserves its own answer: {}",
        explain(&report)
    );
}

#[tokio::test]
async fn nested_resource_ids_do_not_collide() {
    let (_, report) = run(vec![
        get("/v2/enterprises/1/users/10", r#"{"enterprise":"1","user":"10"}"#),
        get("/v2/enterprises/1/users/11", r#"{"enterprise":"1","user":"11"}"#),
        get("/v2/enterprises/2/users/10", r#"{"enterprise":"2","user":"10"}"#),
        get("/v2/enterprises/2/users/11", r#"{"enterprise":"2","user":"11"}"#),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(
        report.score.behavioral, 4,
        "enterprise and user ids must both survive: {}",
        explain(&report)
    );
}

#[tokio::test]
async fn a_repeated_identical_request_collapses_to_one_mock() {
    let body = r#"{"type":"user","id":"me","login":"ann@example.com"}"#;
    let (mocks, report) = run(vec![
        get("/v2/users/me", body),
        get("/v2/users/me", body),
        get("/v2/users/me", body),
        get("/v2/users/me", body),
    ])
    .await;

    assert_baseline_perfect(&report);
    assert_eq!(mocks, 1, "four identical recordings are one mock");
    assert_eq!(
        report.score.value_equal, 4,
        "a deduplicated mock must reproduce byte for byte: {}",
        explain(&report)
    );
}

// ---------------------------------------------------------------------------
// Whole pipeline, as the CLI runs it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn searches_differing_only_by_request_body_survive_the_whole_pipeline() {
    let (_, report) = run_through_har(vec![
        post(
            "/v2/search",
            r#"{"query":"invoices"}"#,
            r#"{"total":2,"items":[{"type":"file","id":"1"}]}"#,
        ),
        post(
            "/v2/search",
            r#"{"query":"contracts"}"#,
            r#"{"total":5,"items":[{"type":"file","id":"2"}]}"#,
        ),
        post(
            "/v2/search",
            r#"{"query":"receipts"}"#,
            r#"{"total":9,"items":[{"type":"file","id":"3"}]}"#,
        ),
    ])
    .await;

    assert_eq!(
        report.baseline.behavioral, 3,
        "three distinct searches must each be answerable before consolidation; \
         if they collapse to one matcher the recording is already lossy: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.matched, 3,
        "a merged mock must answer for every search it merged: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.behavioral, 3,
        "consolidating the searches must cost nothing: {}",
        explain(&report)
    );
}

#[tokio::test]
async fn a_mixed_corpus_round_trips_through_the_whole_pipeline() {
    let mut recorded = Vec::new();
    for id in 1..=5 {
        recorded.push(get(
            format!("/v2/users/{id}"),
            format!(r#"{{"type":"user","id":"{id}","login":"u{id}@example.com"}}"#),
        ));
    }
    for id in 1..=4 {
        recorded.push(get(
            format!("/v2/files/{id}"),
            format!(r#"{{"type":"file","id":"{id}","name":"f{id}.pdf","size":{}}}"#, 1000 + id),
        ));
    }
    recorded.push(with_status(
        404,
        get(
            "/v2/files/999",
            r#"{"type":"error","status":404,"code":"not_found"}"#,
        ),
    ));
    for offset in [0, 2, 4] {
        recorded.push(get_with_query(
            "/v2/folders/0/items",
            format!("offset={offset}&limit=2"),
            format!(
                r#"{{"total":6,"offset":{offset},"limit":2,"items":[{{"type":"file","id":"{}"}}]}}"#,
                offset + 1
            ),
        ));
    }

    let recorded_count = recorded.len();
    let (mocks, report) = run_through_har(recorded).await;

    assert_eq!(
        report.baseline.behavioral, recorded_count,
        "the recording must replay against its own mocks: {}",
        explain(&report)
    );
    assert_eq!(
        report.score.behavioral, recorded_count,
        "consolidation must preserve every recorded behaviour: {}",
        explain(&report)
    );
    assert!(
        mocks < recorded_count / 2,
        "consolidation must still earn its keep: {recorded_count} recordings -> {mocks} mocks"
    );
}

#[tokio::test]
async fn consolidation_disabled_is_a_faithful_identity() {
    let recorded = vec![
        get("/v2/users/1", r#"{"id":"1"}"#),
        get("/v2/users/2", r#"{"id":"2"}"#),
        get("/v2/users/3", r#"{"id":"3"}"#),
    ];
    let (mocks, report) = run_with(
        recorded,
        ConsolidatorOptions {
            enable_consolidation: false,
            ..ConsolidatorOptions::default()
        },
    )
    .await;

    assert_eq!(mocks, 3, "nothing may be merged when consolidation is off");
    assert_eq!(
        report.score.value_equal, 3,
        "the identity transform must be byte-exact: {}",
        explain(&report)
    );
    assert!(
        (report.behavioral_delta()).abs() < f64::EPSILON,
        "identity costs nothing: {}",
        explain(&report)
    );
}
