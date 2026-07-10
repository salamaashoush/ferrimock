#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use criterion::{Criterion, criterion_group, criterion_main};
use ferrimock::scripting::ScriptHost;
use ferrimock::types::{BodySource, DynamicResponse, HandlerFn, MockDefinition, RequestContext};
use std::hint::black_box;
use std::time::Duration;

const MOCKS_SOURCE: &str = r"
let count = 0;
http.get('/static', () => HttpResponse.json({ ok: true }));
http.get('/fake', () => HttpResponse.json({ id: fake.uuid(), name: fake.name() }));
http.get('/stateful', () => {
    count += 1;
    return HttpResponse.json({ count });
});
http.get('/async', async () => {
    await Promise.resolve();
    return HttpResponse.json({ ok: true });
});
";

fn handler_of(mock: &MockDefinition) -> HandlerFn {
    match &mock.response.body {
        BodySource::Handler(f) => std::sync::Arc::clone(f),
        other => panic!("expected handler body, got {other:?}"),
    }
}

fn request(path: &str) -> RequestContext {
    RequestContext {
        method: "GET".to_string(),
        uri: path.to_string(),
        path: path.to_string(),
        ..RequestContext::default()
    }
}

async fn call(handler: &HandlerFn, path: &str) -> DynamicResponse {
    handler(request(path)).await.expect("handler call")
}

fn bench_script_handlers(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bench.mjs");
    std::fs::write(&file, MOCKS_SOURCE).unwrap();

    let host = ScriptHost::new();
    let mocks = rt.block_on(host.load_file(&file, None)).unwrap();
    assert_eq!(mocks.len(), 4);
    let static_h = handler_of(&mocks[0]);
    let fake_h = handler_of(&mocks[1]);
    let stateful_h = handler_of(&mocks[2]);
    let async_h = handler_of(&mocks[3]);

    let mut group = c.benchmark_group("script_handlers");
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("js_static", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(call(&static_h, "/static").await) });
    });
    group.bench_function("js_fake_data", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(call(&fake_h, "/fake").await) });
    });
    group.bench_function("js_stateful", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(call(&stateful_h, "/stateful").await) });
    });
    group.bench_function("js_async_promise", |b| {
        b.to_async(&rt)
            .iter(|| async { black_box(call(&async_h, "/async").await) });
    });

    group.finish();
    drop(host);
}

criterion_group!(benches, bench_script_handlers);
criterion_main!(benches);
