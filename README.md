# Ferrimock

High-performance HTTP mocking engine for Node.js, powered by Rust. Drop-in replacement for MSW, 1.1-1.7x faster on the interception path and far cheaper to run as a standalone server.

## Why Ferrimock?

- **Faster than MSW** -- 1.4-1.7x on node, 1.1-1.5x on bun; Rust matching engine + NAPI FunctionRef optimization ([method](#performance))
- **MSW drop-in API** -- `setupServer`, `http.get()`, `HttpResponse.json()`, `graphql.link()`, `server.use()`, lifecycle events
- **Declarative mocks** -- YAML/JSON/HAR files with Tera templates and 115+ fake data generators
- **Zero-config interceptor** -- Patches `fetch`, `XMLHttpRequest`, and `http.ClientRequest`, works with any test runner

## Performance

Cost of one mocked `fetch()`, `setupServer` against `setupServer` — the drop-in
path, in-process, no sockets on either side. Lower is better.

**node 22.22.2**, msw 2.13.2:

| Scenario | Ferrimock | MSW | Speedup |
|---|---:|---:|---:|
| Static JSON | 36.9us | 55.2us | **1.49x** |
| Path params | 38.6us | 53.6us | **1.39x** |
| Handler + fake data (`fake.*` vs faker.js) | 38.2us | 65.8us | **1.72x** |

**bun 1.4.0**, msw 2.13.2 — bun narrows the gap by making MSW's interception
cheaper, not by slowing ferrimock down:

| Scenario | Ferrimock | MSW | Speedup |
|---|---:|---:|---:|
| Static JSON | 24.8us | 29.0us | **1.17x** |
| Path params | 26.5us | 30.0us | **1.13x** |
| Handler + fake data | 25.9us | 38.0us | **1.47x** |

Server mode (a real axum server over real TCP) costs 82-104us per request on the
same machine. That is a different shape of work from an interceptor and is not
comparable with the tables above, or with MSW, which has no server mode.

Reproduce: `cd benchmarks && node fair.mjs` (add `BENCH_RUNTIME=node` for node).
Each library is measured in its own process, alternating which runs first —
loading two fetch interceptors into one process penalises whichever runs second
by up to 8x, which is enough on its own to manufacture any result you like.

## Quick Start

```bash
bun add ferrimock
```

### setupServer (MSW drop-in)

```ts
import { setupServer } from 'ferrimock/node'
import { http, HttpResponse, delay } from 'ferrimock'

const server = setupServer(
  http.get('/api/users/:id', async ({ params }) => {
    await delay(100)
    return HttpResponse.json({ id: params.id, name: 'John' })
  }),
)

server.listen({ onUnhandledRequest: 'error' })

// fetch is now intercepted
const res = await fetch('http://localhost/api/users/42')
const user = await res.json() // { id: '42', name: 'John' }

server.close()
```

Existing MSW test suites keep working: swap the `msw` / `msw/node` imports for
`ferrimock` / `ferrimock/node`. Resolvers receive `{ request, params, cookies,
requestId }` with a real Fetch `Request`; returning `undefined` falls through
to the next handler; `{ once: true }`, generator resolvers, `passthrough()`,
absolute-URL predicates, and `server.boundary()` all behave like MSW.
`setupWorker` (browser service worker mode) is not provided — the engine is a
native addon.

### Declarative Mocks (YAML)

```ts
const interceptor = new FerrimockInterceptor()

await interceptor.loadMocks('./mocks')
interceptor.apply()
```

```yaml
# mocks/users.yaml
mocks:
- id: get-user
  match:
    methods: ["GET"]
    url: "/api/users/:id"
  response:
    status: 200
    headers:
      content-type: "application/json"
    template: |
      {
        "id": "{{ captures.id }}",
        "name": "{{ fake_name() }}",
        "email": "{{ fake_email() }}"
      }
```

### Scripted Mocks (QuickJS, no Node required)

`.js`/`.mjs`/`.ts`/`.mts` files in the mocks directory define MSW-style handlers that
run on an embedded QuickJS engine — the CLI server and any Rust embedder execute them
without Node. Files are bundled by rolldown (TypeScript transpiled, `node_modules`
and relative imports resolved + tree-shaken), compiled once to QuickJS bytecode, and
cached on disk keyed by toolchain ABI + transitive input hashes — warm starts skip
bundling and compiling entirely. Error positions map back to the original sources.
Handlers support async/await, `delay()`, `fake.*`, and module-scope state (counters,
in-memory stores) that persists across requests and resets on hot reload.

```js
// mocks/users.mjs
let hits = 0;

http.get('/api/users/:id', ({ params }) => {
  hits += 1;
  return HttpResponse.json({ id: params.id, name: fake.name(), hits });
});

http.post('/api/login', async () => {
  await delay(100);
  return HttpResponse.json({ token: fake.jwt() }, { status: 201 });
});
```

```bash
ferrimock mock serve mocks/   # picks up .js/.mjs next to YAML/JSON/HAR, hot reloads all
```

Portable with Node: `import { http, HttpResponse, fake, delay } from 'ferrimock'` works
in both runtimes — the same file loads under the CLI (QuickJS) and under Node via
`loadMocksDir` (V8), whether it registers with bare calls or `export default [...]`.
RegExp paths (`http.get(/^\/api\/\d+$/i, ...)`), `HttpResponse.error()`, and
`passthrough()` behave the same in both. npm packages resolve and bundle in both
runtimes. Scripted handler calls cost ~10us (Rust matching + QuickJS execution);
matching never touches JS. Enabled via the `scripting` cargo feature (included in
`full`; excluded from the Node addon, where V8 runs the files instead).

### HTTP Server Mode

```ts
import { FerrimockServer, http, HttpResponse } from 'ferrimock'

const server = new FerrimockServer()

server.useHandlers([
  http.get('/api/users/:id', async ({ params }) =>
    HttpResponse.json({ id: params.id, name: 'John' })
  ),
])

const url = await server.listen(3000)
// Server running at http://127.0.0.1:3000

await server.close()
```

## MSW-Compatible API

### HTTP Handlers

```ts
import { http, HttpResponse } from 'ferrimock'

http.get('/path', resolver)      // GET
http.post('/path', resolver)     // POST
http.put('/path', resolver)      // PUT
http.delete('/path', resolver)   // DELETE
http.patch('/path', resolver)    // PATCH
http.head('/path', resolver)     // HEAD
http.options('/path', resolver)  // OPTIONS
http.all('/path', resolver)      // Any method

// One-time handlers
http.get('/path', resolver, { once: true })

// RegExp matching
http.get(/^\/api\/users\/\d+$/, resolver)

// Absolute URLs (host + path matching)
http.get('https://api.example.com/users/:id', resolver)
```

### GraphQL Handlers

```ts
import { graphql, HttpResponse } from 'ferrimock'

graphql.query('GetUser', ({ query, variables, operationName }) =>
  HttpResponse.json({ data: { id: variables.id } })
)
graphql.mutation('CreateUser', resolver)
graphql.mutation(/^Update/, resolver)   // RegExp operation names
graphql.operation(resolver)             // any operation

// Endpoint-scoped handlers
const github = graphql.link('https://api.github.com/graphql')
github.query('GetRepo', resolver)
```

### Responses

`HttpResponse` extends the native `Response` — handlers can also return any
plain `Response`.

```ts
HttpResponse.json({ key: 'value' })
HttpResponse.json({ key: 'value' }, { status: 201, headers: { 'x-custom': 'val' } })
HttpResponse.text('plain text')
HttpResponse.html('<h1>Hello</h1>')
HttpResponse.xml('<root/>')
HttpResponse.arrayBuffer(buffer)
HttpResponse.formData(formData)
HttpResponse.redirect('/target', 302)
HttpResponse.error()  // simulate network failure
new HttpResponse('body', { status: 418, statusText: "I'm a teapot" })
new HttpResponse(readableStream)  // streamed body; delivered live by the interceptor
new Response(null, { status: 204 })
```

Streamed bodies pass through the interceptor untouched — the caller reads
the handler's own `ReadableStream`, chunk timing included. The standalone
TCP server and the QuickJS runtime deliver the drained (buffered) body.

### Resolver Info

Resolvers receive MSW's info object:

```ts
http.post('/api/users/:id', async ({ request, params, cookies, requestId }) => {
  request.url                      // full URL (real Fetch Request)
  request.method                   // 'POST'
  request.headers.get('accept')    // case-insensitive Headers
  await request.json()             // parsed body
  await request.text()             // raw body
  await request.formData()         // multipart or urlencoded body
  params.id                        // ':id' capture
  cookies.session                  // parsed request cookies
  requestId                        // matches lifecycle-event requestId
})
```

Returning `undefined` falls through to the next matching handler. Generator
resolvers (`function*`) advance one yield per request, and the last value
repeats after the generator is done.

### Utilities

```ts
import { delay, passthrough, bypass } from 'ferrimock'

// Delay response
http.get('/api/slow', async () => {
  await delay(200)        // exact ms
  await delay('real')     // random 100-400ms
  await delay('infinite') // never resolves (test timeouts)
  return HttpResponse.json({ ok: true })
})

// Passthrough to real network
http.get('/api/real', () => passthrough())

// Bypass interception for a specific request
const realResponse = await fetch(bypass('http://real-api.com/data'))
```

### Verifying What Ran

Every match is counted with no setup — including declarative mocks, which have no
resolver to hold a spy.

```ts
server.matchCount('get-user')   // 3
server.matchCounts()            // [{ mockId: 'get-user', count: 3 }]
server.resetMatchCounts()
```

### Server Methods

```ts
const server = setupServer(...handlers)

server.listen({ onUnhandledRequest: 'warn' }) // 'bypass' | 'warn' | 'error' | callback
server.use(...handlers)          // Runtime overrides (higher priority)
server.resetHandlers()           // Drop runtime overrides, restore initial handlers
server.resetHandlers(...next)    // Replace the entire handler set
server.restoreHandlers()         // Re-enable consumed { once } handlers
server.listHandlers()            // List active handlers
server.boundary(callback)        // Scoped handler isolation
server.close()                   // Restore fetch/XHR/ClientRequest
```

### Lifecycle Events

```ts
server.events.on('request:start', ({ request, requestId }) => { ... })
server.events.on('request:match', ({ request, requestId }) => { ... })
server.events.on('request:unhandled', ({ request, requestId }) => { ... })
server.events.on('request:end', ({ request, requestId }) => { ... })
server.events.on('response:mocked', ({ request, requestId, response }) => { ... })
server.events.on('response:bypass', ({ request, requestId, response }) => { ... })
server.events.on('unhandledException', ({ request, requestId, error }) => { ... })
```

### Fake Data (115+ generators)

```ts
import { fake } from 'ferrimock'

fake.uuid()         // '550e8400-e29b-41d4-a716-446655440000'
fake.name()         // 'John Smith'
fake.email()        // 'john@example.com'
fake.phone()        // '+1-555-0123'
fake.city()         // 'San Francisco'
fake.url()          // 'https://example.com'
fake.ipv4()         // '192.168.1.1'
fake.creditCard()   // '4111111111111111'
fake.jwt()          // 'eyJhbGciOiJIUzI1NiJ9...'
fake.sentence()     // 'The quick brown fox...'
// ... 100+ more
```

Seed the generators to make a run reproducible — same values, same order, every
machine. Templates get one stream per mock id, so a mock's response is stable no
matter how requests to other mocks interleave.

```ts
fake.setSeed(42)
fake.resetSeedStreams()  // replay from the top, e.g. in beforeEach
fake.setSeed(null)       // back to entropy
```

```bash
ferrimock mock serve mocks/ --seed 42     # or FERRIMOCK_SEED=42
```

## Spec-Derived Backends

Point a mock at a GraphQL schema or an OpenAPI document and it serves the whole
API — a seeded, relational world with working reads, writes and relations —
instead of a pile of canned responses.

A schema declares *entities*, not routes. Where the API answers is a mock's job,
because a `.graphql` has nowhere to say it lives behind `https://api.example.com`
rather than on localhost.

```yaml
# mocks/filestore.yaml
world:
  schemas:
    - filestore.graphql
    - filestore-content.openapi.yaml     # merges into the SAME entity world
  seed: 42                 # same seed, same world, every run
  counts: { User: 25, Folder: 200 }
  cascade_delete: true     # a delete takes dependent records with it

mocks:
  - id: filestore-graphql
    match:
      POST: https://api.example.com/graphql
    serve: graphql

  - id: filestore-rest
    match:
      url: https://api.example.com/2.0   # base; operations supply path and method
    serve: rest

  # Overrides are ordinary mocks winning on ordinary priority.
  - id: quota-exceeded
    match:
      POST: https://api.example.com/graphql
      graphql: { mutation: CreateFolder }
    response:
      json: { errors: [{ message: Storage quota exceeded }] }

  - id: uploads-down
    match:
      POST: https://api.example.com/2.0/files/content
    response:
      status: 503
```

```graphql
# POST https://api.example.com/graphql — relations resolve, writes persist
query { folders { name owner { name email } } }
mutation { createFolder(name: "Reports") { id } }
```

```bash
# The same entities over REST — paging, filtering, sorting and writes
curl 'https://api.example.com/2.0/folders?limit=25&sort=-name'
curl 'https://api.example.com/2.0/folders/f_1/items'
curl -X POST -d '{"name":"Reports"}' https://api.example.com/2.0/folders
```

GraphQL mounts as one endpoint, because the client chooses the operation name. A
document *designs* its endpoints, so it mounts one mock per operation
(`filestore-rest#getFolder`) — coverage names them, `verify()` counts one of them, and
overriding one is an ordinary higher-priority mock at that path.

`ferrimock world explain` prints the entities, the rule and confidence behind
every inferred relation, and how many operations are answered from the world
rather than from their declared shape alone.

### One world, shared by every kind of mock

The entities are not private to the schema. A JS handler, a Tera template and a
schema-derived route all read and write the same store — so a user created in a
handler shows up in the next GraphQL query.

```ts
import { http, HttpResponse, world } from 'ferrimock'

http.post('https://api.example.com/2.0/users', async ({ request }) => {
  const { name } = await request.json()
  const user = world.create('User', { name })       // visible to the schema
  return HttpResponse.json(user, { status: 201 })
})
```

```yaml
response:
  template: '{"total": {{ entity_count(type="User") }}}'
```

Reachable over HTTP too, for a test driver that does not embed the engine:

```bash
curl localhost:3000/__mock/world                 # entities, seed, pending writes
curl localhost:3000/__mock/world/User?limit=10   # a page
curl -X POST localhost:3000/__mock/world/User -d '{"name":"Ada"}'
curl -X DELETE localhost:3000/__mock/world       # reset to the seeded world
```

```bash
ferrimock world explain --dir mocks/   # what is in the world, and from where
```

Details in [Mock Engine](docs/MOCK_ENGINE.md).

## Reverse Proxy

Put ferrimock in front of a dev server or a backend and point the browser at it
instead. A request that matches a mock is answered locally; everything else
reaches the real thing. One origin covers both, so there is no CORS to
configure and nothing in the application changes.

```bash
# In front of a vite dev server
ferrimock proxy http://localhost:5173

# API to a backend, everything else to vite. The longest prefix wins
ferrimock proxy -r /api=http://localhost:8080 -r /=http://localhost:5173

# With mocks, so anything they match never reaches the upstream
ferrimock proxy --mocks ./mocks http://localhost:5173

# Record what does reach the real backend, ready to consolidate into mocks
ferrimock proxy --record ./recordings https://api.example.com

# Terminate TLS with a generated certificate, for a secure browsing context
ferrimock proxy --tls http://localhost:5173
```

The forwarding path never collects a body. A request is read into memory only
when some registered mock matches on request bodies, and a response only when a
`patch:` mock is rewriting one; everything else moves frame by frame. So an
upload, a bundle and an event stream each cost one frame of memory rather than
their own size, and HMR WebSockets and SSE work without configuration.
Recording keeps that property by teeing the body as it streams rather than
collecting it first.

Downstream it is an axum router on an axum server, so HTTP/1.1, HTTP/2 and
WebSocket all work as they do anywhere else in axum. Upstream it speaks
HTTP/1.1 and HTTP/2 by ALPN, TLS with optional certificate validation, and
WebSocket.

From Rust, behind the `proxy` feature:

```rust
use ferrimock::proxy::{ProxyConfig, RouteConfig};

let mut config = ProxyConfig {
    routes: vec![
        RouteConfig::parse("/api=http://localhost:8080")?,
        RouteConfig::parse("/=http://localhost:5173")?,
    ],
    ..ProxyConfig::default()
};
config.compile();

let proxy = ferrimock::proxy::start(config, Some(matcher)).await?;
println!("listening on {}", proxy.url());
```

## Packages

| Package | Description |
|---------|-------------|
| `ferrimock` (npm) | The MSW drop-in surface (`ferrimock` + `ferrimock/node`), alias of `ferrimock` |
| `ferrimock` | setupServer, interceptor, HttpResponse, config loader |
| `@ferrimock/node` | Rust NAPI bindings (http, graphql, HttpResponse builders, fake, world, FerrimockServer) |
| `@ferrimock/playwright` | Playwright fixture adapter |

## Rust Library

Ferrimock is also a standalone Rust library for mock matching, template rendering, and HTTP server.

```toml
[dependencies]
ferrimock = { git = "https://github.com/salamaashoush/ferrimock", features = ["full"] }
```

See [Mock Engine](docs/MOCK_ENGINE.md), [Fake Data](docs/FAKE_DATA.md), [GraphQL](docs/GRAPHQL_MOCKS.md), [CLI Reference](docs/CLI_REFERENCE.md).

## CLI

```bash
npm install -g @ferrimock/cli              # Prebuilt binary via npm
cargo install ferrimock-cli --locked       # Or build from source

ferrimock mock serve mocks/                # Serve mocks with hot reload
ferrimock proxy http://localhost:5173      # Proxy a dev server, mocks first
ferrimock mock create "/api/users/:id"     # Create a mock
ferrimock mock test -m GET /api/users/123  # Test matching
ferrimock fake data email --count 10       # Generate fake data
ferrimock world explain                    # Entities a mocks dir builds
```

## License

MIT OR Apache-2.0
