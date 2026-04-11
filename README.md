# Mockpit

High-performance HTTP mocking engine for Node.js, powered by Rust. Drop-in replacement for MSW with 3-4x better performance.

## Why Mockpit?

- **3-4x faster than MSW** -- Rust mock matching engine + NAPI FunctionRef optimization
- **MSW-compatible API** -- `http.get()`, `MockResponse.json()`, `server.use()`, lifecycle events
- **Declarative mocks** -- YAML/JSON/HAR files with Tera templates and 115+ fake data generators
- **Zero-config interceptor** -- Patches `fetch` and `XMLHttpRequest`, works with any test runner

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

### Interceptor (fetch patching, like MSW)

```ts
import { MockpitInterceptor, http, MockResponse, delay } from '@mockpit/core'

const interceptor = new MockpitInterceptor()

interceptor.useHandlers([
  http.get('/api/users/:id', async ({ params }) => {
    await delay(100)
    return MockResponse.json({ id: params.id, name: 'John' })
  }),
])

interceptor.apply()

// fetch is now intercepted
const res = await fetch('http://localhost/api/users/42')
const user = await res.json() // { id: '42', name: 'John' }

interceptor.dispose()
```

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

### HTTP Server Mode

```ts
import { MockpitServer, http, MockResponse } from '@mockpit/core'

const server = new MockpitServer()

server.useHandlers([
  http.get('/api/users/:id', async ({ params }) =>
    MockResponse.json({ id: params.id, name: 'John' })
  ),
])

const url = await server.listen(3000)
// Server running at http://127.0.0.1:3000

await server.close()
```

## MSW-Compatible API

### HTTP Handlers

```ts
import { http, MockResponse, fake } from '@mockpit/core'

http.get('/path', handler)      // GET
http.post('/path', handler)     // POST
http.put('/path', handler)      // PUT
http.delete('/path', handler)   // DELETE
http.patch('/path', handler)    // PATCH
http.head('/path', handler)     // HEAD
http.options('/path', handler)  // OPTIONS
http.all('/path', handler)      // Any method

// RegExp matching
http.get(/^\/api\/users\/\d+$/, handler)
```

### GraphQL Handlers

```ts
import { graphql } from '@mockpit/core'

graphql.query('GetUser', handler)
graphql.mutation('CreateUser', handler)
graphql.operation(handler)  // any operation
```

### Response Builders

```ts
MockResponse.json({ key: 'value' })
MockResponse.json({ key: 'value' }, { status: 201, headers: { 'x-custom': 'val' } })
MockResponse.text('plain text')
MockResponse.html('<h1>Hello</h1>')
MockResponse.xml('<root/>')
MockResponse.arrayBuffer(buffer)
MockResponse.empty(204)
MockResponse.error()  // simulate network failure
```

### Request Context

```ts
http.get('/api/users/:id', async (req) => {
  req.method        // 'GET'
  req.path          // '/api/users/42'
  req.uri           // '/api/users/42?page=1'
  req.params        // { id: '42' }
  req.query         // { page: '1' }
  req.headers       // { 'content-type': 'application/json' }
  req.cookies       // { session: 'abc123' }
  req.body          // raw body string
  req.bodyJson      // parsed JSON body
  req.requestId     // unique request ID

  // Fast single-value lookups
  req.param('id')       // '42'
  req.header('accept')  // 'application/json'
  req.queryParam('page') // '1'
})
```

### Utilities

```ts
import { delay, passthrough, bypass } from '@mockpit/core'

// Delay response
http.get('/api/slow', async () => {
  await delay(200)        // exact ms
  await delay('real')     // random 100-400ms
  await delay('infinite') // never resolves (test timeouts)
  return MockResponse.json({ ok: true })
})

// Passthrough to real network
http.get('/api/real', async () => passthrough())

// Bypass interception for a specific request
const realResponse = await fetch(bypass('http://real-api.com/data'))
```

### Server Methods

```ts
const interceptor = new MockpitInterceptor()

interceptor.useHandlers([...])    // Register initial handlers
interceptor.use(handler)          // Add runtime handler (higher priority)
interceptor.resetHandlers()       // Remove all handlers
interceptor.restoreHandlers()     // Re-enable consumed once handlers
interceptor.listHandlers()        // List active handlers
interceptor.boundary(callback)    // Scoped handler isolation

interceptor.apply({
  onUnhandledRequest: 'warn'      // 'bypass' | 'warn' | 'error' | callback
})
```

### Lifecycle Events

```ts
interceptor.events.on('request:start', ({ request, requestId }) => { ... })
interceptor.events.on('request:match', ({ request, requestId }) => { ... })
interceptor.events.on('request:unhandled', ({ request, requestId }) => { ... })
interceptor.events.on('request:end', ({ request, requestId }) => { ... })
interceptor.events.on('response:mocked', ({ request, requestId, response }) => { ... })
interceptor.events.on('response:bypass', ({ request, requestId, response }) => { ... })
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
| `@mockpit/core` | Interceptor, MSW-compat API, config loader |
| `@mockpit/node` | Rust NAPI bindings (http, graphql, MockResponse, fake, MockpitServer) |
| `@mockpit/cli` | CLI for mock management, fake data generation |
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
