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
  state is deterministic given the seed plus the sequence of writes. Two passes
  run over a record after its fields are drawn, and neither makes a field depend
  on another record: lifecycle timestamps are dealt back out in the order their
  names say they happened, and `*_count` fields are answered from the relation
  they name.
- `consolidator` - Smart mock consolidation with pattern detection
- `type_detector` - What a field holds. One `FieldType` vocabulary and one
  name-matching layer (`matches_field_name` and friends, which normalise
  `createdAt`/`created_at`/`CREATED-AT`), shared by every lane including
  `ferrimock-ml`. The *rules* are not shared, and deliberately:
  `detect_from_semantic_context` confirms a guess about a name against the
  values a recording actually carried, while `spec::infer::semantics` has no
  values and instead has a declared type and a `format`. Neither is a subset of
  the other, and where both answer they must agree — pinned by a test in
  `spec::infer::semantics`. The one declared exception is a bare `name`: the
  spec lane knows the owning entity, so `Folder.name` is a folder's name there
  and a person's in the recording lane. A third, name-only rule set used to sit
  beside these for the old GraphQL mock generator; both are gone.
- `graphql` - Reading a GraphQL schema: introspection over the wire, the
  response parsed into a `ParsedSchema`, and SDL written back out. Only reading —
  what a schema *means* is `spec::infer::graphql` and what it *serves* is
  `spec::bind::graphql`.
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

Two front ends compile into one world: `spec::infer::graphql` reads SDL and
`spec::infer::openapi` reads an OpenAPI 3.0/3.1 document. Both produce an
`EntityGraph`, so a `User` declared by a schema and a `User` described by a
document are one `User` with one set of instances. The module split is the same
on both sides: *read a spec* (`infer`), *bind it to a protocol* (`bind`), *mount
it as ordinary mocks* (`emit`). `spec::bind::plan::RootPlan` is shared — the six
rungs (get/list/create/update/delete/unclassified) are what both classifiers
land on, and what coverage counts.

The split that keeps this one system rather than two:

- **A schema declares entities, never routes.** It has nowhere to write down
  that it is served at `https://api.example.com/graphql` rather than on localhost,
  and guessing is how a proxy answers on the wrong host. Loading a `.graphql`
  populates the world and registers *zero* mocks; the loader warns when a world
  has entities but nothing serves them. An OpenAPI document *does* carry paths,
  but still not a host — `servers:` is reported, never mounted from.
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
  schemas:
    - schemas/filestore.graphql
    - schemas/filestore-content.openapi.yaml   # merges into the SAME entity graph
  seed: 42
  counts: { User: 25 }

mocks:
  - id: filestore-graphql
    match:
      POST: https://api.example.com/graphql
    serve: graphql

  - id: filestore-rest
    match:
      url: https://api.example.com/2.0       # base; operations supply path + method
    serve: rest

  # An override is an ordinary mock winning on ordinary priority.
  - id: quota-exceeded
    match:
      POST: https://api.example.com/graphql
      graphql: { mutation: CreateFolder }
    response:
      json: { errors: [{ message: Storage quota exceeded }] }
```

Which file is read as what is decided by extension, never by contents — a file
that has to be opened before anyone can say what it is fails differently every
time its contents change. A bare `.yaml`/`.json` is a mock collection; an
OpenAPI document auto-loaded from a mocks directory is named `*.openapi.yaml`
(or `.yml`/`.json`); anything named under `world.schemas` loads whatever it is
called, as GraphQL for `.graphql`/`.gql` and as OpenAPI otherwise.

Schema-derived routes sit at `config::serve::SERVED_PRIORITY` (50), below the
default 100, so a hand-written mock outranks the backend without anyone doing
arithmetic. A `serve:` mock whose config spells out the default priority is read
as not having chosen.

GraphQL mounts as **one** mock matching any operation. Matching by operation
name would be finer grained, but the name is chosen by the client, not the
schema — a schema-derived backend cannot know it in advance, and pretending
otherwise would leave real requests unmatched. Consequence: `verify()` on a
GraphQL mount asserts the endpoint's total, not per-operation; assert on the
override mock for that.

OpenAPI is the opposite and expands to **one mock per operation**, id
`{mount-id}#{operationId}` (falling back to `{method}-{path}` when the document
names none). The mount supplies the base path and Host; the document supplies
method and path, which go through the ordinary `config::parse_url_pattern`, so
`{param}` becomes a named capture like any hand-written route. A `match.method`
on a `serve: rest` mock is a validation error — operations carry their own.
Accepted cost: a 500-operation document becomes 500 `MockDefinition`s. What it
buys is what a single glob mock cannot give — coverage that names the endpoints,
`verify("filestore-rest#getFolder", Exactly(1))`, and an override that is an ordinary
higher-priority mock at that path rather than a special case.

### Inferring an entity graph from an OpenAPI document

A GraphQL schema states which types have identity; a document does not, so
identity is read off the shape of the surface. Every fact carries the `Rule` that
produced it and a `Confidence`, and `ferrimock world explain` prints both —
inference that cannot explain itself is not usable on a real document.

`CollectionItemPair` (a collection path beside an item path) decides which
schemas are entities and which path parameter addresses one; `SchemaRef`,
`PathNesting`, `SpecLink`, `ForeignKeyName` and `VendorExtension` decide the
relations between them. The `Carrier` says how a link rides on the wire, and the
choice is load-bearing: a `ForeignKey(field)` relation *is* the scalar field, not
a sibling of it, because the store already writes a to-one link's value as the
target's key — so `folder.user_id` holds a key that resolves rather than a
plausible-looking UUID that does not.

The carrier may name a *different* field than the one holding the link, which is
what a document declaring both `user_id` and a `$ref`'d `customer` compiles to:
one link, written twice. Left as two relations they derive independently and the
key names a different user than the object beside it. `Carrier::key_field` is
the one place that answers "which field holds this link's key".

A path addressing one instance by several parameters that each name a field of
the schema — `/repos/{owner}/{repo}` — produces a `CompositeKey` over all of
them. A key of one part keeps the derivation it has always had; the parts of a
composite key are each derived from their own field, or every part of the key
reads as the same value.

Name matching matches names, never meanings: `owner_id` finds an entity called
`Owner`, and nothing teaches the engine that an owner is a `User`. Domain
knowledge belongs in a `ConsolidationProfile` (`spec_relation` for `x-`
extensions, `pagination_dialect` for what this API calls a limit), never in the
engine.

The document is read off a `serde_json::Value` rather than deserialized into
typed structs. Typed OpenAPI crates model 3.0 and 3.1 as separate type systems
and shape their `Either` fields as untagged enums — and untagged buffering under
`arbitrary_precision` turns a number into a private one-key map (see below).
Walking a `Value` meets neither problem, and the 3.0/3.1 divergences (`nullable`,
`exclusiveMinimum`, `type: [x, "null"]`) are few enough to name in one reader.

Resolution is explicit: `serve: graphql` binds the single GraphQL schema in the
world, and refuses with both paths named when there is more than one. Say which
with `serve: { protocol: graphql, schema: <path> }`. Two mounts of the same
schema serve identical data — that is what sharing the world means, and
"same schema, two independent datasets" is deliberately not offered.

Adding a schema **rebuilds** the store and replays every write onto it, so
loading a second schema does not discard state a handler already wrote. Entity
names and ordinals that already existed keep their exact values, because the
base layer derives from the seed. A patch whose record no longer exists (an
entity's count shrank) is reported as a `DeltaConflict` rather than dropped.
A rebuilt census steps over any key a created record already owns: growing a
count would otherwise re-derive a key that is live, and serve two records under
it. Skipping decouples an instance's ordinal from its position, which is why
everything pairing two instances compares *keys* rather than ordinals.

Each source's declaration is kept apart and the merged graph is recomposed from
all of them, so a reload replaces that source's contribution — a field removed
from a schema is removed from the world — while two schemas describing one
entity union their fields rather than one silently replacing the other.
Rebuilds are serialized; a write landing between the snapshot and the swap is
lost, which is a startup and hot-reload window, not a request-path one.

Who owns whom is a `Partition`: each parent draws a weight, the child positions
are cut in proportion, and both directions read the same boundaries. Hashing each
child independently spread them evenly, which no real dataset is. Three things
fall out of the range being the answer — the distribution is lopsided, reading
one parent's children costs that parent's children rather than a filter over
every child, and a `*_count` field is arithmetic rather than a scan. Partitions
depend only on the seed and the two census sizes, all fixed for a store's life,
so they are built on first use and never invalidated.

An entity that owns *itself* cannot use that partition. Cutting a census
against itself has a fixed point for every seed and every count — the owning
map is monotone over a rising boundary vector, so `owner_of(i) - i` has to
cross zero — and a third of a twelve-record hierarchy came out as its own
parent. Self-relations are levelled instead: positions are cut into contiguous
levels, each level is partitioned across the one above it, and level zero has
nothing above it, which is where the world's roots come from. A parent is
always at a lower level than its child, so a cycle of any length is impossible
rather than merely unlikely, and the range property survives — one parent still
owns one contiguous run, so reading its children is a range read and counting
them is still arithmetic. A `parent` the spec marked non-nullable is therefore
unsatisfiable, and `world explain` says so.

Because a hierarchy has generations, a delete cascades to a *fixpoint* rather
than one level. Stopping at the first generation leaves everything below it
pointing at a tombstone, which is the dangling key this store exists to make
impossible.

Ownership is contiguous in *partition position*; where an instance sits in the
*census* is a seeded shuffle of that. Without the separation the partition was
visible in a single response: the number of runs of the parent key down an
unsorted page equalled the number of distinct parents on it, exactly, with no
variance at any size -- an identity rather than a statistic. Levels would have
made it louder still, since every root would have come first. The shuffle is a
Fisher-Yates table and its inverse, two `Vec<u32>` beside a census that already
holds a `Vec<EntityKey>` and a slot map, so it costs one array index per child
read. Nothing outside `Ownership` is handed a range: the two spaces meeting in
a caller is exactly the bug -- a `*_count` drifting from the list endpoint by
one per write -- that keeping them apart prevents.

`Slot` is why the partition works after a census had to step over a reserved
key: `ordinal` is what a record's values derive from, `index` is where it sits
among its siblings, and everything pairing two instances works in `index`.

`core::world::store::distribution` is where a value's *shape* lives, separate
from what draws it. Every draw is a pure map over the bytes the field already
derived, so nothing about laziness, replay or determinism changes -- only what
comes out. The defaults are the point: uniform everywhere is the loudest
statistical signature an engine can have, and it is not one a client has to
work to see. A number nothing bounded is log-uniform over orders of magnitude
with a little mass on zero; a number with a *narrow* declared range stays
uniform, because a rating or a percentage is not Benford-ish and a log-normal
truncated below a decade is uniform again anyway. An enum is Zipf over a
permutation keyed on the field, which gives a skewed marginal without claiming
which member is modal -- declaration order does not say: lifecycle enums list
the terminal state last, machine-emitted schemas are often alphabetical, and
protobuf mandates `UNSPECIFIED` first. A boolean gets a chance drawn per field
and pushed away from the middle. A collection length is geometric rather than
always two. Which member is *actually* modal, and what the real rates are, is
something only a recording can say.

`required` and `nullable` are separate facts on a `FieldDef`, because a schema
gives two separate answers: `required` says the key is in the payload,
`nullable` says the value may be null. A GraphQL field that was selected is
always present and may be null; an OpenAPI property left out of `required` may
not be there at all, and answering it with `null` because it happened to be
optional violates the `type: string` that declared it. So an optional field
loses its key and a nullable one keeps it holding null -- each at a rate drawn
per field rather than per record, the way a real column is null a twentieth of
the time or half of it. A filter over an absent field matches nothing but `Ne`,
which is what a real API does and what a test asserting on it has to expect.

How many instances an entity gets is read off its place in the graph rather
than from one constant. The child end of a to-one link is more numerous than
the parent end -- a file store has more files than folders and more folders
than users -- so the count fans out with depth and stops at a cap, because the
census is eager and a five-deep document would otherwise ask for ten thousand
leaves. `world.count` still sets a flat default for everything, `world.counts`
still names one entity, and `world.scale` multiplies whatever the default
resolved to, which is how a mount asks for a bigger world without naming every
entity in it. Size is not a cosmetic setting: an entity smaller than one page
hands a client the whole population in a single request, and a five-member enum
needs about forty draws before anything can tell its distribution from uniform,
so a world too small to sample is a world whose statistics cannot be tested.

`ferrimock world doctor` lints the generated world for the things that give a
mock away, and it is the number any change to the world is judged against. It
runs with no corpus, because the case a mock exists for is the one where no
corpus of real responses exists; each check fails independently and reports the
measurement that failed it, so a change either moves a check or it does not. Two
outcomes are not a pass: a **defect** is a behaviour no real API has, and a
check the world is **too small to measure** — a five-member enum needs about
forty draws, which a twelve-record entity cannot supply — is reported as its own
outcome rather than silently as a clean bill.

`World::reset()` drops every write and leaves exactly what the seed derives —
call it between tests, or state leaks from one into the next.
`World::pending_writes()` is how you see that it did.

`MockRegistry::with_world()` gives a registry its own world for isolation, which
integration tests need because the process-global one is shared. `entity_*`
template functions read the *global* world (Tera's function registry is
stateless, so there is nowhere to thread a handle through — the same constraint
`PersistenceStore` already lives with), so `with_world` publishes its world there
when nothing has claimed it yet. The first registry in a process therefore gets
templates that read exactly what its routes serve; a second cannot displace it,
and keeps its own world for matching while templates go on reading the first.

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
`count`, `get`, paged and filtered lists, writes, and rebuilds, plus mounting and
answering an OpenAPI document (`rest/*`). It is what caught an unfiltered
`limit: 25` costing 21ms on a 10,000-instance entity by materialising everything
before paginating; such a page is now answered from the census, deriving only the
window. Filtered or sorted lists still scan, which is inherent.

That scan is on the REST request path, because a query parameter naming a field
becomes a predicate: `rest/answer/list_filtered_25` costs ~23ms on a
10,000-instance entity against ~1.5us for a lookup and ~730us for an unfiltered
page. Reads by key and unfiltered pages are flat in the world's size; a filtered
list is linear in it. Worth knowing before pointing a load test at one.

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
