# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Ferrimock is a high-performance HTTP mocking engine for Node.js, powered by Rust via NAPI. It provides an MSW-compatible API that is 3-4x faster than MSW, plus declarative YAML/JSON mocks with Tera template rendering and 115+ fake data generators.

## Workspace Structure

Monorepo with Cargo workspace (3 Rust crates) + bun workspaces (3 JS packages).

### Rust Crates

**ferrimock** (library) -- Core mock engine:
- `types` - Core types: RequestContext, URL patterns, matchers, body sources, HandlerFn
- `config` - Mock configuration parsing (YAML/JSON), HAR file loading
- `engine` - MockRegistry, MockMatcher, validation, scopes, call tracking
- `handler` - MSW-style handler builder API (http::get, graphql::query, etc.)
- `template` - Tera template rendering with 115+ fake data functions
- `fake_data` - Fake data generators: names, emails, UUIDs, images, PDFs
- `consolidator` - Smart mock consolidation with pattern detection
- `graphql` - GraphQL introspection parsing and mock generation
- `server` - HTTP server utilities: hot reload, graceful shutdown
- `api` - Mock management HTTP API (axum router)
- `recorder` - HTTP request/response recording
- `scripting` - JS-scripted mock handlers on embedded QuickJS (feature `scripting`)

**ferrimock-napi** (cdylib) -- Node.js NAPI bindings:
- `http_ns.rs` - `http.get/post/put/delete/patch/head/options/all` with RegExp, absolute URLs, `{ once }`
- `graphql_ns.rs` - `graphql.query/mutation/operation` (string or RegExp names, endpoint scoping)
- `response_ns.rs` - `HttpResponse.json/text/html/xml/arrayBuffer/redirect/error` builders
- `handler_bridge.rs` - HandlerFn (TSFN for server) + FunctionRef (direct call for interceptor)
- `request_context.rs` - RequestInfo / GraphQLRequestInfo resolver info (MSW shapes; `request` is a real Fetch Request)
- `server.rs` - FerrimockServer with FunctionRef-optimized matchRequest (fall-through/exclude support), use/resetHandlers/resetRuntimeHandlers/listHandlers
- `fake_ns.rs` - 115+ fake data generators exposed to JS

**ferrimock-cli** (binary) -- CLI for mock management and fake data generation.

### JavaScript Packages

**ferrimock** -- Main user-facing package:
- `node.ts` - setupServer (the MSW drop-in entry point, exported as `ferrimock/node`)
- `interceptor.ts` - FerrimockInterceptor (patches fetch/XHR/ClientRequest), fall-through loop, lifecycle events, boundary, onUnhandledRequest
- `http-response.ts` - HttpResponse class extending the native Response
- `registration.ts` - http/graphql factories (Response normalization, generators, graphql.link, collection window)
- `msw-compat.ts` - delay(), passthrough(), bypass() utilities
- `events.ts` - LifecycleEvents emitter (request:start/match/unhandled/end, response:mocked/bypass, unhandledException)
- `config.ts` / `loader.ts` - Config loading

**ferrimock** (npm) -- bare-specifier alias re-exporting ferrimock, so mock files
`import { http } from 'ferrimock'` in both Node and the embedded QuickJS runtime.
The only CLI is the Rust binary (ferrimock-cli).

**@ferrimock/playwright** -- Playwright fixture adapter.

## Essential Commands

```bash
# Rust
cargo check --workspace                          # Fast compile check
cargo test -p ferrimock --lib                       # Run Rust tests (607 tests)
cargo check -p ferrimock-napi                       # Check NAPI bindings

# Build native module
cd crates/ferrimock-napi && bunx @napi-rs/cli build --platform --release

# JavaScript tests
bun test ./packages/core/test/                    # All JS tests
bun test ./packages/core/test/msw-compat.test.ts  # MSW compatibility tests
bun test ./packages/core/test/interceptor.test.ts # Interceptor + benchmarks
bun test ./crates/ferrimock-napi/test/              # NAPI binding tests
```

## Architecture

### NAPI FunctionRef Optimization

The key performance optimization: `matchRequest()` uses `FunctionRef` to call JS handlers directly from the deferred resolver callback (~1us) instead of ThreadsafeFunction (~22us UV loop wakeup).

Flow:
1. `matchRequest()` called from JS
2. `spawn_future_with_callback` runs Rust matching on tokio
3. Deferred resolver runs on JS thread:
   - Declarative mock: response already built in Rust
   - Handler mock: `FunctionRef::borrow_back()` + `Function::call()` (~1us direct napi_call_function)
   - Async handlers: detected via `napi_is_promise`, chained with `PromiseRaw::then()`
4. Result: JS handler calls are 3-4x faster than MSW

Key files:
- `handler_bridge.rs` - TSFN (server mode) + FunctionRef (interceptor mode)
- `server.rs` - `match_request` with `MaybePromise` return type for sync/async handler support

### Mock Request Flow

1. Request arrives -> `MockMatcher::find_match()`
2. URL pattern matching (Express `:id`, Glob, Regex, Exact) by priority
3. Header/query/body/GraphQL matching evaluation
4. Once handlers auto-disable after first match
5. Response generation: inline, template (Tera), file, or handler (JS function)

### QuickJS Scripting (feature `scripting`)

`.js`/`.mjs`/`.ts`/`.mts` mock files run on embedded QuickJS (rquickjs 0.12,
`parallel` feature) — no Node needed. Architecture:

- rolldown bundler front-end (`scripting/bundle.rs`): TS transpile, node_modules +
  relative import resolution, tree-shaking, single ESM output; only the `ferrimock`
  specifier stays external (re-links against the runtime ModuleDef). Source maps
  remap error positions back to original files (`remap_error`).
- Bytecode disk cache (`scripting/bytecode_cache.rs`): `Module::write` output cached
  under an ABI-tagged dir (QuickJS version, crate version, arch, endianness, pointer
  width), validated by content hashes of every transitive input from the source map.
  `FERRIMOCK_CACHE_DIR` overrides location; `FERRIMOCK_NO_BYTECODE_CACHE` disables.
- GOTCHA: rolldown_common force-enables `serde_json/arbitrary_precision`
  workspace-wide, which breaks serde untagged-enum buffering on floats. HAR parsing
  goes through `config::parse_har` (AP-safe); never `serde_json::from_str::<Har>`.

- One `ScriptEngine` per script file (`scripting/host.rs`). Hot reload / poison
  recovery = drop the file's engine, re-evaluate on a fresh one. Module-scope state
  resets on reload.
- Single-owner VM event loop (`scripting/vm.rs`): exactly one never-completing tokio
  task polls the runtime scheduler; everything else submits jobs via `VmHandle`.
  Never use transient `async_with!` against the runtime — rquickjs's scheduler has a
  single waker slot and a short-lived poller kills it.
- `http.get(path, fn)` at evaluation time persists the handler into VM-side slots
  (`scripting/slots.rs`) and the loader builds normal `MockDefinition`s with
  `BodySource::Handler` — matching never crosses into JS.
- Two-layer timeout (`scripting/bridge.rs`): QuickJS interrupt handler kills runaway
  bytecode at `handler_timeout` (poisons the engine); a tokio backstop (+1s grace)
  frees requests parked on host awaits.
- `fake.*` dispatches through the same Tera function registry templates use
  (`scripting/bindings/fake.rs`) — one source of truth, embedder plugin functions
  (`register_template_function`) work from JS automatically.
- Tests: `tests/scripting_tests.rs`. Bench: `benches/script_performance.rs`
  (~10us per scripted handler call).

### MSW API Compatibility

Implemented (MSW and web-standard naming only; no aliases):
- `setupServer(...handlers)` from `ferrimock/node`: listen/close/use/resetHandlers(...next)/restoreHandlers/listHandlers/boundary/events
- `http.get/post/put/delete/patch/head/options/all` with string, RegExp, and absolute-URL predicates; `{ once: true }`
- `graphql.query/mutation/operation` (string or RegExp operation names) + `graphql.link(url)`
- `HttpResponse` (extends Response in Node; native class in QuickJS): json/text/html/xml/arrayBuffer/formData/redirect/error + constructor
- Resolver info: `{ request, params, cookies, requestId }`; GraphQL: `{ query, variables, operationName, cookies, request, requestId }`
- `undefined` return = fall-through to the next handler; generator resolvers
- `delay()`, `passthrough()`, `bypass()`
- Lifecycle events incl. `unhandledException`; `onUnhandledRequest` strategies
- `ReadableStream` response bodies: the interceptor delivers the handler's
  original Response (live stream, zero copies) via the stream stash; the
  standalone TCP server and the QuickJS lane deliver drained (buffered) bodies
- `request.formData()` + `HttpResponse.formData()`: native on Node (real
  Request/Response); native `FormData`/`File` classes + multipart/urlencoded
  codecs on the QuickJS lane

Not covered (by design): `setupWorker` (browser service worker; the engine is a
native addon).

## Code Standards

- Idiomatic Rust with zero-cost abstractions
- `anyhow::Result` for application code
- `unsafe` denied in ferrimock-napi (except marked `#[allow(unsafe_code)]` for NAPI FFI)
- FxHashMap for performance-critical paths (not std HashMap)
- All new code must include tests
- Run `cargo test -p ferrimock --lib` and `bun test` before committing
