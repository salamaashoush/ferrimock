//! What the proxy costs.
//!
//! Every arm here measures the *same* upstream through the *same* client, so
//! the only difference between "direct" and the proxy arms is the hop. A
//! number quoted without its direct baseline says nothing: most of what these
//! measure is loopback TCP and the client, not the proxy.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ferrimock::engine::{MockMatcher, MockRegistry};
use ferrimock::proxy::{ProxyConfig, ProxyHandle, RouteConfig};
use http_body_util::{Full, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Body sizes worth separating: a header-dominated response, a typical JSON
/// payload, and one large enough that copying it would show up.
const SIZES: [(&str, usize); 3] = [("1KB", 1024), ("64KB", 64 * 1024), ("1MB", 1024 * 1024)];

/// An upstream that answers instantly, so the measurement is the path and not
/// the origin.
async fn upstream() -> SocketAddr {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let _ = stream.set_nodelay(true);
            tokio::spawn(async move {
                let service = hyper::service::service_fn(|request: Request<Incoming>| async move {
                    let size = request
                        .uri()
                        .path()
                        .trim_start_matches('/')
                        .parse::<usize>()
                        .unwrap_or(0);
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header(http::header::CONTENT_TYPE, "application/octet-stream")
                            .body(BoxBody::new(Full::new(Bytes::from(vec![b'x'; size]))))
                            .unwrap(),
                    )
                });

                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
            });
        }
    });

    addr
}

async fn proxy_for(upstream: SocketAddr, matcher: Option<MockMatcher>) -> ProxyHandle {
    let mut config = ProxyConfig {
        listen: ([127, 0, 0, 1], 0).into(),
        routes: vec![RouteConfig::parse(&format!("/=http://{upstream}")).unwrap()],
        ..ProxyConfig::default()
    };
    config.compile();
    ferrimock::proxy::start(config, matcher).await.unwrap()
}

/// One registry with a mock that matches nothing the benchmark requests, so
/// every request pays the match attempt and then forwards. This is the cost a
/// proxy with mocks loaded actually carries.
async fn registry_with_a_miss() -> MockRegistry {
    let registry = MockRegistry::new();
    let file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
    std::fs::write(
        file.path(),
        "mocks:\n  - id: never\n    match:\n      GET: /this-path-is-never-requested\n    response:\n      json: { ok: true }\n",
    )
    .unwrap();
    registry.load_collection_file(file.path()).await.unwrap();
    registry
}

fn get(client: &reqwest::Client, url: &str) -> impl Future<Output = usize> + Send {
    let pending = client.get(url).send();
    async move {
        let response = pending.await.expect("request failed");
        response.bytes().await.expect("body failed").len()
    }
}

fn bench_forwarding(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let client = Arc::new(reqwest::Client::builder().build().unwrap());

    let (origin, plain, with_mocks) = runtime.block_on(async {
        let origin = upstream().await;
        let plain = proxy_for(origin, None).await;
        let matcher = MockMatcher::new(registry_with_a_miss().await);
        let with_mocks = proxy_for(origin, Some(matcher)).await;
        (origin, plain, with_mocks)
    });

    let direct_url = format!("http://{origin}");
    let plain_url = plain.url();
    let mocked_url = with_mocks.url();

    let mut group = c.benchmark_group("proxy/forward");
    for (label, size) in SIZES {
        group.throughput(Throughput::Bytes(size as u64));

        // Built once per arm rather than per iteration: formatting a URL is
        // not what this is measuring.
        for (arm, base) in [
            ("direct", &direct_url),
            ("proxy", &plain_url),
            ("proxy+mock-miss", &mocked_url),
        ] {
            let url = format!("{base}/{size}");
            group.bench_with_input(BenchmarkId::new(arm, label), &url, |b, url| {
                b.to_async(&runtime).iter(|| get(&client, url));
            });
        }
    }
    group.finish();
}

fn bench_mock_hit(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let client = Arc::new(reqwest::Client::builder().build().unwrap());

    let proxy = runtime.block_on(async {
        let origin = upstream().await;
        let registry = MockRegistry::new();
        let file = tempfile::Builder::new().suffix(".yaml").tempfile().unwrap();
        std::fs::write(
            file.path(),
            "mocks:\n  - id: hit\n    match:\n      GET: /mocked\n    response:\n      json: { ok: true }\n",
        )
        .unwrap();
        registry.load_collection_file(file.path()).await.unwrap();
        proxy_for(origin, Some(MockMatcher::new(registry))).await
    });

    let url = format!("{}/mocked", proxy.url());

    // A mock hit never reaches the upstream, so this is the whole cost of the
    // proxy answering: accept, match, generate, write.
    c.bench_function("proxy/mock_hit", |b| {
        b.to_async(&runtime).iter(|| get(&client, &url));
    });
}

criterion_group!(benches, bench_forwarding, bench_mock_hit);
criterion_main!(benches);
