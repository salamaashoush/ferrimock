#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! A machine declared in YAML, driving routes, with no schema anywhere.
//!
//! The whole point of naming machines was that they stop being an entity's
//! idea, so the test that matters is one with no `world:` block in it at all.

use ferrimock::engine::types::ResponseGeneratorExt;
use ferrimock::engine::{MockMatcher, MockRegistry};

const COLLECTION: &str = r#"
machines:
  order:
    states:
      - name: created
        on: { pay: paid, cancel: cancelled }
      - name: paid
        on: { ship: shipped, refund: created }
      - name: shipped
        on: { deliver: delivered }
      - name: delivered
      - name: cancelled

mocks:
  - id: get-order
    match: { GET: "/api/order/:id" }
    response:
      template: '{"state": "{{ machine_state(machine="order", key=captures.id) }}"}'

  - id: pay-order
    match: { POST: "/api/order/:id/pay" }
    response:
      template: |-
        {%- if machine_can(machine="order", key=captures.id, event="pay") -%}
        {"status": 200, "body": {"state": "{{ machine_fire(machine="order", key=captures.id, event="pay") }}"}}
        {%- else -%}
        {"status": 409, "body": {"error": "cannot pay from {{ machine_state(machine="order", key=captures.id) }}"}}
        {%- endif -%}

  - id: ship-order
    match: { POST: "/api/order/:id/ship" }
    response:
      template: |-
        {%- if machine_can(machine="order", key=captures.id, event="ship") -%}
        {"status": 200, "body": {"state": "{{ machine_fire(machine="order", key=captures.id, event="ship") }}"}}
        {%- else -%}
        {"status": 409, "body": {"error": "cannot ship from {{ machine_state(machine="order", key=captures.id) }}"}}
        {%- endif -%}
"#;

async fn served(matcher: &MockMatcher, method: &str, path: &str) -> (u16, String) {
    let verb: http::Method = method.parse().expect("a method");
    let found = matcher
        .find_match(&verb, path, None, &http::HeaderMap::new(), None)
        .unwrap_or_else(|| panic!("nothing matched {method} {path}"));
    let rendered = found
        .mock
        .response
        .generate_dynamic(
            method,
            path,
            None,
            &http::HeaderMap::new(),
            None,
            found.captures,
            found.mock.vars.as_ref(),
        )
        .await
        .expect("the matched mock renders");
    (
        rendered.status.unwrap_or(found.mock.response.status).into(),
        String::from_utf8_lossy(&rendered.body).to_string(),
    )
}

/// The sequence a real client walks, and the two things a machine has to get
/// right: a read never advances anything, and a move nothing declares is
/// refused rather than quietly allowed.
#[tokio::test]
async fn a_machine_declared_in_yaml_drives_routes_without_a_schema() {
    let dir = std::env::temp_dir().join("ferrimock-machine-tests");
    std::fs::create_dir_all(&dir).expect("a temp dir");
    std::fs::write(dir.join("machines.yaml"), COLLECTION).expect("writes");

    let registry = MockRegistry::new();
    registry.load_from_directory(&dir).await.expect("loads");
    ferrimock::template::get_global_machines().reset();
    let matcher = MockMatcher::new(registry.clone());

    let (_, body) = served(&matcher, "GET", "/api/order/7").await;
    assert!(body.contains("created"), "{body}");

    // Reading is not moving. A `GET` that advances a lifecycle is a mock lying
    // about a safe method, and it is the bug every poll-counter scenario has.
    let (_, body) = served(&matcher, "GET", "/api/order/7").await;
    assert!(body.contains("created"), "a read moved it: {body}");

    // `shipped` is reachable, but not from here, and the refusal says so.
    let (status, body) = served(&matcher, "POST", "/api/order/7/ship").await;
    assert_eq!(status, 409, "{body}");
    assert!(body.contains("cannot ship from created"), "{body}");

    let (status, body) = served(&matcher, "POST", "/api/order/7/pay").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("paid"), "{body}");

    let (status, body) = served(&matcher, "POST", "/api/order/7/ship").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("shipped"), "{body}");

    // A key is an instance. Order 8 has not been anywhere.
    let (_, body) = served(&matcher, "GET", "/api/order/8").await;
    assert!(body.contains("created"), "{body}");

    // And what the run never exercised is a question with an answer, reported
    // beside which mocks never ran. A mock that never fired is one question; a
    // branch of a mock that never fired is another, and template branching
    // cannot answer it because the branches are not data.
    let coverage = registry.coverage();
    assert!(
        coverage
            .unreached_states
            .contains(&"order.cancelled".to_string()),
        "nothing cancelled anything: {:?}",
        coverage.unreached_states
    );
    assert!(
        !coverage
            .unreached_states
            .contains(&"order.paid".to_string()),
        "`paid` was reached: {:?}",
        coverage.unreached_states
    );
    assert!(
        coverage
            .untaken_edges
            .contains(&"order.shipped --deliver->".to_string()),
        "nothing delivered: {:?}",
        coverage.untaken_edges
    );
    assert!(
        !coverage
            .untaken_edges
            .contains(&"order.created --pay->".to_string()),
        "`pay` was taken: {:?}",
        coverage.untaken_edges
    );
}

const SUGARED: &str = r#"
machines:
  gate:
    states:
      - name: shut
        on: { open: ajar }
      - name: ajar
        on: { open: wide, shut: shut }
      - name: wide
        on: { shut: shut }

mocks:
  - id: read-gate
    match: { GET: "/gate/:id" }
    when: { machine: gate, key: "{{ captures.id }}" }
    states:
      shut: { status: 423, json: { state: "shut" } }
      ajar: { status: 200, json: { state: "ajar" } }
      wide: { status: 200, json: { state: "wide" } }

  - id: open-gate
    match: { POST: "/gate/:id/open" }
    fire: { machine: gate, key: "{{ captures.id }}", event: open }
    response:
      template: '{"moved": "{{ _fired }}"}'
"#;

/// `when:`/`states:`/`fire:` say the same thing the template calls say, with
/// the states visible as data instead of buried in a branch. The behaviour has
/// to be identical, including that a read never moves anything.
#[tokio::test]
async fn the_declarative_binding_says_what_the_template_calls_say() {
    let dir = std::env::temp_dir().join("ferrimock-machine-sugar");
    std::fs::create_dir_all(&dir).expect("a temp dir");
    std::fs::write(dir.join("gate.yaml"), SUGARED).expect("writes");

    let registry = MockRegistry::new();
    registry.load_from_directory(&dir).await.expect("loads");
    ferrimock::template::get_global_machines().reset();
    let matcher = MockMatcher::new(registry.clone());

    // The per-state status is the state's own, not the mock's.
    let (status, body) = served(&matcher, "GET", "/gate/1").await;
    assert_eq!(status, 423, "{body}");
    assert!(body.contains("shut"), "{body}");

    let (status, _) = served(&matcher, "GET", "/gate/1").await;
    assert_eq!(status, 423, "reading did not move it");

    let (_, body) = served(&matcher, "POST", "/gate/1/open").await;
    assert!(
        body.contains("ajar"),
        "`fire` answers where it landed: {body}"
    );

    let (status, body) = served(&matcher, "GET", "/gate/1").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("ajar"), "{body}");

    let (_, _) = served(&matcher, "POST", "/gate/1/open").await;
    let (status, body) = served(&matcher, "GET", "/gate/1").await;
    assert_eq!(status, 200);
    assert!(body.contains("wide"), "{body}");

    // Still a separate instance per key.
    let (status, _) = served(&matcher, "GET", "/gate/2").await;
    assert_eq!(status, 423);
}
