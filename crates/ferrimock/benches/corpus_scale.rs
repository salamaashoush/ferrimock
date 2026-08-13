#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! What matching, diagnosing and suggesting cost on a registry the size and
//! shape a real recording produces.
//!
//! The other benchmarks measure a registry built for the measurement: uniform
//! mocks, plain paths, everything enabled for the whole run. A converted
//! recording is none of those. It is several hundred exact request lines, most
//! of them carrying a query, a share of them GraphQL operations sharing one
//! endpoint, and — where the recording answered the same call differently as it
//! went on — chains of one-shot mocks that retire as the replay runs and
//! invalidate the registry's sorted view each time they do.
//!
//! `explain` and `suggest` are here because they walk the whole registry per
//! miss. They are off the hot path, but a developer-facing server turns them on
//! and then pays for them on every request it does not answer.

use criterion::{Criterion, criterion_group, criterion_main};
use ferrimock::engine::registry::UnmatchedRequest;
use ferrimock::engine::types::{
    BodySource, MockDefinition, QueryMatcher, RequestMatcher, ResponseGenerator, UrlPattern,
};
use ferrimock::engine::{MockMatcher, MockRegistry, suggest};
use http::{HeaderMap, Method, StatusCode};
use smallvec::smallvec;
use std::hint::black_box;

/// Mocks per registry. A recording of a single page interaction converts to a
/// few hundred, so this is the upper end of one recording rather than a whole
/// suite of them.
const CORPUS_SIZE: usize = 700;

/// A request line in the shape recordings actually produce.
fn request_line(i: usize) -> String {
    match i % 4 {
        0 => format!("/api/folders/{i}/items"),
        1 => format!("/api/activity?cursor=eyJwIjoxfQ%3D%3D&pageSize={i}"),
        2 => format!("/api/labels:bulk?items=item_{i}%2Citem_{}", i + 1),
        _ => {
            format!("/api/attributes:bulk?fileIds={i}&template=review")
        }
    }
}

fn mock(id: usize, url: &str, once: bool, pinned: Option<(&str, &str)>) -> MockDefinition {
    MockDefinition {
        id: format!("har-entry-{id}").into(),
        priority: if url.contains('?') { 200 } else { 100 },
        enabled: true,
        once,
        scope: None,
        source_file: None,
        request_transforms: None,
        request: RequestMatcher {
            methods: smallvec![Method::GET],
            url_patterns: smallvec![UrlPattern::exact(url)],
            header_matchers: smallvec![],
            query_matchers: pinned
                .map(|(k, v)| smallvec![QueryMatcher::exact(k, v)])
                .unwrap_or_default(),
            body_matcher: None,
            graphql_matcher: None,
        },
        response: ResponseGenerator::new(StatusCode::OK, BodySource::inline(r#"{"ok":true}"#)),
        vars: None,
        streaming: None,
    }
}

/// A registry shaped like a converted recording. `sequenced` adds the one-shot
/// chains that a recording of a changing endpoint produces.
fn corpus_registry(sequenced: bool) -> MockRegistry {
    let registry = MockRegistry::new();
    for i in 0..CORPUS_SIZE {
        registry.add_mock(mock(i, &request_line(i), false, None));
    }
    // A handful of endpoints answered differently each time they were called.
    if sequenced {
        for step in 0..3 {
            registry.add_mock(mock(CORPUS_SIZE + step, "/api/uploads", step < 2, None));
        }
    }
    // The credential-redacted shape: path only, surviving parameter pinned.
    registry.add_mock(mock(
        CORPUS_SIZE + 10,
        "/api/files",
        false,
        Some(("fields", "name")),
    ));
    registry
}

fn split(line: &str) -> (&str, Option<&str>) {
    match line.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (line, None),
    }
}

fn bench_match(c: &mut Criterion) {
    let matcher = MockMatcher::new(corpus_registry(false));
    let headers = HeaderMap::new();

    let hit = request_line(CORPUS_SIZE - 3);
    let (hit_path, hit_query) = split(&hit);
    c.bench_function("corpus/match_hit", |b| {
        b.iter(|| {
            black_box(matcher.find_match(
                &Method::GET,
                black_box(hit_path),
                black_box(hit_query),
                &headers,
                None,
            ))
        });
    });

    c.bench_function("corpus/match_miss", |b| {
        b.iter(|| {
            black_box(matcher.find_match(
                &Method::GET,
                black_box("/api/folders/999999/items"),
                None,
                &headers,
                None,
            ))
        });
    });
}

fn bench_diagnostics(c: &mut Criterion) {
    let matcher = MockMatcher::new(corpus_registry(false));
    let headers = HeaderMap::new();

    c.bench_function("corpus/explain_miss", |b| {
        b.iter(|| {
            black_box(matcher.explain(
                &Method::GET,
                black_box("/api/activity"),
                black_box(Some("cursor=unrecorded&pageSize=6")),
                &headers,
                None,
            ))
        });
    });

    let now = chrono::Utc::now();
    let unmatched: Vec<UnmatchedRequest> = (0..50)
        .map(|i| UnmatchedRequest {
            method: "GET".to_string(),
            path: "/api/activity".to_string(),
            query: Some(format!("cursor=unrecorded{i}&pageSize=6")),
            count: 1,
            first_seen: now,
            last_seen: now,
        })
        .collect();

    c.bench_function("corpus/suggest_50_misses", |b| {
        b.iter(|| black_box(suggest(&matcher, black_box(&unmatched))));
    });
}

/// Replaying a recording end to end, with and without the one-shot chains.
/// Retiring a mock invalidates the registry's sorted view, so this is where
/// sequencing would show up if it cost anything.
fn bench_replay(c: &mut Criterion) {
    let headers = HeaderMap::new();
    let lines: Vec<String> = (0..CORPUS_SIZE).map(request_line).collect();

    for sequenced in [false, true] {
        let name = if sequenced {
            "corpus/replay_sequenced"
        } else {
            "corpus/replay_flat"
        };
        c.bench_function(name, |b| {
            b.iter_batched(
                || MockMatcher::new(corpus_registry(sequenced)),
                |matcher| {
                    for line in &lines {
                        let (path, query) = split(line);
                        black_box(matcher.find_match(&Method::GET, path, query, &headers, None));
                    }
                    if sequenced {
                        for _ in 0..3 {
                            black_box(matcher.find_match(
                                &Method::GET,
                                "/api/uploads",
                                None,
                                &headers,
                                None,
                            ));
                        }
                    }
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
}

criterion_group!(benches, bench_match, bench_diagnostics, bench_replay);
criterion_main!(benches);
