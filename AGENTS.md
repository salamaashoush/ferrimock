# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Ferrimock is a high-performance HTTP mocking engine for Node.js, powered by Rust via NAPI. It provides an MSW-compatible API that is 1.1-1.7x faster than MSW on the interception path (see Benchmarking), plus declarative YAML/JSON mocks with Tera template rendering and 115+ fake data generators.

## Workspace Structure

Monorepo with Cargo workspace (3 Rust crates) + bun workspaces (3 JS packages).

### Rust Crates

**ferrimock** (library) -- Core mock engine:
- `types` - Core types: RequestContext, URL patterns, matchers, body sources, HandlerFn
- `config` - Mock configuration parsing (YAML/JSON), HAR file loading.
  `network_error: true` desugars to the marker-header template the server and the
  interceptor already honour — faults add no runtime path
- `engine` - MockRegistry, MockMatcher, validation, scopes, call tracking
- `engine::registry` match counting - every match increments a monotonic per-mock
  counter (always on, one relaxed add). `match_count`/`verify` are the assertion
  surface; `get_call_count` is the *retained* call-tracking buffer and plateaus at
  its window, so never assert on it
- `engine::diagnostics` - `MockMatcher::explain()`: per-criterion match reports and
  ranked near misses. Evaluates through the matcher's own predicates — never
  reimplement matching logic in a renderer (the CLI used to, and drifted)
- `handler` - MSW-style handler builder API (http::get, graphql::query, etc.)
- `template` - Tera template rendering with 115+ fake data functions
- `fake_data` - Fake data generators: names, emails, UUIDs, images, PDFs
- `fake_data::rng` - Seedable random source behind every generator, template
  function and filter. Unseeded it delegates to `rand::rng()`; seeded, draws come
  from a thread-scoped stream (installed per mock id by `template::renderer`) or
  a process-wide stream. Generators must never call `rand::rng()` directly —
  that bypasses `--seed`.
- `core::world` - The entity world: the merged `EntityGraph` (types, keys,
  relations) and the seeded `EntityStore` over it. Owned by `MockRegistry`
  beside `PersistenceStore` — one is untyped scratch state, the other is the
  world a mocked API pretends to have. A spec *populates* it; nothing owns it,
  which is what lets a template, a script and a schema-derived route read and
  write the same entities. Rule of thumb: if it has a type and a key in the API
  you are mocking, it is the world; if it is a counter or a flag for your test,
  it is the store.
- `core::world::store` - Three layers, one mutable: *census* (how many instances
  and their keys, eager and tiny), *base* (field values derived from seed +
  entity + ordinal + field path — pure, never stored), *delta* (creations,
  patches, tombstones). So the world is deterministic given the seed, and the
  state is deterministic given the seed plus the sequence of writes.
- `consolidator` - Smart mock consolidation with pattern detection
- `graphql` - GraphQL introspection parsing and mock generation
- `server` - HTTP server utilities: hot reload, graceful shutdown
- `api` - Mock management HTTP API (axum router)
- `recorder` - HTTP request/response recording
- `scripting` - JS-scripted mock handlers on embedded QuickJS (feature `scripting`)
- `spec` - Reading a schema into the world and binding it to a protocol
  (feature `spec`). `infer` (SDL -> entity graph), `bind` (graph + store ->
  executable GraphQL schema), `emit` (backend -> ordinary `MockDefinition`).
  Deliberately holds no state: the world lives in `core`.

**ferrimock-napi** (cdylib) -- Node.js NAPI bindings:
- `http_ns.rs` - `http.get/post/put/delete/patch/head/options/all` with RegExp, absolute URLs, `{ once }`
- `graphql_ns.rs` - `graphql.query/mutation/operation` (string or RegExp names, endpoint scoping)
- `response_ns.rs` - `HttpResponse.json/text/html/xml/arrayBuffer/redirect/error` builders
- `handler_bridge.rs` - HandlerFn (TSFN for server) + FunctionRef (direct call for interceptor)
- `request_context.rs` - RequestInfo / GraphQLRequestInfo resolver info (MSW shapes; `request` is a real Fetch Request)
- `server.rs` - FerrimockServer with FunctionRef-optimized matchRequest (fall-through/exclude support), use/resetHandlers/resetRuntimeHandlers/listHandlers
- `fake_ns.rs` - 115+ fake data generators exposed to JS
- `world_ns.rs` - `world.types/count/get/list/related/create/update/replace/delete/reset/pendingWrites`
  over the engine's entity world, mirroring the QuickJS `world.*` surface so a
  handler behaves the same on either runtime. Synchronous (a DashMap read behind
  an Arc). The addon enables the `spec` feature; without it the loader would
  ignore every `.graphql` and the world would always be empty under Node.

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
`world` is exported here too; note `crates/ferrimock-napi/index.mjs` is a
hand-maintained ESM shim listing each named export, while `index.js` and
`index.d.ts` are generated by the napi CLI and must never be hand-edited.
The only CLI is the Rust binary (ferrimock-cli).

**@ferrimock/playwright** -- Playwright fixture adapter.

## Essential Commands

```bash
# Rust
cargo check --workspace                          # Fast compile check
cargo test -p ferrimock --lib                       # Run Rust unit tests
cargo test --workspace --all-features               # Everything
cargo check -p ferrimock-napi                       # Check NAPI bindings

# Build native module
cd crates/ferrimock-napi && bunx @napi-rs/cli build --platform --release

# JavaScript tests
bun test ./packages/core/test/                    # All JS tests
bun test ./crates/ferrimock-napi/test/world.test.ts  # Entity world from Node
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
4. Result: JS handler calls are 1.1-1.7x faster than MSW, depending on runtime and scenario

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

### Spec-derived backends and the entity world (feature `spec`)

A schema is a type system, not a list of observations, so it does not compile to
independent mocks. It compiles into the engine's **world** — entity types, keys,
relations — which a seeded store answers queries against and which protocol
bindings serve.

The split that keeps this one system rather than two:

- **A schema declares entities, never routes.** It has nowhere to write down
  that it is served at `https://api.example.com/graphql` rather than on localhost,
  and guessing is how a proxy answers on the wrong host. Loading a `.graphql`
  populates the world and registers *zero* mocks; the loader warns when a world
  has entities but nothing serves them.
- **A route is a mock.** `serve:` is a mode alongside `response:`, `patch:`,
  `sse:` and `ws:` — not a response body, but a protocol behavior bound to a
  matched URL, exactly like `ws:`. `match` says where the API answers, `serve`
  says which schema answers there.
- **The world is not the spec's.** It lives in `core`, is owned by
  `MockRegistry`, and is reachable from templates (`entity_*`), scripts
  (`world.*` on both the QuickJS and Node lanes) and HTTP. A JS handler that
  creates a user is answering the same question the schema's `users` query
  answers.

```yaml
world:
  schemas: [schemas/filestore.graphql]
  seed: 42
  counts: { User: 25 }

mocks:
  - id: filestore-graphql
    match:
      POST: https://api.example.com/graphql
    serve: graphql

  # An override is an ordinary mock winning on ordinary priority.
  - id: quota-exceeded
    match:
      POST: https://api.example.com/graphql
      graphql: { mutation: CreateFolder }
    response:
      json: { errors: [{ message: Storage quota exceeded }] }
```

Schema-derived routes sit at `config::serve::SERVED_PRIORITY` (50), below the
default 100, so a hand-written mock outranks the backend without anyone doing
arithmetic. A `serve:` mock whose config spells out the default priority is read
as not having chosen.

GraphQL mounts as **one** mock matching any operation. Matching by operation
name would be finer grained, but the name is chosen by the client, not the
schema — a schema-derived backend cannot know it in advance, and pretending
otherwise would leave real requests unmatched. Consequence: `verify()` on a
GraphQL mount asserts the endpoint's total, not per-operation; assert on the
override mock for that. A protocol that designs many endpoints (OpenAPI) expands
to one mock per operation instead, so coverage names the endpoints.

### JSON into a JS runtime

`serde_json/arbitrary_precision` is force-enabled workspace-wide by
`rolldown_common`. Under it `serde_json::Value::Number`'s `Serialize` emits a
private one-key map that only serde_json's own deserializer intercepts, so any
*other* serializer produces `{"$serde_json::private::Number": "3"}` where a
number was meant.

- **QuickJS**: never `rquickjs_serde::to_value` for a `serde_json::Value`. Use
  `scripting::bindings::convert::json_to_js`, which walks the value into native
  values — also faster, since it skips the serde data model entirely. It defines
  own properties via `JS_DefineProperty` rather than `Object::set`, so a
  `__proto__` key in an entity lands as a field instead of firing the prototype
  setter.
- **Reading JS into Rust** stays on `rquickjs_serde::from_value`: the token is
  only ever produced by `Serialize`, so the inbound direction never meets it.
- **NAPI** needs no workaround — `napi`'s `ToNapiValue for &Value` matches the
  enum directly and `Number` goes through `is_i64`/`as_i64`/`as_f64`.

### Absolute URLs in a match

A bare absolute `match.url` splits into a path pattern plus a `Host` matcher,
the same way `http.get("https://api.example.com/x")` does — a server sees `GET /x`
with `Host: api.example.com`, never the whole URL, so keeping it as one string would
never match anything behind a proxy. An `exact:`-prefixed URL is left whole:
that is what the HAR loader and the consolidator emit, and they mean the request
line verbatim.

## Benchmarking

`benches/world_performance.rs` (feature `spec`) covers the entity world: seeding,
`count`, `get`, paged and filtered lists, writes, and rebuilds. It is what caught
an unfiltered `limit: 25` costing 21ms on a 10,000-instance entity by
materialising everything before paginating; such a page is now answered from the
census, deriving only the window. Filtered or sorted lists still scan, which is
inherent.

Never measure ferrimock and another interceptor in the same process. Whichever
loads second is penalised — MSW measures 28.9us alone and 232.5us when it follows
ferrimock, an 8x swing decided by ordering alone. Cross-library numbers come from
`benchmarks/fair.mjs`, which runs each library in its own process and alternates
which goes first. `packages/core/test/comparison.test.ts` measures ferrimock only,
for the same reason.

Warm both arms identically, and never quote a server-mode figure (real TCP)
against an interceptor's (in-process). The README's original "3-4x faster than
MSW" came from breaking both rules at once.

## Code Standards

- Idiomatic Rust with zero-cost abstractions
- `anyhow::Result` for application code
- `unsafe` denied in ferrimock-napi (except marked `#[allow(unsafe_code)]` for NAPI FFI)
- FxHashMap for performance-critical paths (not std HashMap)
- All new code must include tests
- Run `cargo test -p ferrimock --lib` and `bun test` before committing

## Consolidation

Consolidation compresses a recording into patterns and templates. It is lossy,
so a reduction ratio on its own says nothing -- collapsing every mock into one
would score 99%. Every change to the consolidator has to be judged against
replay fidelity, not size.

### Fidelity

`consolidator::fidelity` replays each recorded request through the consolidated
collection and diffs the answer against what was really recorded, at levels that
fail independently: matched, no cross-talk, status exact, shape equal, constants
held, value equal. It scores the *unconsolidated* collection the same way, so a
failure is attributable -- a recording the recorder cannot replay is not the
consolidator's fault, and the delta between the two is what consolidation cost.

```bash
ferrimock mock consolidate in.json out.json --verify traffic.har --fail-under 0.95
```

`--verify` takes a recording session or a HAR -- the formats that keep requests
alongside responses. A consolidated mock collection cannot be verified against
itself: it no longer records what was asked.

### Domain knowledge

The engine ships defensible defaults and no API-specific rules. Anything that
depends on knowing a particular API -- that `/v2/` is a version rather than an
id, that `continuation` is a cursor, which hosts serve file content -- goes in a
`profile::ConsolidationProfile` supplied by the embedder. Do not add such rules
to the engine; add the hook the profile needs.

### Tests

- `tests/consolidator_fidelity.rs` -- scenarios someone thought of, each
  asserting both the reduction and the fidelity it must not cost.
- `tests/consolidator_props.rs` -- proptest over a generated synthetic API.
  Invariants are behavioural: grouping and templating are the engine's business,
  answering every recorded request correctly is not.
- `fuzz/` -- cargo-fuzz targets for crash safety and the invariants that hold
  over arbitrary input. Needs nightly:

```bash
cargo +nightly install cargo-fuzz
scripts/fuzz.sh          # every target, 60s each
scripts/fuzz.sh 0 consolidate   # one target, until stopped
```
