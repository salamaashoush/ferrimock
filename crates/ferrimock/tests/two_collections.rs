#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Anything a collection contributes to a process-global, contributed by two.
//!
//! Machines were installed by *replacing* the registry's set, so a directory
//! holding two collections kept only whichever loaded last — and every test
//! loaded one collection, so nothing noticed. The bug is not specific to
//! machines: it is what a global installed by assignment does, and the only
//! reliable way to catch it is to stop testing these with a single file.

use ferrimock::engine::MockRegistry;

async fn loaded(files: &[(&str, &str)]) -> MockRegistry {
    let dir = std::env::temp_dir().join("ferrimock-two-collections");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a temp dir");
    for (name, body) in files {
        std::fs::write(dir.join(name), body).expect("writes");
    }
    let registry = MockRegistry::new();
    registry.load_from_directory(&dir).await.expect("loads");
    registry
}

/// One test rather than two, deliberately: these assert on a *process-global*,
/// and two of them running in parallel would each see the other's machines. A
/// global is exactly what is being tested, so the test has to be honest about
/// sharing one.
#[tokio::test]
async fn machines_survive_a_second_collection_and_a_reload_that_drops_one() {
    let registry = loaded(&[
        (
            "orders.yaml",
            r"
machines:
  order:
    states:
      - name: created
        on: { pay: paid }
      - name: paid
mocks:
  - id: o
    match: { GET: /o }
    response: { status: 200, body: 'o' }
",
        ),
        (
            "gates.yaml",
            r"
machines:
  gate:
    states:
      - name: shut
        on: { open: wide }
      - name: wide
mocks:
  - id: g
    match: { GET: /g }
    response: { status: 200, body: 'g' }
",
        ),
    ])
    .await;

    let named = || ferrimock::template::get_global_machines().names();
    assert!(
        named().iter().any(|name| name == "order") && named().iter().any(|name| name == "gate"),
        "one collection's machines replaced the other's: {:?}",
        named()
    );
    // Both mocks survived, which was always true — and is what made the machine
    // bug look like it could not happen.
    assert_eq!(registry.get_all_mocks().len(), 2);

    // A reload that drops one file must forget only that file's machines. A
    // plain merge keeps `order` forever; a plain replace loses `gate`.
    let _ = loaded(&[(
        "orders.yaml",
        r"
machines:
  gate:
    states: [{ name: shut }, { name: wide }]
mocks:
  - id: o
    match: { GET: /o }
    response: { status: 200, body: 'o' }
",
    )])
    .await;

    assert!(
        named().iter().any(|name| name == "gate"),
        "the surviving machine went with the deleted one: {:?}",
        named()
    );
    assert!(
        !named().iter().any(|name| name == "order"),
        "a machine no file declares any more still exists: {:?}",
        named()
    );
}
