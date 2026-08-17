//! What the entity world costs on the request path.
//!
//! The claims worth measuring are the ones the design rests on:
//!
//! - reads are derived, not materialised, so a large world is not a large
//!   allocation — `count` is arithmetic over the census;
//! - a rebuild is a load-time cost, not a request-time one;
//! - a schema-derived GraphQL answer is dominated by execution, not by the
//!   store underneath it.
//!
//! Numbers here are single-process and comparable only against each other.
//! Never quote one against another library's — see the benchmarking rules in
//! AGENTS.md.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;

use ferrimock::core::{EntityQuery, World, WorldSettings};

const SCHEMA: &str = r"
    type User {
      id: ID!
      name: String!
      email: String!
      bio: String
    }
    type Folder {
      id: ID!
      name: String!
      owner: User!
    }
    type Query {
      users: [User!]!
      user(id: ID!): User
      folders: [Folder!]!
    }
    type Mutation {
      createFolder(name: String!): Folder
    }
";

fn world_with(users: usize, folders: usize) -> Arc<World> {
    let world = Arc::new(World::new());
    world
        .configure(
            &WorldSettings {
                seed: Some(42),
                counts: [
                    (lean_string::LeanString::from("User"), users),
                    (lean_string::LeanString::from("Folder"), folders),
                ]
                .into_iter()
                .collect(),
                ..WorldSettings::default()
            },
            std::path::Path::new("bench"),
        )
        .unwrap();
    ferrimock::spec::source::load_schema(
        SCHEMA,
        std::path::Path::new("bench.graphql"),
        &world,
        false,
    )
    .unwrap();
    world
}

/// Building the world: the load-time cost of a schema, at several sizes.
///
/// The census is eager but tiny (keys only); records stay underived until read,
/// so this should scale with the key count rather than the field count.
fn bench_seeding(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/seed");
    for size in [100, 1_000, 10_000] {
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
            b.iter(|| black_box(world_with(size, size)));
        });
    }
    group.finish();
}

/// `count` is census arithmetic — no record is built.
fn bench_count(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/count");
    for size in [100, 10_000] {
        let world = world_with(size, size);
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| black_box(world.count("User")));
        });
    }
    group.finish();
}

/// Reading one record by key: the derivation cost of a single instance.
fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/get");
    for size in [100, 10_000] {
        let world = world_with(size, size);
        let key = world.store().keys("User")[size / 2].to_string();
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| black_box(world.get("User", &key)));
        });
    }
    group.finish();
}

/// A page off the front of a list.
///
/// Answered from the census: only the requested window is derived, so this is
/// dominated by the page size rather than the entity's. This benchmark is what
/// found the original behaviour — materialising every record before paginating
/// cost 21ms for 25 records out of 10,000.
///
/// Still not flat, because `keys` clones the entity's key vector to filter
/// tombstones out of it. That is the remaining O(n), and it is cheap next to
/// deriving records.
fn bench_list_page(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/list_page_25");
    for size in [100, 1_000, 10_000] {
        let world = world_with(size, 10);
        let query = EntityQuery {
            limit: Some(25),
            ..EntityQuery::default()
        };
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| black_box(world.list("User", &query).unwrap()));
        });
    }
    group.finish();
}

/// A filtered read, which is a scan.
///
/// Filtering and sorting have to see every candidate, so these do materialise
/// the entity before paginating. That is inherent, not an oversight — the
/// contrast with `list_page_25` is the point.
fn bench_list_filtered(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/list_filtered");
    for size in [100, 1_000] {
        let world = world_with(size, 10);
        let name = world
            .get("User", &world.store().keys("User")[0].to_string())
            .unwrap()["name"]
            .as_str()
            .unwrap()
            .to_string();
        let query = EntityQuery {
            filter: std::iter::once(("name".to_string(), serde_json::json!(name))).collect(),
            ..EntityQuery::default()
        };
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| black_box(world.list("User", &query).unwrap()));
        });
    }
    group.finish();
}

/// A write, and the read that follows it through the delta layer.
fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/write");
    let world = world_with(1_000, 100);

    group.bench_function("create", |b| {
        b.iter(|| {
            let created = world
                .create("User", serde_json::json!({ "name": "bench" }))
                .unwrap();
            black_box(&created);
            // Kept bounded: the delta is the only mutable state, and letting it
            // grow unboundedly would measure the delta, not the write.
            world
                .delete("User", created["id"].as_str().unwrap())
                .unwrap();
        });
    });

    let patched = world.store().keys("User")[0].to_string();
    group.bench_function("patch_then_read", |b| {
        b.iter(|| {
            world
                .update("User", &patched, serde_json::json!({ "name": "changed" }))
                .unwrap();
            black_box(world.get("User", &patched));
        });
    });

    group.finish();
    world.reset();
}

/// Rebuilding after a schema is added: the cost paid at load and hot reload,
/// never on the request path.
fn bench_rebuild(c: &mut Criterion) {
    let mut group = c.benchmark_group("world/rebuild");
    for size in [100, 1_000] {
        let world = world_with(size, size);
        // A delta to carry across, so this measures the replay too.
        for i in 0..10 {
            world
                .create(
                    "User",
                    serde_json::json!({ "name": format!("created-{i}") }),
                )
                .unwrap();
        }
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| black_box(world.rebuild().unwrap()));
        });
    }
    group.finish();
}

/// The OpenAPI document used for the REST benchmarks.
const DOCUMENT: &str = r#"
openapi: 3.0.3
info: { title: bench }
paths:
  /folders:
    get:
      operationId: listFolders
      parameters:
        - { name: limit, in: query, schema: { type: integer } }
        - { name: name, in: query, schema: { type: string } }
      responses:
        "200":
          content:
            application/json:
              schema:
                type: object
                properties:
                  entries:
                    type: array
                    items: { $ref: '#/components/schemas/Folder' }
                  total_count: { type: integer }
  /folders/{folder_id}:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /users/{user_id}:
    parameters:
      - { name: user_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getUser
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
components:
  schemas:
    Folder:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
        user_id: { type: string }
    User:
      type: object
      required: [id]
      properties:
        id: { type: string }
        name: { type: string }
"#;

fn rest_world(folders: usize) -> Arc<World> {
    let world = Arc::new(World::new());
    world
        .configure(
            &WorldSettings {
                seed: Some(42),
                counts: [
                    (lean_string::LeanString::from("User"), 50),
                    (lean_string::LeanString::from("Folder"), folders),
                ]
                .into_iter()
                .collect(),
                ..WorldSettings::default()
            },
            std::path::Path::new("bench"),
        )
        .unwrap();
    ferrimock::spec::source::load_schema(
        DOCUMENT,
        std::path::Path::new("bench.openapi.yaml"),
        &world,
        false,
    )
    .unwrap();
    world
}

fn rest_backend(world: &Arc<World>) -> ferrimock::spec::bind::rest::RestBackend {
    let table = Arc::new(
        ferrimock::spec::infer::openapi::parse_openapi(DOCUMENT)
            .unwrap()
            .0,
    );
    ferrimock::spec::bind::rest::RestBackend::build(&table, world)
}

fn request(path: &str, query: &str, captures: &[(&str, &str)]) -> ferrimock::types::RequestContext {
    let mut ctx = ferrimock::types::RequestContext::new();
    ctx.method = "GET".to_string();
    ctx.path = path.to_string();
    ctx.uri = path.to_string();
    ctx.query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    ctx.captures = captures
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    ctx
}

fn operation(
    backend: &ferrimock::spec::bind::rest::RestBackend,
    id: &str,
) -> Arc<ferrimock::spec::bind::rest::answer::BoundOperation> {
    Arc::clone(
        backend
            .operations
            .iter()
            .find(|operation| operation.id == id)
            .expect("the operation is mounted"),
    )
}

/// Mounting a document: the load-time cost of one bound operation per endpoint.
fn bench_rest_mount(c: &mut Criterion) {
    let mut group = c.benchmark_group("rest/mount");
    let world = rest_world(10_000);
    group.bench_function("3 operations over 10k folders", |b| {
        b.iter(|| black_box(rest_backend(&world).operations.len()));
    });
    group.finish();
}

/// Answering, at the sizes where the difference shows.
///
/// A lookup and an unfiltered page are answered from the census, so they should
/// be flat in the world's size. A *filtered* list scans, which is inherent —
/// the number here is what says whether that is acceptable.
fn bench_rest_answer(c: &mut Criterion) {
    let mut group = c.benchmark_group("rest/answer");
    for size in [100, 10_000] {
        let world = rest_world(size);
        let backend = rest_backend(&world);
        let key = world
            .list("Folder", &EntityQuery::default())
            .unwrap()
            .records[0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let name = world
            .list("Folder", &EntityQuery::default())
            .unwrap()
            .records[0]["name"]
            .as_str()
            .unwrap()
            .to_string();

        let get = operation(&backend, "getFolder");
        let ctx = request(&format!("/folders/{key}"), "", &[("folder_id", &key)]);
        group.bench_with_input(BenchmarkId::new("get", size), &size, |b, _| {
            b.iter(|| black_box(get.answer(&ctx)));
        });

        let list = operation(&backend, "listFolders");
        let paged = request("/folders", "limit=25", &[]);
        group.throughput(Throughput::Elements(25));
        group.bench_with_input(BenchmarkId::new("list_page_25", size), &size, |b, _| {
            b.iter(|| black_box(list.answer(&paged)));
        });

        let filtered = request("/folders", &format!("limit=25&name={name}"), &[]);
        group.bench_with_input(BenchmarkId::new("list_filtered_25", size), &size, |b, _| {
            b.iter(|| black_box(list.answer(&filtered)));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_seeding,
    bench_count,
    bench_get,
    bench_list_page,
    bench_list_filtered,
    bench_write,
    bench_rebuild,
    bench_rest_mount,
    bench_rest_answer,
);
criterion_main!(benches);
