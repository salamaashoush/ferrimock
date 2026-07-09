#![cfg(feature = "scripting")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::mem_forget
)]
//! End-to-end tests for JS-scripted mock handlers: load a `.js` mock
//! file onto a QuickJS engine, then drive the produced `MockDefinition`
//! handlers directly.

use mockpit::scripting::ScriptHost;
use mockpit::types::{BodySource, MockDefinition, RequestContext};

fn write_mock(dir: &std::path::Path, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, source).expect("write mock file");
    path
}

async fn call_handler(
    mock: &MockDefinition,
    ctx: RequestContext,
) -> mockpit::Result<mockpit::types::DynamicResponse> {
    match &mock.response.body {
        BodySource::Handler(f) => f(ctx).await,
        other => panic!("expected handler body, got {other:?}"),
    }
}

fn request(method: &str, path: &str) -> RequestContext {
    RequestContext {
        method: method.to_string(),
        uri: path.to_string(),
        path: path.to_string(),
        ..RequestContext::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn loads_handlers_from_globals_and_module_import() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "users.mjs",
        r"
import { http as mhttp, HttpResponse as HR } from 'mockpit';

http.get('/api/users/:id', ({ params }) => {
    return HttpResponse.json({ id: params.id, name: 'John' });
});

mhttp.post('/api/users', (req) => {
    return HR.json({ created: true }, { status: 201 });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 2);

    let mut ctx = request("GET", "/api/users/42");
    ctx.captures.insert("id".to_string(), "42".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["id"], "42");
    assert_eq!(body["name"], "John");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(200));

    let resp = call_handler(&mocks[1], request("POST", "/api/users"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(201));
}

#[tokio::test(flavor = "multi_thread")]
async fn async_handler_with_delay_and_fake_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "async.mjs",
        r"
http.get('/api/slow', async () => {
    await delay(10);
    const id = fake.uuid();
    return HttpResponse.json({ id });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 1);

    let started = std::time::Instant::now();
    let resp = call_handler(&mocks[0], request("GET", "/api/slow"))
        .await
        .expect("handler");
    assert!(started.elapsed() >= std::time::Duration::from_millis(10));
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    let id = body["id"].as_str().expect("uuid string");
    assert_eq!(id.len(), 36, "expected uuid, got {id}");
}

#[tokio::test(flavor = "multi_thread")]
async fn stateful_handlers_share_module_scope() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "counter.mjs",
        r"
let count = 0;
http.post('/api/count', () => {
    count += 1;
    return HttpResponse.json({ count });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    for expected in 1..=3 {
        let resp = call_handler(&mocks[0], request("POST", "/api/count"))
            .await
            .expect("handler");
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
        assert_eq!(body["count"], expected);
    }

    // Reload resets module state and replaces the engine.
    let mocks = host.load_file(&path, None).await.expect("reload");
    let resp = call_handler(&mocks[0], request("POST", "/api/count"))
        .await
        .expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_object_and_string_returns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "shapes.mjs",
        r"
http.get('/structured', () => ({ status: 418, body: { teapot: true } }));
http.get('/text', () => 'plain text');
http.get('/nothing', () => {});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 3);

    let resp = call_handler(&mocks[0], request("GET", "/structured"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(418));
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["teapot"], true);

    let resp = call_handler(&mocks[1], request("GET", "/text"))
        .await
        .expect("handler");
    assert_eq!(&resp.body[..], b"plain text");

    let resp = call_handler(&mocks[2], request("GET", "/nothing"))
        .await
        .expect("handler");
    assert!(resp.body.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn throwing_handler_surfaces_script_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "throws.mjs",
        r"
http.get('/boom', () => {
    throw new Error('kaboom');
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    let err = call_handler(&mocks[0], request("GET", "/boom"))
        .await
        .expect_err("should propagate throw");
    assert!(err.to_string().contains("kaboom"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn syntax_error_fails_load_with_location() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(dir.path(), "broken.mjs", "http.get('/x', () => {");

    let host = ScriptHost::new();
    let err = host
        .load_file(&path, None)
        .await
        .expect_err("syntax error should fail load");
    assert!(
        matches!(err, mockpit::MockpitError::Script(_)),
        "got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn imports_are_bundled_relative_and_outside_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("mocks/lib")).expect("mkdir");
    write_mock(
        dir.path(),
        "mocks/lib/helpers.mjs",
        "export const NAME = 'shared';",
    );
    // Imports above the mocks dir bundle too — resolution is rolldown's.
    write_mock(dir.path(), "outside.mjs", "export const WHERE = 'outside';");
    let path = write_mock(
        dir.path(),
        "mocks/main.mjs",
        r"
import { NAME } from './lib/helpers.mjs';
import { WHERE } from '../outside.mjs';
http.get('/name', () => HttpResponse.json({ name: NAME, where: WHERE }));
",
    );

    let host = ScriptHost::new();
    let mocks = host
        .load_file(&path, Some(&dir.path().join("mocks")))
        .await
        .expect("load with imports");
    let resp = call_handler(&mocks[0], request("GET", "/name"))
        .await
        .expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["name"], "shared");
    assert_eq!(body["where"], "outside");
}

#[tokio::test(flavor = "multi_thread")]
async fn typescript_mock_files_transpile_and_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "typed.ts",
        r"
interface User {
    id: string;
    name: string;
}

const seed: User[] = [{ id: '1', name: 'Ada' }];

http.get('/api/typed/:id', (req): unknown => {
    const found: User | undefined = seed.find((u) => u.id === req.params.id);
    return found
        ? HttpResponse.json(found)
        : HttpResponse.json({ error: 'not found' }, { status: 404 });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load ts");

    let mut ctx = request("GET", "/api/typed/1");
    ctx.captures.insert("id".to_string(), "1".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["name"], "Ada");

    let mut ctx = request("GET", "/api/typed/9");
    ctx.captures.insert("id".to_string(), "9".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(404));
}

#[tokio::test(flavor = "multi_thread")]
async fn node_modules_packages_resolve_and_bundle() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pkg = dir.path().join("node_modules/greeter");
    std::fs::create_dir_all(&pkg).expect("mkdir pkg");
    std::fs::write(
        pkg.join("package.json"),
        r#"{ "name": "greeter", "version": "1.0.0", "main": "index.js", "type": "module" }"#,
    )
    .expect("pkg json");
    std::fs::write(
        pkg.join("index.js"),
        "export function greet(name) { return `hello ${name}`; }",
    )
    .expect("pkg main");

    let path = write_mock(
        dir.path(),
        "uses-pkg.mjs",
        r"
import { greet } from 'greeter';
http.get('/greet/:name', ({ params }) => HttpResponse.json({ msg: greet(params.name) }));
",
    );

    let host = ScriptHost::new();
    let mocks = host
        .load_file(&path, Some(dir.path()))
        .await
        .expect("load with npm package");
    let mut ctx = request("GET", "/greet/world");
    ctx.captures.insert("name".to_string(), "world".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["msg"], "hello world");
}

#[tokio::test(flavor = "multi_thread")]
async fn bytecode_cache_reload_and_invalidation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(dir.path(), "cached.mjs", "http.get('/v', () => 'v1');");

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("first load");
    let resp = call_handler(&mocks[0], request("GET", "/v"))
        .await
        .expect("handler");
    assert_eq!(&resp.body[..], b"v1");

    // Unchanged file: reload goes through the cache-hit path.
    let mocks = host.load_file(&path, None).await.expect("cached reload");
    let resp = call_handler(&mocks[0], request("GET", "/v"))
        .await
        .expect("handler");
    assert_eq!(&resp.body[..], b"v1");

    // Edited file: the transitive-input hash check must miss and serve
    // the new behavior, never stale bytecode.
    std::fs::write(&path, "http.get('/v', () => 'v2');").expect("edit");
    let mocks = host
        .load_file(&path, None)
        .await
        .expect("invalidated reload");
    let resp = call_handler(&mocks[0], request("GET", "/v"))
        .await
        .expect("handler");
    assert_eq!(&resp.body[..], b"v2");
}

#[tokio::test(flavor = "multi_thread")]
async fn once_option_is_carried_onto_the_mock() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "once.mjs",
        "http.get('/one-shot', () => 'ok', { once: true });",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert!(mocks[0].once);
}

#[tokio::test(flavor = "multi_thread")]
async fn runaway_loop_is_halted_and_poisons_the_engine() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "spin.mjs",
        r"
http.get('/spin', () => {
    for (;;) {}
});
http.get('/fine', () => 'ok');
",
    );

    let host = ScriptHost::new();
    host.set_config(mockpit::scripting::ScriptEngineConfig {
        handler_timeout: std::time::Duration::from_millis(200),
        ..Default::default()
    });
    let mocks = host.load_file(&path, None).await.expect("load");

    let err = call_handler(&mocks[0], request("GET", "/spin"))
        .await
        .expect_err("runaway loop must be halted");
    assert!(
        matches!(err, mockpit::MockpitError::Script(_)),
        "got: {err}"
    );

    // Engine is poisoned: subsequent calls fail fast instead of hanging.
    let err = call_handler(&mocks[1], request("GET", "/fine"))
        .await
        .expect_err("poisoned engine should reject calls");
    assert!(err.to_string().contains("poisoned"), "got: {err}");

    // Reload recovers with a fresh engine.
    let mocks = host.load_file(&path, None).await.expect("reload");
    let resp = call_handler(&mocks[1], request("GET", "/fine"))
        .await
        .expect("fresh engine works");
    assert_eq!(&resp.body[..], b"ok");
}

#[tokio::test(flavor = "multi_thread")]
async fn regexp_paths_match_like_napi() {
    use http::Method;
    use mockpit::engine::{MockMatcher, MockRegistry};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "regex.mjs",
        r"http.get(/^\/api\/v\d+\/users$/i, () => HttpResponse.json({ matched: 'regex' }));",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 1);

    let registry = MockRegistry::new();
    for mock in mocks {
        registry.add_mock(mock);
    }
    let matcher = MockMatcher::new(registry);

    let headers = http::HeaderMap::new();
    assert!(
        matcher
            .find_match(&Method::GET, "/api/v3/users", None, &headers, None)
            .is_some()
    );
    // The `i` flag carries over.
    assert!(
        matcher
            .find_match(&Method::GET, "/API/V3/USERS", None, &headers, None)
            .is_some()
    );
    assert!(
        matcher
            .find_match(&Method::GET, "/api/users", None, &headers, None)
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_regexp_fails_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Lookahead is valid JS RegExp syntax but unsupported by the Rust
    // regex crate, so the load must fail loudly instead of matching all.
    let path = write_mock(
        dir.path(),
        "badregex.mjs",
        r"http.get(/^(?=nope)/, () => 'x');",
    );

    let host = ScriptHost::new();
    let err = host
        .load_file(&path, None)
        .await
        .expect_err("unsupported regex must fail load");
    assert!(
        matches!(err, mockpit::MockpitError::Script(_)),
        "got: {err}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn error_and_passthrough_set_marker_headers() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "markers.mjs",
        r"
http.get('/down', () => HttpResponse.error());
http.get('/skip', () => passthrough());
http.get('/bypassed', () => HttpResponse.json({ same: bypass('x') === 'x' }));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let resp = call_handler(&mocks[0], request("GET", "/down"))
        .await
        .expect("handler");
    assert_eq!(
        resp.headers
            .as_ref()
            .and_then(|h| h.get(mockpit::types::NETWORK_ERROR_HEADER))
            .map(String::as_str),
        Some("1")
    );

    let resp = call_handler(&mocks[1], request("GET", "/skip"))
        .await
        .expect("handler");
    assert_eq!(
        resp.headers
            .as_ref()
            .and_then(|h| h.get(mockpit::types::PASSTHROUGH_HEADER))
            .map(String::as_str),
        Some("1")
    );

    let resp = call_handler(&mocks[2], request("GET", "/bypassed"))
        .await
        .expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["same"], true);
}

#[cfg(feature = "server")]
mod serve_markers {
    use super::*;
    use http::Method;
    use mockpit::engine::{MockMatcher, MockRegistry};
    use mockpit::scripting::ScriptHost;

    async fn matcher_for(source: &str) -> MockMatcher {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_mock(dir.path(), "serve.mjs", source);
        let host = ScriptHost::new();
        let mocks = host.load_file(&path, None).await.expect("load");
        let registry = MockRegistry::new();
        for mock in mocks {
            registry.add_mock(mock);
        }
        // Engines live as long as the host; leak it so handlers stay
        // callable after this helper returns (test-only).
        std::mem::forget(host);
        MockMatcher::new(registry)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn passthrough_serves_unmatched_with_signal_header() {
        let matcher = matcher_for("http.get('/pt', () => passthrough());").await;
        let resp = mockpit::services::serve::respond(
            &matcher,
            &Method::GET,
            "/pt",
            None,
            &http::HeaderMap::new(),
            None,
        )
        .await;
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers()
                .get(mockpit::types::PASSTHROUGH_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn undefined_return_falls_through_to_next_handler() {
        let matcher = matcher_for(
            r"
http.get('/api/data', ({ request }) => {
    if (request.headers.get('x-special') === '1') { return HttpResponse.json({ from: 'special' }); }
    return undefined;
});
http.get('/api/data', () => HttpResponse.json({ from: 'default' }));
",
        )
        .await;

        let mut headers = http::HeaderMap::new();
        headers.insert("x-special", http::HeaderValue::from_static("1"));
        let resp = mockpit::services::serve::respond(
            &matcher,
            &Method::GET,
            "/api/data",
            None,
            &headers,
            None,
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["from"], "special");

        let resp = mockpit::services::serve::respond(
            &matcher,
            &Method::GET,
            "/api/data",
            None,
            &http::HeaderMap::new(),
            None,
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(json["from"], "default");

        // No handler responds -> unmatched 404.
        let mut headers = http::HeaderMap::new();
        headers.insert("x-skip-all", http::HeaderValue::from_static("1"));
        let resp = mockpit::services::serve::respond(
            &matcher,
            &Method::GET,
            "/api/missing",
            None,
            &headers,
            None,
        )
        .await;
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn once_handler_is_consumed_after_first_match() {
        let matcher = matcher_for(
            r"
http.get('/data', () => HttpResponse.json({ from: 'once' }), { once: true });
http.get('/data', () => HttpResponse.json({ from: 'fallback' }));
",
        )
        .await;

        for expected in ["once", "fallback", "fallback"] {
            let resp = mockpit::services::serve::respond(
                &matcher,
                &Method::GET,
                "/data",
                None,
                &http::HeaderMap::new(),
                None,
            )
            .await;
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .expect("body");
            let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
            assert_eq!(json["from"], expected);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn network_error_aborts_the_body_stream() {
        let matcher = matcher_for("http.get('/down', () => HttpResponse.error());").await;
        let resp = mockpit::services::serve::respond(
            &matcher,
            &Method::GET,
            "/down",
            None,
            &http::HeaderMap::new(),
            None,
        )
        .await;
        assert_eq!(resp.status(), http::StatusCode::OK);
        let read = axum::body::to_bytes(resp.into_body(), usize::MAX).await;
        assert!(
            read.is_err(),
            "body stream must error to simulate network failure"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_http_response_class() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "http_response.mjs",
        r#"
http.get('/api/ctor', () => new HttpResponse('teapot', {
    status: 418,
    statusText: "I'm a teapot",
    headers: { 'x-custom': 'yes' },
}));
http.get('/api/static', () => HttpResponse.json({ via: 'static' }, { status: 202 }));
http.get('/api/redirect', () => HttpResponse.redirect('/target', 307));
http.get('/api/text', () => HttpResponse.text('plain body'));
"#,
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 4);

    let resp = call_handler(&mocks[0], request("GET", "/api/ctor"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(418));
    assert_eq!(resp.status_text.as_deref(), Some("I'm a teapot"));
    let headers = resp.headers.expect("headers");
    assert_eq!(headers.get("x-custom").map(String::as_str), Some("yes"));
    assert_eq!(resp.body.as_ref(), b"teapot");

    let resp = call_handler(&mocks[1], request("GET", "/api/static"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(202));

    let resp = call_handler(&mocks[2], request("GET", "/api/redirect"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(307));
    assert_eq!(
        resp.headers
            .expect("headers")
            .get("location")
            .map(String::as_str),
        Some("/target")
    );

    let resp = call_handler(&mocks[3], request("GET", "/api/text"))
        .await
        .expect("handler");
    assert_eq!(resp.body.as_ref(), b"plain body");
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_resolver_info_request_view() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "request_view.mjs",
        r"
http.post('/api/users/:id', async ({ request, params, cookies, requestId }) => {
    const body = await request.json();
    const clone = request.clone();
    return HttpResponse.json({
        id: params.id,
        url: request.url,
        method: request.method,
        contentType: request.headers.get('Content-Type'),
        hasHeader: request.headers.has('COOKIE'),
        session: cookies.session,
        hasRequestId: typeof requestId === 'string' && requestId.length > 0,
        echo: body,
        text: await clone.text(),
        sameView: request === clone ? 'same' : 'copy',
    });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut ctx = request("POST", "/api/users/42");
    ctx.captures.insert("id".to_string(), "42".to_string());
    ctx.headers
        .insert("content-type".to_string(), "application/json".to_string());
    ctx.headers
        .insert("cookie".to_string(), "session=abc123".to_string());
    ctx.headers
        .insert("host".to_string(), "api.test".to_string());
    ctx.body = Some(r#"{"hello":"world"}"#.to_string());

    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json body");
    assert_eq!(body["id"], "42");
    assert_eq!(body["url"], "http://api.test/api/users/42");
    assert_eq!(body["method"], "POST");
    assert_eq!(body["contentType"], "application/json");
    assert_eq!(body["hasHeader"], true);
    assert_eq!(body["session"], "abc123");
    assert_eq!(body["hasRequestId"], true);
    assert_eq!(body["echo"]["hello"], "world");
    assert_eq!(body["text"], r#"{"hello":"world"}"#);
    assert_eq!(body["sameView"], "copy");
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_undefined_return_is_fallthrough() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "fallthrough.mjs",
        r"
http.get('/api/maybe', ({ request }) => {
    if (request.headers.get('x-special') === '1') { return HttpResponse.json({ special: true }); }
    return undefined;
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut ctx = request("GET", "/api/maybe");
    ctx.headers.insert("x-special".to_string(), "1".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    assert!(!resp.is_fallthrough());

    let resp = call_handler(&mocks[0], request("GET", "/api/maybe"))
        .await
        .expect("handler");
    assert!(resp.is_fallthrough(), "undefined return must fall through");
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_generator_resolvers_advance_and_stick() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "generator.mjs",
        r"
http.get('/api/poll', function* () {
    yield HttpResponse.json({ status: 'pending' });
    yield HttpResponse.json({ status: 'running' });
    return HttpResponse.json({ status: 'done' });
});
http.get('/api/async-poll', async function* () {
    yield HttpResponse.json({ n: 1 });
    return HttpResponse.json({ n: 2 });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut seen = Vec::new();
    for _ in 0..4 {
        let resp = call_handler(&mocks[0], request("GET", "/api/poll"))
            .await
            .expect("handler");
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
        seen.push(body["status"].as_str().expect("status").to_string());
    }
    assert_eq!(seen, ["pending", "running", "done", "done"]);

    let mut seen = Vec::new();
    for _ in 0..3 {
        let resp = call_handler(&mocks[1], request("GET", "/api/async-poll"))
            .await
            .expect("handler");
        let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
        seen.push(body["n"].as_i64().expect("n"));
    }
    assert_eq!(seen, [1, 2, 2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_graphql_resolver_info_and_link() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "graphql_info.mjs",
        r"
graphql.query('GetUser', ({ query, variables, operationName }) =>
    HttpResponse.json({ data: {
        op: operationName,
        id: variables.id,
        hasQuery: typeof query === 'string',
    }}));

const api = graphql.link('https://api.github.com/graphql');
api.query('GetRepo', () => HttpResponse.json({ data: { source: 'github' } }));

graphql.mutation(/^Update/, ({ operationName }) =>
    HttpResponse.json({ data: { matched: operationName } }));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 3);

    // Resolver info: query/variables/operationName from the request body.
    let mut ctx = request("POST", "/graphql");
    ctx.body = Some(
        r#"{"query":"query GetUser($id: ID!) { user(id: $id) { id } }","variables":{"id":"u1"}}"#
            .to_string(),
    );
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(body["data"]["op"], "GetUser");
    assert_eq!(body["data"]["id"], "u1");
    assert_eq!(body["data"]["hasQuery"], true);

    // graphql.link scopes via native matchers: exact path + Host header.
    let link_mock = &mocks[1];
    assert!(
        link_mock
            .request
            .url_patterns
            .iter()
            .any(|p| p.matches("/graphql")),
        "link mock must match the endpoint path"
    );
    assert_eq!(link_mock.request.header_matchers.len(), 1);

    // RegExp operation names compile into the native matcher.
    let regex_mock = &mocks[2];
    let gql = regex_mock
        .request
        .graphql_matcher
        .as_ref()
        .expect("graphql matcher");
    assert!(gql.operation_name_regex.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_readable_stream_response_bodies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "streams.mjs",
        r"
http.get('/api/sync-stream', () => {
    const stream = new ReadableStream({
        start(controller) {
            controller.enqueue('hello ');
            controller.enqueue('world');
            controller.close();
        },
    });
    return new HttpResponse(stream, { status: 200, headers: { 'content-type': 'text/plain' } });
});
http.get('/api/pull-stream', () => {
    let n = 0;
    const stream = new ReadableStream({
        async pull(controller) {
            await delay(5);
            n += 1;
            controller.enqueue(`chunk${n};`);
            if (n === 3) { controller.close(); }
        },
    });
    return new HttpResponse(stream);
});
http.get('/api/async-start-stream', () => {
    const stream = new ReadableStream({
        async start(controller) {
            controller.enqueue('a');
            await delay(5);
            controller.enqueue('b');
            controller.close();
        },
    });
    return new HttpResponse(stream);
});
http.get('/api/error-stream', () => {
    const stream = new ReadableStream({
        start(controller) {
            controller.enqueue('partial');
            controller.error('boom');
        },
    });
    return new HttpResponse(stream);
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 4);

    let resp = call_handler(&mocks[0], request("GET", "/api/sync-stream"))
        .await
        .expect("handler");
    assert_eq!(resp.body.as_ref(), b"hello world");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(200));

    let resp = call_handler(&mocks[1], request("GET", "/api/pull-stream"))
        .await
        .expect("handler");
    assert_eq!(resp.body.as_ref(), b"chunk1;chunk2;chunk3;");

    let resp = call_handler(&mocks[2], request("GET", "/api/async-start-stream"))
        .await
        .expect("handler");
    assert_eq!(resp.body.as_ref(), b"ab");

    let err = call_handler(&mocks[3], request("GET", "/api/error-stream"))
        .await
        .expect_err("errored stream must fail the response");
    assert!(err.to_string().contains("boom"), "got: {err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn msw_request_form_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "forms.mjs",
        r"
http.post('/api/urlencoded', async ({ request }) => {
    const form = await request.formData();
    return HttpResponse.json({
        user: form.get('user'),
        tags: form.getAll('tag'),
        decoded: form.get('note'),
        missing: form.get('nope'),
    });
});
http.post('/api/multipart', async ({ request }) => {
    const form = await request.formData();
    const file = form.get('upload');
    return HttpResponse.json({
        field: form.get('field'),
        fileName: file.name,
        fileType: file.type,
        fileText: file.text(),
        fileSize: file.size,
    });
});
http.get('/api/form-response', () => {
    const form = new FormData();
    form.append('greeting', 'hello');
    form.append('upload', new File(['file body'], 'notes.txt', { type: 'text/plain' }));
    return HttpResponse.formData(form);
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 3);

    let mut ctx = request("POST", "/api/urlencoded");
    ctx.headers.insert(
        "content-type".to_string(),
        "application/x-www-form-urlencoded".to_string(),
    );
    ctx.body = Some("user=ada&tag=a&tag=b&note=hello+world%21".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(body["user"], "ada");
    assert_eq!(body["tags"], serde_json::json!(["a", "b"]));
    assert_eq!(body["decoded"], "hello world!");
    assert_eq!(body["missing"], serde_json::Value::Null);

    let boundary = "----testboundary42";
    let multipart_body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"field\"\r\n\r\nvalue1\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"data.txt\"\r\nContent-Type: text/plain\r\n\r\nfile contents\r\n--{boundary}--\r\n"
    );
    let mut ctx = request("POST", "/api/multipart");
    ctx.headers.insert(
        "content-type".to_string(),
        format!("multipart/form-data; boundary={boundary}"),
    );
    ctx.body = Some(multipart_body);
    let resp = call_handler(&mocks[1], ctx).await.expect("handler");
    let body: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(body["field"], "value1");
    assert_eq!(body["fileName"], "data.txt");
    assert_eq!(body["fileType"], "text/plain");
    assert_eq!(body["fileText"], "file contents");
    assert_eq!(body["fileSize"], 13);

    // HttpResponse.formData round-trips through the request parser.
    let resp = call_handler(&mocks[2], request("GET", "/api/form-response"))
        .await
        .expect("handler");
    let content_type = resp
        .headers
        .as_ref()
        .and_then(|h| h.get("content-type"))
        .expect("content-type");
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .expect("boundary param");
    let body = String::from_utf8(resp.body.to_vec()).expect("utf8 body");
    assert!(body.contains(&format!("--{boundary}\r\n")));
    assert!(body.contains("Content-Disposition: form-data; name=\"greeting\""));
    assert!(body.contains("hello"));
    assert!(
        body.contains("Content-Disposition: form-data; name=\"upload\"; filename=\"notes.txt\"")
    );
    assert!(body.contains("Content-Type: text/plain"));
    assert!(body.contains("file body"));
    assert!(body.contains(&format!("--{boundary}--")));
}

#[tokio::test(flavor = "multi_thread")]
async fn all_http_verbs_register_handlers() {
    use http::Method;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "verbs.mjs",
        r"
http.put('/r', () => HttpResponse.json({ m: 'put' }));
http.patch('/r', () => HttpResponse.json({ m: 'patch' }));
http.delete('/r', () => HttpResponse.json({ m: 'delete' }));
http.head('/r', () => new HttpResponse(null, { status: 200 }));
http.options('/r', () => new HttpResponse(null, { status: 204 }));
http.all('/r', () => HttpResponse.json({ m: 'all' }));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 6);

    let expected = [
        (Some(Method::PUT), Some("put")),
        (Some(Method::PATCH), Some("patch")),
        (Some(Method::DELETE), Some("delete")),
        (Some(Method::HEAD), None),
        (Some(Method::OPTIONS), None),
        (None, Some("all")),
    ];
    for (mock, (method, body_marker)) in mocks.iter().zip(expected) {
        match &method {
            Some(m) => assert_eq!(mock.request.methods.as_slice(), std::slice::from_ref(m)),
            None => assert!(
                mock.request.methods.is_empty(),
                "http.all matches any method"
            ),
        }
        let resp = call_handler(
            mock,
            request(method.as_ref().map_or("GET", Method::as_str), "/r"),
        )
        .await
        .expect("handler");
        if let Some(marker) = body_marker {
            let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
            assert_eq!(json["m"], marker);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn html_xml_arraybuffer_statics_set_content_types() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "statics.mjs",
        r"
http.get('/h', () => HttpResponse.html('<p>hi</p>'));
http.get('/x', () => HttpResponse.xml('<a/>'));
http.get('/b', () => HttpResponse.arrayBuffer(new Uint8Array([1, 2, 3]).buffer));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let resp = call_handler(&mocks[0], request("GET", "/h"))
        .await
        .expect("handler");
    let headers = resp.headers.as_ref().expect("headers");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/html")
    );
    assert_eq!(&resp.body[..], b"<p>hi</p>");

    let resp = call_handler(&mocks[1], request("GET", "/x"))
        .await
        .expect("handler");
    let headers = resp.headers.as_ref().expect("headers");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("text/xml")
    );

    let resp = call_handler(&mocks[2], request("GET", "/b"))
        .await
        .expect("handler");
    let headers = resp.headers.as_ref().expect("headers");
    assert_eq!(
        headers.get("content-type").map(String::as_str),
        Some("application/octet-stream")
    );
    assert_eq!(&resp.body[..], &[1, 2, 3]);
}

#[tokio::test(flavor = "multi_thread")]
async fn response_global_aliases_http_response() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "response_alias.mjs",
        r"
http.get('/plain', () => new Response('hello', { status: 201 }));
http.get('/json', () => Response.json({ ok: true }));
http.get('/is', () => {
    const r = new Response(null);
    return HttpResponse.json({ same: Response === HttpResponse, instance: r instanceof HttpResponse });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let resp = call_handler(&mocks[0], request("GET", "/plain"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(201));
    assert_eq!(&resp.body[..], b"hello");

    let resp = call_handler(&mocks[1], request("GET", "/json"))
        .await
        .expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["ok"], true);

    let resp = call_handler(&mocks[2], request("GET", "/is"))
        .await
        .expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["same"], true);
    assert_eq!(json["instance"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn response_instance_exposes_read_api() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "read_api.mjs",
        r"
http.get('/read', () => {
    const r = HttpResponse.json({ n: 7 }, { status: 201, headers: { 'x-extra': 'yes' } });
    const err = HttpResponse.error();
    return HttpResponse.json({
        ok: r.ok,
        type: r.type,
        errType: err.type,
        contentType: r.headers.get('content-type'),
        extra: r.headers.get('x-extra'),
        body: r.json(),
        text: HttpResponse.text('plain').text(),
    });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    let resp = call_handler(&mocks[0], request("GET", "/read"))
        .await
        .expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["type"], "default");
    assert_eq!(json["errType"], "error");
    assert_eq!(json["contentType"], "application/json");
    assert_eq!(json["extra"], "yes");
    assert_eq!(json["body"]["n"], 7);
    assert_eq!(json["text"], "plain");
}

#[tokio::test(flavor = "multi_thread")]
async fn redirect_validates_status_code() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "redirect.mjs",
        r"
http.get('/bad', () => HttpResponse.redirect('/target', 200));
http.get('/ok', () => HttpResponse.redirect('/target', 308));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let err = call_handler(&mocks[0], request("GET", "/bad")).await;
    assert!(err.is_err(), "redirect(200) must throw a RangeError");

    let resp = call_handler(&mocks[1], request("GET", "/ok"))
        .await
        .expect("handler");
    assert_eq!(resp.status.map(|s| s.as_u16()), Some(308));
    let headers = resp.headers.as_ref().expect("headers");
    assert_eq!(headers.get("location").map(String::as_str), Some("/target"));
}

#[tokio::test(flavor = "multi_thread")]
async fn response_init_headers_accept_headers_instance_and_pairs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "init_headers.mjs",
        r"
http.get('/via-headers', () => {
    const h = new Headers({ 'X-From': 'headers-class' });
    h.append('Set-Cookie', 'a=1');
    h.append('Set-Cookie', 'b=2');
    return new HttpResponse('ok', { headers: h });
});
http.get('/via-pairs', () => new HttpResponse('ok', { headers: [['X-From', 'pairs'], ['X-Two', '2']] }));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let resp = call_handler(&mocks[0], request("GET", "/via-headers"))
        .await
        .expect("handler");
    let headers = resp.headers.as_ref().expect("headers");
    assert_eq!(
        headers.get("x-from").map(String::as_str),
        Some("headers-class")
    );
    assert_eq!(
        headers.get("set-cookie").map(String::as_str),
        Some("a=1\nb=2")
    );

    let resp = call_handler(&mocks[1], request("GET", "/via-pairs"))
        .await
        .expect("handler");
    let headers = resp.headers.as_ref().expect("headers");
    assert_eq!(headers.get("x-from").map(String::as_str), Some("pairs"));
    assert_eq!(headers.get("x-two").map(String::as_str), Some("2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn headers_class_is_mutable_and_iterable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "headers_mut.mjs",
        r"
http.get('/hdrs', ({ request }) => {
    const h = new Headers([['b', '2']]);
    h.set('A', '1');
    h.append('a', '3');
    h.delete('b');
    const spread = [...h].map(([k, v]) => `${k}=${v}`).join('|');
    const reqCanRead = request.headers.get('x-in');
    request.headers.set('x-in', 'mutated');
    return HttpResponse.json({
        combined: h.get('a'),
        has: h.has('B'),
        spread,
        setCookie: new Headers([['Set-Cookie', 'x=1']]).getSetCookie(),
        reqCanRead,
        reqMutated: request.headers.get('x-in'),
    });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut ctx = request("GET", "/hdrs");
    ctx.headers
        .insert("x-in".to_string(), "original".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["combined"], "1, 3");
    assert_eq!(json["has"], false);
    assert_eq!(json["spread"], "a=1, 3");
    assert_eq!(json["setCookie"][0], "x=1");
    assert_eq!(json["reqCanRead"], "original");
    assert_eq!(json["reqMutated"], "mutated");
}

#[tokio::test(flavor = "multi_thread")]
async fn url_and_search_params_globals() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "url_global.mjs",
        r"
http.get('/api/items', ({ request }) => {
    const url = new URL(request.url);
    const sp = new URLSearchParams('a=1&a=2&b=hello%20world');
    sp.append('c', '3');
    const rel = new URL('/sub/path?x=1', 'https://example.com:8443');
    return HttpResponse.json({
        pathname: url.pathname,
        host: url.host,
        q: url.searchParams.get('q'),
        origin: rel.origin,
        relPath: rel.pathname,
        port: rel.port,
        protocol: rel.protocol,
        all: sp.getAll('a'),
        decoded: sp.get('b'),
        size: sp.size,
        str: sp.toString(),
        spread: [...sp].length,
        canParse: URL.canParse('notaurl'),
    });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut ctx = request("GET", "/api/items");
    ctx.headers
        .insert("host".to_string(), "svc.test:9000".to_string());
    ctx.query.insert("q".to_string(), "find".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["pathname"], "/api/items");
    assert_eq!(json["host"], "svc.test:9000");
    assert_eq!(json["q"], "find");
    assert_eq!(json["origin"], "https://example.com:8443");
    assert_eq!(json["relPath"], "/sub/path");
    assert_eq!(json["port"], "8443");
    assert_eq!(json["protocol"], "https:");
    assert_eq!(json["all"][0], "1");
    assert_eq!(json["all"][1], "2");
    assert_eq!(json["decoded"], "hello world");
    assert_eq!(json["size"], 4);
    assert_eq!(json["str"], "a=1&a=2&b=hello%20world&c=3");
    assert_eq!(json["spread"], 4);
    assert_eq!(json["canParse"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn cookie_values_are_url_decoded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "cookies.mjs",
        r"
http.get('/c', ({ cookies }) => HttpResponse.json({ v: cookies.pref }));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut ctx = request("GET", "/c");
    ctx.headers
        .insert("cookie".to_string(), "pref=dark%20mode%3A1".to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["v"], "dark mode:1");
}

#[tokio::test(flavor = "multi_thread")]
async fn wildcard_segments_capture_into_numeric_params() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "wildcard.mjs",
        r"
http.get('/files/*', ({ params }) => HttpResponse.json({ splat: params['0'] }));
http.get('/a/*/b/*', ({ params }) => HttpResponse.json({ first: params['0'], second: params['1'] }));
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let captures = mocks[0].request.url_patterns[0]
        .extract_captures("/files/img/logo.png")
        .expect("wildcard captures");
    assert_eq!(captures.get("0").map(String::as_str), Some("img/logo.png"));

    let mut ctx = request("GET", "/files/img/logo.png");
    ctx.captures = captures;
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["splat"], "img/logo.png");

    let captures = mocks[1].request.url_patterns[0]
        .extract_captures("/a/x/b/y/z")
        .expect("multi wildcard captures");
    assert_eq!(captures.get("0").map(String::as_str), Some("x"));
    assert_eq!(captures.get("1").map(String::as_str), Some("y/z"));
}

#[tokio::test(flavor = "multi_thread")]
async fn graphql_operation_matches_any_operation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "gql_operation.mjs",
        r"
graphql.operation(({ operationName, variables }) => {
    return HttpResponse.json({ data: { op: operationName, vars: variables } });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");
    assert_eq!(mocks.len(), 1);
    assert!(
        mocks[0].request.graphql_matcher.is_some(),
        "graphql.operation registers a matcher"
    );

    let mut ctx = request("POST", "/graphql");
    ctx.body = Some(r#"{"query":"query Anything { id }","variables":{"x":1}}"#.to_string());
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["data"]["op"], "Anything");
    assert_eq!(json["data"]["vars"]["x"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_request_body_reaches_array_buffer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = write_mock(
        dir.path(),
        "binary_body.mjs",
        r"
http.post('/upload', ({ request }) => {
    const bytes = new Uint8Array(request.arrayBuffer());
    return HttpResponse.json({ len: bytes.length, first: bytes[0], last: bytes[bytes.length - 1] });
});
",
    );

    let host = ScriptHost::new();
    let mocks = host.load_file(&path, None).await.expect("load");

    let mut ctx = request("POST", "/upload");
    ctx.body_bytes = Some(bytes::Bytes::from_static(&[0xff, 0x00, 0x88, 0xfe]));
    let resp = call_handler(&mocks[0], ctx).await.expect("handler");
    let json: serde_json::Value = serde_json::from_slice(&resp.body).expect("json");
    assert_eq!(json["len"], 4);
    assert_eq!(json["first"], 255);
    assert_eq!(json["last"], 254);
}

#[cfg(feature = "server")]
mod streaming_scripts {
    use futures::{SinkExt, StreamExt};
    use mockpit::services::serve::{ServeInput, start};
    use tokio_tungstenite::tungstenite::Message;

    async fn serve_script(source: &str) -> (mockpit::services::serve::ServeHandle, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mocks.mjs"), source).expect("write script");
        let handle = start(ServeInput {
            port: 0,
            mocks_dir: Some(dir.path().to_string_lossy().into_owned()),
            ..ServeInput::default()
        })
        .await
        .expect("server start");
        // The tempdir must outlive the server (scripts reload from disk).
        std::mem::forget(dir);
        let ws_url = handle.url.replace("http://", "ws://");
        (handle, ws_url)
    }

    async fn http_get_stream(url: &str) -> (u16, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let url = url::Url::parse(url).expect("url");
        let host = url.host_str().expect("host");
        let port = url.port().expect("port");
        let mut stream = tokio::net::TcpStream::connect((host, port))
            .await
            .expect("tcp connect");
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n",
            url.path()
        );
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.expect("read");
        let text = String::from_utf8_lossy(&raw);
        let status: u16 = text
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .expect("status");
        let body = text
            .split("\r\n\r\n")
            .nth(1)
            .unwrap_or_default()
            .to_string();
        (status, body)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sse_resolver_sends_frames_and_closes() {
        let (handle, _) = serve_script(
            r"
sse('/api/stream', async ({ client }) => {
    client.send({ data: 'one' });
    await delay(20);
    client.send({ event: 'price', id: '7', data: { n: 1 } });
    client.send({ retry: 5000 });
    client.close();
});
",
        )
        .await;

        let started = std::time::Instant::now();
        let (status, body) = http_get_stream(&format!("{}/api/stream", handle.url)).await;
        assert_eq!(status, 200);
        assert!(started.elapsed() >= std::time::Duration::from_millis(20));

        // Handler-lane frames use MSW wire shape (no space after colon).
        assert!(body.contains("data:one\n\n"), "{body}");
        assert!(
            body.contains("id:7\nevent:price\ndata:{\"n\":1}\n\n"),
            "{body}"
        );
        assert!(body.contains("retry:5000\n\n"), "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sse_resolver_error_aborts_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (handle, _) = serve_script(
            r"
sse('/api/broken', async ({ client }) => {
    client.send({ data: 'before-error' });
    await delay(30);
    client.error();
});
",
        )
        .await;

        // The connection tears down mid-body: reading to end still works
        // at the TCP level (connection reset ends the read), but the body
        // must not terminate with a clean frame after 'before-error'.
        let url = url::Url::parse(&format!("{}/api/broken", handle.url)).expect("url");
        let mut stream = tokio::net::TcpStream::connect((
            url.host_str().expect("host"),
            url.port().expect("port"),
        ))
        .await
        .expect("tcp");
        let request = format!(
            "GET {} HTTP/1.1\r\nHost: h\r\nConnection: close\r\n\r\n",
            url.path()
        );
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut raw = Vec::new();
        let _ = stream.read_to_end(&mut raw).await; // reset acceptable
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("before-error"), "{text}");
        assert!(
            !text.contains("\r\n0\r\n\r\n"),
            "chunked stream must not terminate cleanly: {text}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_link_connection_listener_and_client_events() {
        let (_handle, ws_url) = serve_script(
            r"
const chat = ws.link('/ws/chat/:room');
chat.addEventListener('connection', ({ client, params }) => {
    client.send(JSON.stringify({ hello: params.room }));
    client.addEventListener('message', (event) => {
        if (event.data === 'bye') {
            client.close(4001, 'goodbye');
            return;
        }
        client.send('echo:' + event.data);
    });
});
",
        )
        .await;

        let (mut client, response) =
            tokio_tungstenite::connect_async(format!("{ws_url}/ws/chat/lobby"))
                .await
                .expect("connect");
        assert_eq!(response.status(), 101);

        let hello = match client.next().await.expect("frame").expect("ok") {
            Message::Text(text) => text.to_string(),
            other => panic!("expected text, got {other:?}"),
        };
        let hello: serde_json::Value = serde_json::from_str(&hello).expect("json");
        assert_eq!(hello["hello"], "lobby");

        client.send(Message::text("ping")).await.expect("send");
        let echoed = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert_eq!(echoed, "echo:ping");

        client.send(Message::text("bye")).await.expect("send");
        let close = loop {
            if let Message::Close(frame) = client.next().await.expect("frame").expect("ok") {
                break frame.expect("close frame");
            }
        };
        assert_eq!(u16::from(close.code), 4001);
        assert_eq!(close.reason.as_str(), "goodbye");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_binary_frames_cross_both_directions() {
        let (_handle, ws_url) = serve_script(
            r"
const bin = ws.link('/ws/bin');
bin.addEventListener('connection', ({ client }) => {
    client.addEventListener('message', (event) => {
        if (typeof event.data === 'string') {
            client.send(new Uint8Array([9, 8, 7]));
        } else {
            const bytes = new Uint8Array(event.data.buffer ?? event.data);
            client.send('len:' + bytes.length);
        }
    });
});
",
        )
        .await;

        let (mut client, _) = tokio_tungstenite::connect_async(format!("{ws_url}/ws/bin"))
            .await
            .expect("connect");

        client.send(Message::text("gimme")).await.expect("send");
        let binary = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Binary(bytes) => break bytes,
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected binary, got {other:?}"),
            }
        };
        assert_eq!(&binary[..], &[9, 8, 7]);

        client
            .send(Message::Binary(vec![1u8, 2, 3, 4].into()))
            .await
            .expect("send");
        let reply = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert_eq!(reply, "len:4");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_link_accepts_regexp_urls() {
        let (_handle, ws_url) = serve_script(
            r"
const live = ws.link(/wss?:\/\/[^/]+\/live\/\d+/);
live.addEventListener('connection', ({ client }) => {
    client.send('regex-hit');
});
",
        )
        .await;

        let (mut client, response) = tokio_tungstenite::connect_async(format!("{ws_url}/live/42"))
            .await
            .expect("connect");
        assert_eq!(response.status(), 101);
        let first = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert_eq!(first, "regex-hit");

        // Non-matching path: no ws mock -> plain 404, handshake fails.
        assert!(
            tokio_tungstenite::connect_async(format!("{ws_url}/other"))
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_close_events_carry_code_and_reason() {
        let (handle, ws_url) = serve_script(
            r"
let lastClose = null;
http.get('/last-close', () => HttpResponse.json(lastClose));
const chat = ws.link('/ws/chat');
chat.addEventListener('connection', ({ client }) => {
    client.addEventListener('close', (event) => {
        lastClose = { type: event.type, code: event.code, reason: event.reason };
    });
});
",
        )
        .await;

        let (mut client, _) = tokio_tungstenite::connect_async(format!("{ws_url}/ws/chat"))
            .await
            .expect("connect");
        client
            .send(Message::Close(Some(
                tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: 4005.into(),
                    reason: "later".into(),
                },
            )))
            .await
            .expect("send close");

        // The close listener runs as a VM job after the frame arrives.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let recorded = loop {
            let (status, body) = http_get_stream(&format!("{}/last-close", handle.url)).await;
            assert_eq!(status, 200);
            if body.contains("4005") {
                break body;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "close event never recorded: {body}"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        };
        // The raw body still carries chunked framing; extract the object.
        let json_start = recorded.find('{').expect("json body");
        let json_end = recorded.rfind('}').expect("json end");
        let json = recorded
            .get(json_start..=json_end)
            .expect("json byte range");
        let parsed: serde_json::Value = serde_json::from_str(json).expect("json");
        assert_eq!(parsed["type"], "close");
        assert_eq!(parsed["code"], 4005);
        assert_eq!(parsed["reason"], "later");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_connection_info_exposes_offered_protocols() {
        let (_handle, ws_url) = serve_script(
            r"
const proto = ws.link('/ws/proto');
proto.addEventListener('connection', ({ client, info }) => {
    client.send(JSON.stringify(info.protocols));
});
",
        )
        .await;

        let request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                format!("{ws_url}/ws/proto"),
            )
            .map(|mut req| {
                req.headers_mut().insert(
                    "sec-websocket-protocol",
                    "chat.v2, chat.v1".parse().expect("header"),
                );
                req
            })
            .expect("request");
        let (mut client, response) = tokio_tungstenite::connect_async(request)
            .await
            .expect("connect");
        // Handler lane confirms the first offered subprotocol.
        assert_eq!(
            response
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|v| v.to_str().ok()),
            Some("chat.v2")
        );

        let first = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        let protocols: Vec<String> = serde_json::from_str(&first).expect("json");
        assert_eq!(protocols, vec!["chat.v2", "chat.v1"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sse_server_connect_forwards_upstream_frames() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Real upstream: declarative SSE playback.
        let upstream_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            upstream_dir.path().join("upstream.yaml"),
            r#"
mocks:
  - id: real-sse
    match: { url: "/real" }
    sse:
      events:
        - { data: from-upstream }
        - { event: secret, data: hidden }
        - { data: tail }
"#,
        )
        .expect("write yaml");
        let upstream = start(ServeInput {
            port: 0,
            mocks_dir: Some(upstream_dir.path().to_string_lossy().into_owned()),
            ..ServeInput::default()
        })
        .await
        .expect("upstream start");

        // The sse handler's absolute URL doubles as the passthrough
        // target; the mock matches via its Host matcher.
        let (handle, _) = serve_script(&format!(
            r"
sse('{}/real', async ({{ client, server }}) => {{
    client.send({{ data: 'local-first' }});
    const source = server.connect();
    source.addEventListener('secret', (event) => {{
        event.preventDefault();
        client.send({{ data: 'redacted' }});
    }});
    source.addEventListener('error', () => {{
        client.close();
    }});
}});
",
            upstream.url
        ))
        .await;

        // Raw GET with the upstream's Host header (the absolute predicate
        // splits into Host matcher + path).
        let upstream_url = url::Url::parse(&upstream.url).expect("url");
        let upstream_host = format!(
            "{}:{}",
            upstream_url.host_str().expect("h"),
            upstream_url.port().expect("p")
        );
        let mock_url = url::Url::parse(&handle.url).expect("url");
        let mut stream = tokio::net::TcpStream::connect((
            mock_url.host_str().expect("h"),
            mock_url.port().expect("p"),
        ))
        .await
        .expect("tcp");
        let request = format!(
            "GET /real HTTP/1.1\r\nHost: {upstream_host}\r\nAccept: text/event-stream\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut raw = Vec::new();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stream.read_to_end(&mut raw),
        )
        .await;
        assert!(read.is_ok(), "sse passthrough stream never ended");
        let text = String::from_utf8_lossy(&raw);

        assert!(text.contains("data:local-first"), "{text}");
        // Unprevented upstream frames forward verbatim.
        assert!(text.contains("data:from-upstream"), "{text}");
        assert!(text.contains("data:tail"), "{text}");
        // The prevented frame was replaced by the listener's rewrite.
        assert!(text.contains("data:redacted"), "{text}");
        assert!(!text.contains("hidden"), "{text}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ws_server_connect_forwards_upstream_traffic() {
        // Upstream: declarative echo server.
        let upstream_dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            upstream_dir.path().join("upstream.yaml"),
            r"
mocks:
  - id: real
    match: { url: '/real' }
    ws:
      echo: true
      on_connect: [ { send: upstream-hello } ]
",
        )
        .expect("write yaml");
        let upstream = start(ServeInput {
            port: 0,
            mocks_dir: Some(upstream_dir.path().to_string_lossy().into_owned()),
            ..ServeInput::default()
        })
        .await
        .expect("upstream start");
        let upstream_ws = upstream.url.replace("http://", "ws://");

        let (_handle, ws_url) = serve_script(&format!(
            r"
const proxy = ws.link('{upstream_ws}/real');
proxy.addEventListener('connection', ({{ client, server }}) => {{
    server.connect();
    client.addEventListener('message', (event) => {{
        if (event.data === 'local') {{
            event.preventDefault();
            client.send('answered-locally');
        }}
    }});
    server.addEventListener('message', (event) => {{
        if (event.data === 'upstream-hello') {{
            event.preventDefault();
            client.send('rewrote-hello');
        }}
    }});
}});
",
        ))
        .await;

        let url = url::Url::parse(&upstream.url).expect("url");
        let host = format!("{}:{}", url.host_str().expect("h"), url.port().expect("p"));
        let request =
            tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(
                format!("{ws_url}/real"),
            )
            .map(|mut req| {
                req.headers_mut()
                    .insert("host", host.parse().expect("host header"));
                req
            })
            .expect("request");

        let (mut client, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("connect");

        // Upstream's greeting was intercepted and rewritten by the server
        // message listener (preventDefault suppressed forwarding).
        let first = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert_eq!(first, "rewrote-hello");

        // preventDefault on the client side answers locally (not forwarded).
        client.send(Message::text("local")).await.expect("send");
        let second = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert_eq!(second, "answered-locally");

        // Unprevented client frames forward upstream; the echo comes back.
        client.send(Message::text("roundtrip")).await.expect("send");
        let third = loop {
            match client.next().await.expect("frame").expect("ok") {
                Message::Text(text) => break text.to_string(),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("expected text, got {other:?}"),
            }
        };
        assert_eq!(third, "roundtrip");
    }
}
