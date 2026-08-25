// See the note in `ferrimock`'s lib.rs: proving the mock loader's future
// `Send` needs more solver depth than the default allows since
// nightly-2026-08-24, and the limit is per crate rather than inherited.
#![recursion_limit = "256"]
#![cfg(all(feature = "server", feature = "spec", feature = "graphql"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! A world that outlives the process it was written in.
//!
//! Driven through a real server, because the contract is not "the delta
//! serialises" — the unit tests beside `persist.rs` cover that — but "stop,
//! start again, and the API answers with what was written".
//!
//! One test, and a serial one. A process has one world: two of these running
//! side by side would each load a schema into it and `serve: graphql` would
//! then match several. And between phases the world is reset explicitly,
//! because a second `start` in the same process inherits the delta the first
//! one left — without the reset, a restore that read nothing would pass.

use ferrimock::core::world::global_world;
use ferrimock::services::serve::{ServeHandle, ServeInput, start};
use serde_json::{Value, json};

const SCHEMA: &str = r"
type User { id: ID!, name: String!, email: String! }
type Post { id: ID!, title: String!, author: User! }

type Query {
  user(id: ID!): User
  users: [User!]!
  posts: [Post!]!
}

type Mutation {
  createUser(name: String!, email: String!): User!
  createPost(title: String!, authorId: ID!): Post!
}
";

fn collection(persistence: Option<&str>) -> String {
    let line = persistence.map_or_else(String::new, |name| format!("  persistence: {name}\n"));
    format!(
        "world:\n  schemas:\n    - schema.graphql\n  seed: 1\n  count: 0\n{line}\n\
         mocks:\n  - id: gql\n    match:\n      POST: /graphql\n    serve: graphql\n"
    )
}

/// Start a server on the collection currently written in `dir`, against a
/// world holding nothing — which is what a fresh process would have.
async fn restart(dir: &std::path::Path) -> ServeHandle {
    global_world().reset();
    start(ServeInput {
        port: 0,
        mock_file: Some(dir.join("mocks.yaml").to_string_lossy().into_owned()),
        ..ServeInput::default()
    })
    .await
    .expect("server start")
}

async fn gql(url: &str, query: &str) -> Value {
    reqwest::Client::new()
        .post(format!("{url}/graphql"))
        .json(&json!({ "query": query }))
        .send()
        .await
        .expect("request")
        .json()
        .await
        .expect("json")
}

async fn names(url: &str) -> Vec<String> {
    gql(url, "{ users { name } }").await["data"]["users"]
        .as_array()
        .expect("users")
        .iter()
        .map(|user| user["name"].as_str().expect("name").to_string())
        .collect()
}

async fn create_user(url: &str, name: &str) -> String {
    let made = gql(
        url,
        &format!(
            r#"mutation {{ createUser(name: "{name}", email: "{name}@example.com") {{ id }} }}"#
        ),
    )
    .await;
    made["data"]["createUser"]["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

#[tokio::test]
#[serial_test::serial]
async fn a_world_comes_back_with_what_was_written_into_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("schema.graphql"), SCHEMA).expect("schema");
    std::fs::write(
        dir.path().join("mocks.yaml"),
        collection(Some("state.json")),
    )
    .expect("mocks");
    let state = dir.path().join("state.json");

    // An empty world, filled through the API.
    let first = restart(dir.path()).await;
    assert!(
        names(&first.url).await.is_empty(),
        "`count: 0` starts empty"
    );
    let ada = create_user(&first.url, "Ada").await;
    gql(
        &first.url,
        &format!(r#"mutation {{ createPost(title: "Notes", authorId: "{ada}") {{ id }} }}"#),
    )
    .await;

    assert!(!state.exists(), "nothing is written until the world stops");
    drop(first);
    assert!(state.exists(), "stopping wrote the world's state");

    // A world that knows nothing until it reads the file.
    let second = restart(dir.path()).await;
    assert_eq!(names(&second.url).await, ["Ada"]);

    // The link is what a round trip can quietly lose: the delta holds the
    // author as a key, and it has to resolve against a world rebuilt from a
    // seed that never knew about either record.
    let posts = gql(&second.url, "{ posts { title author { id name } } }").await;
    assert_eq!(posts["data"]["posts"][0]["title"], "Notes");
    assert_eq!(posts["data"]["posts"][0]["author"]["id"], ada.as_str());
    assert_eq!(posts["data"]["posts"][0]["author"]["name"], "Ada");

    // A restored world is still writable, and still keeps what it is given.
    create_user(&second.url, "Grace").await;
    drop(second);

    let third = restart(dir.path()).await;
    assert_eq!(names(&third.url).await, ["Ada", "Grace"]);
    drop(third);

    // What the file holds is the writes, and only those: three creations,
    // not a copy of a world the seed already derives.
    let held: Value =
        serde_json::from_str(&std::fs::read_to_string(&state).expect("state")).expect("json");
    assert_eq!(
        held["seed"], 1,
        "the file records the seed it means something against"
    );
    assert_eq!(
        held["delta"]["entries"].as_array().expect("entries").len(),
        3,
        "two users and a post, and nothing else"
    );
}
