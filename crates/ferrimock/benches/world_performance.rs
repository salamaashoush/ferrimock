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

criterion_group!(
    benches,
    bench_seeding,
    bench_count,
    bench_get,
    bench_list_page,
    bench_list_filtered,
    bench_write,
    bench_rebuild,
);
criterion_main!(benches);
