# Mockpit

High-performance HTTP mocking engine for Node.js, powered by Rust. Drop-in replacement for MSW with 3-4x better performance.

## Why Mockpit?

- **3-4x faster than MSW** -- Rust mock matching engine + NAPI FunctionRef optimization
- **MSW drop-in API** -- `setupServer`, `http.get()`, `HttpResponse.json()`, `graphql.link()`, `server.use()`, lifecycle events
- **Declarative mocks** -- YAML/JSON/HAR files with Tera templates and 115+ fake data generators
- **Zero-config interceptor** -- Patches `fetch`, `XMLHttpRequest`, and `http.ClientRequest`, works with any test runner

## Performance

| Mode | Mockpit | MSW | Speedup |
|------|---------|-----|---------|
| Declarative (inline) | 9us | N/A | Rust-only, no JS crossing |
| Template + fake data | 8us | N/A | Rust Tera engine + fake generators |
| JS handler (static) | 15us | 37us | **2.5x** |
| JS handler + fake data | 18us | 46us | **2.6x** |
| Full interceptor flow | 13us | 35us | **2.7x** |

## Quick Start

```bash
bun add @mockpit/core
```

### setupServer (MSW drop-in)

```ts
import { setupServer } from 'mockpit/node'
import { http, HttpResponse, delay } from 'mockpit'

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
`mockpit` / `mockpit/node`. Resolvers receive `{ request, params, cookies,
requestId }` with a real Fetch `Request`; returning `undefined` falls through
to the next handler; `{ once: true }`, generator resolvers, `passthrough()`,
absolute-URL predicates, and `server.boundary()` all behave like MSW.
`setupWorker` (browser service worker mode) is not provided — the engine is a
native addon.

### Declarative Mocks (YAML)

```ts
const interceptor = new MockpitInterceptor()

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
mockpit mock serve mocks/   # picks up .js/.mjs next to YAML/JSON/HAR, hot reloads all
```

Portable with Node: `import { http, HttpResponse, fake, delay } from 'mockpit'` works
in both runtimes — the same file loads under the CLI (QuickJS) and under Node via
`loadMocksDir` (V8), whether it registers with bare calls or `export default [...]`.
RegExp paths (`http.get(/^\/api\/\d+$/i, ...)`), `HttpResponse.error()`, and
`passthrough()` behave the same in both. npm packages resolve and bundle in both
runtimes. Scripted handler calls cost ~10us (Rust matching + QuickJS execution);
matching never touches JS. Enabled via the `scripting` cargo feature (included in
`full`; excluded from the Node addon, where V8 runs the files instead).

### HTTP Server Mode

```ts
import { MockpitServer, http, HttpResponse } from '@mockpit/core'

const server = new MockpitServer()

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
import { http, HttpResponse } from 'mockpit'

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
import { graphql, HttpResponse } from 'mockpit'

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
import { delay, passthrough, bypass } from 'mockpit'

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
import { fake } from '@mockpit/core'

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

## Packages

| Package | Description |
|---------|-------------|
| `mockpit` (npm) | The MSW drop-in surface (`mockpit` + `mockpit/node`), alias of `@mockpit/core` |
| `@mockpit/core` | setupServer, interceptor, HttpResponse, config loader |
| `@mockpit/node` | Rust NAPI bindings (http, graphql, HttpResponse builders, fake, MockpitServer) |
| `@mockpit/playwright` | Playwright fixture adapter |

## Rust Library

Mockpit is also a standalone Rust library for mock matching, template rendering, and HTTP server.

```toml
[dependencies]
mockpit = { git = "https://github.com/salamaashoush/mockpit", features = ["full"] }
```

See [Mock Engine](docs/MOCK_ENGINE.md), [Fake Data](docs/FAKE_DATA.md), [GraphQL](docs/GRAPHQL_MOCKS.md), [CLI Reference](docs/CLI_REFERENCE.md).

## CLI

```bash
mockpit mock serve mocks/                # Serve mocks with hot reload
mockpit mock create "/api/users/:id"     # Create a mock
mockpit mock test -m GET /api/users/123  # Test matching
mockpit fake data email --count 10       # Generate fake data
```

## License

MIT OR Apache-2.0
