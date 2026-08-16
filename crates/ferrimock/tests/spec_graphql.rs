//! A GraphQL schema in, a working backend out.
//!
//! These run the whole path — SDL, entity graph, seeded store, executable
//! schema — and assert the three properties the design exists for: the
//! response answers the request that was asked, the same entity is the same
//! entity everywhere, and a write is visible to the next read.

#![cfg(feature = "spec")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use ferrimock::spec::bind::graphql::{GraphQLBackend, parse_request};
use ferrimock::spec::infer::graphql::{parse_sdl, to_entity_graph};
use ferrimock::spec::store::{EntityStore, StoreConfig};
use serde_json::{Value as JsonValue, json};

const BLOG: &str = r"
    interface Node { id: ID! }

    type User implements Node {
      id: ID!
      name: String!
      email: String
      address: Address
      posts: [Post!]!
    }

    type Address { city: String!, zip: String! }

    type Post implements Node {
      id: ID!
      title: String!
      status: Status!
      author: User!
    }

    enum Status { DRAFT PUBLISHED }

    input PostInput { title: String, status: Status }
    type CreatePostPayload { post: Post, errors: [String!]! }

    type Query {
      user(id: ID!): User
      users(first: Int, after: String): [User!]!
      post(id: ID!): Post
      posts(first: Int, status: Status): [Post!]!
      health: String!
    }

    type Mutation {
      createPost(input: PostInput!): CreatePostPayload
      updatePost(id: ID!, input: PostInput!): Post
      deletePost(id: ID!): Post
    }
";

fn backend(seed: u64) -> GraphQLBackend {
    build_backend(BLOG, seed, 4, 12)
}

fn build_backend(sdl: &str, seed: u64, users: usize, posts: usize) -> GraphQLBackend {
    let parsed = parse_sdl(sdl).expect("SDL parses");
    let graph = to_entity_graph(&parsed);
    let store = EntityStore::new(
        Arc::new(graph),
        StoreConfig::seeded(seed)
            .with_count("User", users)
            .with_count("Post", posts),
    );
    GraphQLBackend::build(&parsed, Arc::new(store)).expect("schema builds")
}

async fn run(backend: &GraphQLBackend, query: &str) -> JsonValue {
    run_with(backend, &json!({ "query": query })).await
}

async fn run_with(backend: &GraphQLBackend, body: &JsonValue) -> JsonValue {
    let request = parse_request(body.to_string().as_bytes()).expect("request parses");
    let response = backend.execute(request).await;
    assert!(
        response.errors.is_empty(),
        "unexpected GraphQL errors: {:?}",
        response.errors
    );
    serde_json::to_value(&response.data).expect("data serialises")
}

async fn run_expecting_errors(backend: &GraphQLBackend, query: &str) -> Vec<String> {
    let request = parse_request(json!({ "query": query }).to_string().as_bytes()).unwrap();
    let response = backend.execute(request).await;
    response.errors.iter().map(|e| e.message.clone()).collect()
}

#[tokio::test]
async fn the_response_answers_the_selection_that_was_asked() {
    let backend = backend(1);
    let data = run(&backend, "{ users(first: 2) { id } }").await;
    let users = data["users"].as_array().unwrap();
    assert_eq!(users.len(), 2);
    for user in users {
        assert_eq!(
            user.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["id"],
            "only the requested field should come back"
        );
    }

    let wider = run(&backend, "{ users(first: 1) { id name email } }").await;
    let user = &wider["users"][0];
    assert!(user.get("name").is_some());
    assert!(user.as_object().unwrap().contains_key("email"));
}

#[tokio::test]
async fn aliases_and_fragments_work() {
    let backend = backend(2);
    let data = run(
        &backend,
        r"
        { first: users(first: 1) { ...UserBits } }
        fragment UserBits on User { handle: id name }
        ",
    )
    .await;
    let user = &data["first"][0];
    assert!(user.get("handle").is_some(), "aliases must be honoured");
    assert!(user.get("name").is_some());
}

#[tokio::test]
async fn variables_are_coerced() {
    let backend = backend(3);
    let ids = run(&backend, "{ users(first: 1) { id } }").await;
    let id = ids["users"][0]["id"].as_str().unwrap().to_string();

    let data = run_with(
        &backend,
        &json!({
            "query": "query GetUser($id: ID!) { user(id: $id) { id name } }",
            "variables": { "id": id },
            "operationName": "GetUser",
        }),
    )
    .await;
    assert_eq!(data["user"]["id"].as_str().unwrap(), id);
}

#[tokio::test]
async fn skip_and_include_are_honoured() {
    let backend = backend(4);
    let data = run(
        &backend,
        "{ users(first: 1) { id name @skip(if: true) email @include(if: false) } }",
    )
    .await;
    let user = data["users"][0].as_object().unwrap();
    assert!(user.contains_key("id"));
    assert!(!user.contains_key("name"));
    assert!(!user.contains_key("email"));
}

#[tokio::test]
async fn the_same_entity_is_the_same_entity_everywhere() {
    let backend = backend(5);

    let listed = run(&backend, "{ users(first: 1) { id name email } }").await;
    let from_list = listed["users"][0].clone();
    let id = from_list["id"].as_str().unwrap();

    let fetched = run_with(
        &backend,
        &json!({
            "query": "query($id: ID!){ user(id: $id) { id name email } }",
            "variables": { "id": id },
        }),
    )
    .await;
    assert_eq!(
        fetched["user"], from_list,
        "a user fetched by id must match the one the list returned"
    );
}

#[tokio::test]
async fn asking_twice_gives_the_same_answer() {
    let backend = backend(6);
    let first = run(&backend, "{ posts(first: 3) { id title } }").await;
    let second = run(&backend, "{ posts(first: 3) { id title } }").await;
    assert_eq!(first, second);
}

#[tokio::test]
async fn the_same_seed_rebuilds_the_same_world() {
    let a = run(&backend(7), "{ posts(first: 3) { id title } }").await;
    let b = run(&backend(7), "{ posts(first: 3) { id title } }").await;
    assert_eq!(a, b);

    let different = run(&backend(8), "{ posts(first: 3) { id title } }").await;
    assert_ne!(a, different, "a different seed should give a different world");
}

#[tokio::test]
async fn relations_resolve_and_agree_in_both_directions() {
    let backend = backend(9);
    let data = run(
        &backend,
        "{ users { id posts { id author { id name } } } }",
    )
    .await;

    let mut seen_posts = 0;
    for user in data["users"].as_array().unwrap() {
        let user_id = user["id"].as_str().unwrap();
        for post in user["posts"].as_array().unwrap() {
            seen_posts += 1;
            assert_eq!(
                post["author"]["id"].as_str().unwrap(),
                user_id,
                "a post reached through user.posts must name that user as its author"
            );
            assert!(post["author"]["name"].is_string());
        }
    }
    assert!(seen_posts > 0, "the fixture should own some posts");
}

#[tokio::test]
async fn nesting_deeper_keeps_resolving() {
    let backend = backend(10);
    let data = run(
        &backend,
        "{ posts(first: 1) { author { posts { author { id } } } } }",
    )
    .await;
    let inner = &data["posts"][0]["author"]["posts"][0]["author"]["id"];
    assert!(inner.is_string(), "three hops should still resolve");
}

#[tokio::test]
async fn value_objects_are_inlined_rather_than_linked() {
    let backend = backend(11);
    let data = run(&backend, "{ users(first: 1) { address { city zip } } }").await;
    let address = &data["users"][0]["address"];
    assert!(address["city"].is_string());
    assert!(address["zip"].is_string());
}

#[tokio::test]
async fn enums_only_yield_declared_values() {
    let backend = backend(12);
    let data = run(&backend, "{ posts { status } }").await;
    for post in data["posts"].as_array().unwrap() {
        let status = post["status"].as_str().unwrap();
        assert!(
            status == "DRAFT" || status == "PUBLISHED",
            "unexpected enum value {status}"
        );
    }
}

#[tokio::test]
async fn a_missing_instance_is_null_rather_than_an_invention() {
    let backend = backend(13);
    let data = run_with(
        &backend,
        &json!({
            "query": "query($id: ID!){ user(id: $id) { id } }",
            "variables": { "id": "definitely-not-a-real-id" },
        }),
    )
    .await;
    assert_eq!(data["user"], JsonValue::Null);
}

#[tokio::test]
async fn a_write_is_visible_to_the_next_read() {
    let backend = backend(14);
    let before = run(&backend, "{ posts { id } }").await["posts"]
        .as_array()
        .unwrap()
        .len();

    let created = run_with(
        &backend,
        &json!({
            "query": "mutation($input: PostInput!){ createPost(input: $input) { post { id title } errors } }",
            "variables": { "input": { "title": "Hello world" } },
        }),
    )
    .await;

    let post = &created["createPost"]["post"];
    assert_eq!(post["title"].as_str().unwrap(), "Hello world");
    assert!(
        created["createPost"]["errors"].is_array(),
        "a non-null list field must be a list, not null"
    );
    let new_id = post["id"].as_str().unwrap().to_string();

    let after = run(&backend, "{ posts { id } }").await["posts"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after, before + 1);

    let fetched = run_with(
        &backend,
        &json!({
            "query": "query($id: ID!){ post(id: $id) { id title } }",
            "variables": { "id": new_id },
        }),
    )
    .await;
    assert_eq!(fetched["post"]["title"].as_str().unwrap(), "Hello world");
}

#[tokio::test]
async fn an_update_changes_what_the_next_read_sees() {
    let backend = backend(15);
    let id = run(&backend, "{ posts(first: 1) { id } }").await["posts"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    run_with(
        &backend,
        &json!({
            "query": "mutation($id: ID!, $input: PostInput!){ updatePost(id: $id, input: $input) { id title } }",
            "variables": { "id": id, "input": { "title": "Rewritten" } },
        }),
    )
    .await;

    let fetched = run_with(
        &backend,
        &json!({
            "query": "query($id: ID!){ post(id: $id) { title } }",
            "variables": { "id": id },
        }),
    )
    .await;
    assert_eq!(fetched["post"]["title"].as_str().unwrap(), "Rewritten");
}

#[tokio::test]
async fn a_delete_removes_it_from_reads_and_lists() {
    let backend = backend(16);
    let id = run(&backend, "{ posts(first: 1) { id } }").await["posts"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let deleted = run_with(
        &backend,
        &json!({
            "query": "mutation($id: ID!){ deletePost(id: $id) { id } }",
            "variables": { "id": id },
        }),
    )
    .await;
    assert_eq!(deleted["deletePost"]["id"].as_str().unwrap(), id);

    let remaining = run(&backend, "{ posts { id } }").await;
    assert!(
        !remaining["posts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"].as_str() == Some(id.as_str()))
    );
}

#[tokio::test]
async fn a_filter_argument_matching_a_field_narrows_the_list() {
    let backend = backend(17);
    let all = run(&backend, "{ posts { id status } }").await;
    let published = all["posts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["status"] == "PUBLISHED")
        .count();

    let filtered = run(&backend, "{ posts(status: PUBLISHED) { id status } }").await;
    let returned = filtered["posts"].as_array().unwrap();
    assert_eq!(returned.len(), published);
    assert!(returned.iter().all(|p| p["status"] == "PUBLISHED"));
}

#[tokio::test]
async fn pagination_walks_the_set_without_gaps_or_repeats() {
    let backend = backend(18);
    let mut seen: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let body = json!({
            "query": "query($after: String){ users(first: 2, after: $after) { id } }",
            "variables": { "after": cursor },
        });
        let page = run_with(&backend, &body).await;
        let users = page["users"].as_array().unwrap();
        if users.is_empty() {
            break;
        }
        for user in users {
            seen.push(user["id"].as_str().unwrap().to_string());
        }
        cursor = Some(seen.last().unwrap().clone());
        if seen.len() >= 4 {
            break;
        }
    }

    assert_eq!(seen.len(), 4);
    let unique: std::collections::HashSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len(), "pages must not overlap");
}

#[tokio::test]
async fn introspection_answers_so_tooling_can_point_at_the_mock() {
    let backend = backend(19);
    let data = run(
        &backend,
        "{ __schema { queryType { name } types { name kind } } }",
    )
    .await;
    assert_eq!(data["__schema"]["queryType"]["name"], "Query");

    let names: Vec<&str> = data["__schema"]["types"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for expected in ["User", "Post", "Status", "Address", "Node"] {
        assert!(names.contains(&expected), "`{expected}` missing from introspection");
    }

    let typed = run(&backend, "{ users(first: 1) { __typename id } }").await;
    assert_eq!(typed["users"][0]["__typename"], "User");
}

#[tokio::test]
async fn the_generated_sdl_still_describes_the_schema() {
    let backend = backend(20);
    let sdl = backend.sdl();
    assert!(sdl.contains("type User"));
    assert!(sdl.contains("type Post"));
    assert!(parse_sdl(&sdl).is_ok(), "the emitted SDL must be parseable");
}

#[tokio::test]
async fn an_unknown_field_is_a_validation_error_not_a_null() {
    let backend = backend(21);
    let errors = run_expecting_errors(&backend, "{ users { nope } }").await;
    assert!(!errors.is_empty(), "the schema should reject unknown fields");
}

#[tokio::test]
async fn a_field_nothing_could_be_inferred_about_is_counted() {
    let backend = backend(22);
    assert!(
        backend
            .coverage()
            .unclassified()
            .iter()
            .any(|f| f == "Query.health"),
        "a scalar root field cannot be store-backed and should be reported"
    );
    assert!(
        backend
            .coverage()
            .classified()
            .iter()
            .any(|f| f == "Query.user")
    );

    assert_eq!(backend.coverage().fallback_hits(), 0);
    let data = run(&backend, "{ health }").await;
    assert!(data["health"].is_string());
    assert_eq!(
        backend.coverage().fallback_hits(),
        1,
        "answering from the declared shape alone must be counted"
    );
}

#[tokio::test]
async fn coverage_reports_the_share_that_is_store_backed() {
    let backend = backend(23);
    let ratio = backend.coverage().ratio();
    assert!(ratio > 0.0 && ratio < 1.0);
    assert_eq!(
        backend.coverage().classified().len() + backend.coverage().unclassified().len(),
        8,
        "every root field should land on exactly one rung"
    );
}

#[tokio::test]
async fn a_relay_connection_resolves_edges_and_page_info() {
    const RELAY: &str = r"
        type User { id: ID!, name: String! }
        type UserConnection {
          edges: [UserEdge]
          pageInfo: PageInfo!
          totalCount: Int
        }
        type UserEdge { node: User, cursor: String! }
        type PageInfo {
          hasNextPage: Boolean!
          hasPreviousPage: Boolean!
          startCursor: String
          endCursor: String
        }
        type Query { userFeed(first: Int, after: String): UserConnection }
    ";

    let backend = build_backend(RELAY, 24, 6, 0);
    let data = run(
        &backend,
        "{ userFeed(first: 2) { totalCount edges { cursor node { id name } } pageInfo { hasNextPage endCursor } } }",
    )
    .await;

    let feed = &data["userFeed"];
    assert_eq!(feed["totalCount"].as_u64().unwrap(), 6);
    let edges = feed["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2);
    assert!(edges[0]["node"]["name"].is_string());
    assert_eq!(
        edges[0]["cursor"].as_str().unwrap(),
        edges[0]["node"]["id"].as_str().unwrap(),
        "the cursor should address the node it belongs to"
    );
    assert!(feed["pageInfo"]["hasNextPage"].as_bool().unwrap());

    let next_cursor = feed["pageInfo"]["endCursor"].as_str().unwrap().to_string();
    let second = run_with(
        &backend,
        &json!({
            "query": "query($after: String){ userFeed(first: 2, after: $after) { edges { node { id } } } }",
            "variables": { "after": next_cursor },
        }),
    )
    .await;
    let second_ids: Vec<_> = second["userFeed"]["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["node"]["id"].as_str().unwrap())
        .collect();
    let first_ids: Vec<_> = edges
        .iter()
        .map(|e| e["node"]["id"].as_str().unwrap())
        .collect();
    assert!(
        second_ids.iter().all(|id| !first_ids.contains(id)),
        "the second page must not repeat the first"
    );
}

#[tokio::test]
async fn a_schema_with_no_entities_still_builds_and_answers() {
    let backend = build_backend("type Query { ping: String!, count: Int! }", 25, 0, 0);
    let data = run(&backend, "{ ping count }").await;
    assert!(data["ping"].is_string());
    assert!(data["count"].is_number());
    assert_eq!(backend.coverage().ratio(), 0.0);
}

#[tokio::test]
async fn float_variables_survive_the_crossing() {
    const PRICED: &str = r"
        type Item { id: ID!, price: Float! }
        type Query { items(price: Float): [Item!]! }
    ";
    let backend = build_backend(PRICED, 26, 0, 0);
    let request = parse_request(
        json!({
            "query": "query($p: Float){ items(price: $p) { id } }",
            "variables": { "p": 12.5 },
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let response = backend.execute(request).await;
    assert!(
        response.errors.is_empty(),
        "a Float variable must not fail argument validation: {:?}",
        response.errors
    );
}
