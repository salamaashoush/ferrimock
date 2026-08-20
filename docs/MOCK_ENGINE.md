---
sidebar_label: Mock Engine
section: Core Features
order: 2
---

# Mock Engine Guide

HTTP mocking engine for testing, development, and CI/CD workflows.

## Quick Start

```bash
# Interactive wizard (recommended for new users)
ferrimock mock create --interactive

# Quick mode with flags
ferrimock mock create "/api/users/:id" -m GET -s 200 --template

# Start standalone mock server with hot reload
ferrimock mock serve --watch     # Hot reload on port 3006

# Or integrate with your proxy (see Proxy Integration below)
```

Manual mock (`mocks/collections/my-api.yaml`):

```yaml
mocks:
- id: get-user
  match:
    methods: ["GET"]
    url: "/api/users/123"
  response:
    status: 200
    body: '{"id": "123", "name": "John Doe"}'
```

## Matching Rules

- All conditions must match (AND logic)
- Higher priority wins (default: 100)
- First match returns

```yaml
mocks:
- id: specific
  priority: 200 # Matches first
  match:
    url: "/api/users/admin"

- id: generic
  priority: 100 # Matches second
  match:
    url: "^/api/users/\\d+$"
```

### One-shot mocks

`once: true` retires a mock as soon as it matches, so the next request for the same thing falls through to the mock
behind it. Chaining them replays a sequence: the endpoint answers differently each time, and the last mock -- the one
without `once` -- answers from then on.

```yaml
mocks:
- id: job-pending
  once: true
  match: { url: "/api/jobs/7" }
  response: { status: 200, body: '{"state":"pending"}' }

- id: job-done # answers the second request and every one after it
  match: { url: "/api/jobs/7" }
  response: { status: 200, body: '{"state":"done"}' }
```

This is what `mock convert` builds when a recording answered the same request more than one way.

## URL Patterns

Auto-detected by pattern:

| Type    | Example              | Detection                       |
| ------- | -------------------- | ------------------------------- |
| Express | `/api/users/:id`     | Contains `:param` (recommended) |
| Glob    | `/api/**/*.json`     | Contains `**` or `*`            |
| Regex   | `^/api/v\\d+/users$` | Contains `^`, `$`, `\d`, etc.   |
| Exact   | `/api/users`         | Default (simple paths)          |

Express-style is cleanest and auto-generates named captures. Repeatable
params follow path-to-regexp modifiers: `/files/:path+` matches one or
more segments and `/files/:path*` zero or more (the zero-segment case
omits the param). Templates see the joined value (`{{ captures.path }}`
= `a/b/c`); MSW-style handlers receive repeatable params as `string[]`
(the matched segments, percent-decoded), matching MSW's `params` shape.

## Request Matching

### Methods

```yaml
match:
  method: GET # Single
  methods: ["GET", "POST", "PUT"] # Multiple
```

### Headers

```yaml
match:
  headers:
    content-type: "application/json" # Exact
    authorization: "~^Bearer .+" # Regex (~ prefix)
    x-api-key: "?" # Must be present
    x-debug: "!" # Must be absent
```

### Query Parameters

```yaml
match:
  query:
    page: "1"
    id: "\\d+" # Regex supported
```

### Body Matching

```yaml
match:
  body:
    "$.role": "admin" # JSONPath (auto-detected by $.)
    "~email.*@.*\\.com": true # Regex (~ prefix)
    "@username": true # Contains (@ prefix)
```

### GraphQL Matching

```yaml
# Simple string syntax
match:
  graphql: "GetUser" # Operation name
  graphql: "query" # Any query
  graphql: "mutation" # Any mutation
  graphql: true # Introspection queries

# Structured syntax with variables
match:
  graphql:
    mutation: "CreateUser"
    variables:
      "input.role": "admin"
      "input.email": "admin@example.com"

# Introspection specific
match:
  graphql:
    introspection: "schema" # __schema queries only
    introspection: "type" # __type queries only

# GraphQL helpers in templates
response:
  template: '{{ graphql_error(message="Not found", code="NOT_FOUND") }}'
```

## Response Generation

```yaml
mocks:
- id: example
  match:
    url: "/api/data"
  response:
    status: 200
    headers:
      content-type: "application/json"
    body: '{"message": "Static response"}'

    # File-based (static)
    file: "mocks/data/response.json"
    # Template from file (processed by Tera)
    template_file: "mocks/templates/user.tera"

    # Inline template with fake data
    template: '{"id": "{{ captures.id }}", "name": "{{ fake_name() }}"}'
```

## Template Engine

Built on [Tera](https://keats.github.io/tera/) with 115+ custom functions.

### Context Variables

```yaml
# Available in templates:
# {{ method }}              - HTTP method
# {{ path }}                - Request path
# {{ query.limit }}         - Query parameter
# {{ headers.authorization }} - Header value
# {{ body_json.field }}     - Parsed JSON body
# {{ captures.user_id }}    - URL captures
```

### Template Functions Reference

#### Core Functions

| Function                 | Parameters           | Returns           | Example                              |
| ------------------------ | -------------------- | ----------------- | ------------------------------------ |
| `uuid()`                 | None                 | UUID v4 string    | `{{ uuid() }}`                       |
| `now()`                  | None                 | Current timestamp | `{{ now() }}`                        |
| `get_random(start, end)` | start: int, end: int | Random integer    | `{{ get_random(start=1, end=100) }}` |

#### Identity & Personal (7 functions)

| Function            | Returns               | Example              |
| ------------------- | --------------------- | -------------------- |
| `fake_name()`       | Full name             | "John Smith"         |
| `fake_first_name()` | First name            | "John"               |
| `fake_last_name()`  | Last name             | "Smith"              |
| `fake_username()`   | Username              | "user_name123"       |
| `fake_password()`   | Password (8-16 chars) | "xK9#mP2@"           |
| `fake_title()`      | Title                 | "Mr.", "Mrs.", "Dr." |
| `fake_suffix()`     | Suffix                | "Jr.", "Sr.", "III"  |

#### Contact Information (4 functions)

| Function            | Returns    | Example            |
| ------------------- | ---------- | ------------------ |
| `fake_email()`      | Email      | "user@example.com" |
| `fake_free_email()` | Free email | "user@gmail.com"   |
| `fake_phone()`      | Phone      | "(555) 123-4567"   |
| `fake_cell_phone()` | Cell phone | "+1-555-123-4567"  |

#### Location & Address (13 functions)

| Function                   | Returns      | Example              |
| -------------------------- | ------------ | -------------------- |
| `fake_street()`            | Street name  | "Main Street"        |
| `fake_street_address()`    | Full address | "123 Main St"        |
| `fake_city()`              | City         | "New York"           |
| `fake_state()`             | State        | "California"         |
| `fake_state_abbr()`        | State abbrev | "CA"                 |
| `fake_zip()`               | ZIP code     | "90210"              |
| `fake_postal_code()`       | Postal code  | "90210"              |
| `fake_country()`           | Country      | "United States"      |
| `fake_country_code()`      | Country code | "US"                 |
| `fake_latitude()`          | Latitude     | "-45.6789"           |
| `fake_longitude()`         | Longitude    | "123.4567"           |
| `fake_building_number()`   | Building #   | "42"                 |
| `fake_secondary_address()` | Secondary    | "Apt 4", "Suite 200" |

#### Company & Job (7 functions)

| Function                | Returns        | Example                    |
| ----------------------- | -------------- | -------------------------- |
| `fake_company()`        | Company name   | "Acme Corp"                |
| `fake_company_suffix()` | Company suffix | "Inc.", "LLC"              |
| `fake_job_title()`      | Job title      | "Software Engineer"        |
| `fake_industry()`       | Industry       | "Technology"               |
| `fake_job_field()`      | Job field      | "Engineering"              |
| `fake_job_position()`   | Position       | "Manager", "Director"      |
| `fake_job_seniority()`  | Seniority      | "Junior", "Senior", "Lead" |

#### Internet & Networking (17 functions)

| Function                       | Returns        | Example                     |
| ------------------------------ | -------------- | --------------------------- |
| `fake_url()`                   | URL            | "https://example.com"       |
| `fake_domain()`                | Domain         | "example.com"               |
| `fake_ipv4()`                  | IPv4 address   | "192.168.1.1"               |
| `fake_ipv6()`                  | IPv6 address   | "2001:db8::1"               |
| `fake_mac_address()`           | MAC address    | "00:1A:2B:3C:4D:5E"         |
| `fake_user_agent()`            | User agent     | Browser UA string           |
| `fake_user_agent_modern()`     | Modern UA      | Chrome/Firefox UA           |
| `fake_color()`                 | Hex color      | "#A1B2C3"                   |
| `fake_pagination_url()`        | Pagination URL | "/items?page=5&limit=20"    |
| `fake_pagination_url_offset()` | Offset URL     | "/items?offset=50&limit=20" |
| `fake_search_url()`            | Search URL     | "/search?q=status=active"   |
| `fake_file_download_url()`     | Download URL   | Full URL with token         |
| `fake_api_url()`               | API URL        | "/v1/users/123"             |
| `fake_webhook_url()`           | Webhook URL    | "/callbacks/payment"        |
| `fake_api_endpoint()`          | API endpoint   | "/api/v1/users/uuid"        |
| `fake_resource_path()`         | Resource path  | "/users/123"                |

#### Text & Content (6 functions)

| Function                         | Parameters                          | Returns        |
| -------------------------------- | ----------------------------------- | -------------- |
| `fake_word()`                    | None                                | Single word    |
| `fake_words(count)`              | count: number                       | Multiple words |
| `fake_sentence(word_count)`      | word_count: number (default: 5)     | Sentence       |
| `fake_paragraph(sentence_count)` | sentence_count: number (default: 3) | Paragraph      |
| `fake_slug()`                    | None                                | URL slug       |
| `fake_alphanumeric(length)`      | length: number (default: 10)        | Random string  |

#### Finance & Commerce (7 functions)

| Function                 | Parameters      | Returns            |
| ------------------------ | --------------- | ------------------ |
| `fake_credit_card()`     | None            | Credit card number |
| `fake_currency_code()`   | None            | "USD"              |
| `fake_currency_name()`   | None            | "US Dollar"        |
| `fake_currency_symbol()` | None            | "$"                |
| `fake_price(min, max)`   | min, max: float | Price value        |
| `fake_amount()`          | None            | Amount string      |

#### Identifiers & Codes (11 functions)

| Function            | Returns       | Example                                |
| ------------------- | ------------- | -------------------------------------- |
| `fake_uuid()`       | UUID string   | "550e8400-e29b-41d4-a716-446655440000" |
| `fake_isbn()`       | ISBN-10       | "0-123456-78-9"                        |
| `fake_isbn13()`     | ISBN-13       | "978-0-123456-78-9"                    |
| `fake_token()`      | 32-char token | Alphanumeric token                     |
| `fake_etag()`       | HTTP ETag     | ETag value                             |
| `fake_numeric_id()` | Database ID   | "12345678"                             |
| `fake_short_hash()` | Git-like hash | "a1b2c3d"                              |
| `fake_sha256()`     | SHA256 hash   | 64-char hex                            |
| `fake_md5()`        | MD5 hash      | 32-char hex                            |
| `fake_base64()`     | Base64 string | Encoded string                         |
| `fake_jwt()`        | JWT token     | Three-part JWT                         |

#### Dates & Times (5 functions)

| Function                | Returns        | Example                |
| ----------------------- | -------------- | ---------------------- |
| `fake_date()`           | RFC3339 date   | "2023-12-25T10:30:00Z" |
| `fake_time()`           | Time           | "14:30:45"             |
| `fake_iso_date()`       | ISO date       | "2023-12-25"           |
| `fake_unix_timestamp()` | Unix timestamp | 1703505000             |
| `fake_relative_time()`  | Relative time  | "2 hours ago"          |

#### Web-Specific Data (19 functions)

| Function                   | Parameters       | Returns             |
| -------------------------- | ---------------- | ------------------- |
| `fake_boolean()`           | None             | true/false          |
| `fake_filename()`          | None             | "document.pdf"      |
| `fake_file_size(min, max)` | min, max: number | File size in bytes  |
| `fake_download_url()`      | None             | Full download URL   |
| `fake_mime_type()`         | None             | "application/json"  |
| `fake_file_extension()`    | None             | "pdf"               |
| `fake_status_message()`    | None             | "OK", "Not Found"   |
| `fake_api_version()`       | None             | "v1.2.3"            |
| `fake_version()`           | None             | "1.2.3"             |
| `fake_hex_color()`         | None             | "#A1B2C3"           |
| `fake_rgb_color()`         | None             | "rgb(255, 128, 64)" |
| `fake_locale()`            | None             | "en-US"             |
| `fake_timezone()`          | None             | "America/New_York"  |
| `fake_semver()`            | None             | "1.2.3"             |
| `fake_semver_prerelease()` | None             | "1.2.3-alpha.1"     |
| `fake_digit()`             | None             | 0-9                 |
| `fake_number(min, max)`    | min, max: number | Random integer      |
| `fake_float(min, max)`     | min, max: float  | Random float        |

#### File Generation (12 functions)

All return base64-encoded content.

| Function                                                                     | Parameters                              | Returns          |
| ---------------------------------------------------------------------------- | --------------------------------------- | ---------------- |
| `fake_pdf(text, pages)`                                                      | text: string, pages: number             | PDF base64       |
| `fake_png(width, height, color)`                                             | width, height: number, color: hex       | PNG base64       |
| `fake_jpeg(width, height, color, quality)`                                   | quality: 0-100                          | JPEG base64      |
| `fake_pdf_data_uri(text, pages)`                                             | Same as fake_pdf                        | Data URI         |
| `fake_png_data_uri(width, height, color)`                                    | Same as fake_png                        | Data URI         |
| `fake_jpeg_data_uri(...)`                                                    | Same as fake_jpeg                       | Data URI         |
| `fake_image_with_text(text, width, height, bg_color, text_color, font_size)` | Various                                 | PNG with text    |
| `fake_image_gradient(width, height, start_color, end_color, direction)`      | direction: horizontal/vertical/diagonal | Gradient PNG     |
| `fake_image_checkerboard(width, height, color1, color2, square_size)`        | Various                                 | Checkerboard PNG |
| `fake_image_noise(width, height, colored)`                                   | colored: boolean                        | Noise PNG        |
| `fake_image_stripes(width, height, color1, color2, stripe_width, direction)` | Various                                 | Striped PNG      |
| `fake_placeholder(width, height, text, bg_color, text_color)`                | Various                                 | Placeholder PNG  |
| `fake_avatar(initials, size, bg_color, text_color)`                          | initials: string, size: number          | Avatar PNG       |

#### Persistence Store (11 functions)

Thread-safe in-memory key-value store for cross-request state. For *typed*
state — entities with keys and relations — see [The Entity World](#the-entity-world)
and the `entity_*` functions.

| Function                                      | Parameters                 | Returns           | Example                                                       |
| --------------------------------------------- | -------------------------- | ----------------- | ------------------------------------------------------------- |
| `store_get(key)`                              | key: string                | Value or null     | `{{ store_get(key="counter") }}`                              |
| `store_set(key, value, ttl_seconds)`          | key, value, ttl (optional) | Empty string      | `{{ store_set(key="token", value="abc", ttl_seconds=3600) }}` |
| `store_set_nx(key, value, ttl_seconds)`       | Same as store_set          | Boolean (success) | `{{ store_set_nx(key="lock", value="1") }}`                   |
| `store_get_or_set(key, default, ttl_seconds)` | key, default, ttl          | Value             | `{{ store_get_or_set(key="total", default=100) }}`            |
| `store_incr(key)`                             | key: string                | Number            | `{{ store_incr(key="requests") }}`                            |
| `store_decr(key)`                             | key: string                | Number            | `{{ store_decr(key="remaining") }}`                           |
| `store_has(key)`                              | key: string                | Boolean           | `{{ store_has(key="token") }}`                                |
| `store_del(key)`                              | key: string                | Empty string      | `{{ store_del(key="expired") }}`                              |
| `store_clear()`                               | None                       | Empty string      | `{{ store_clear() }}`                                         |
| `store_keys()`                                | None                       | Array             | `{{ store_keys() }}`                                          |
| `store_ttl(key)`                              | key: string                | Number or null    | `{{ store_ttl(key="session") }}`                              |

**Example: OAuth flow with token validation**

```yaml
mocks:
- id: oauth-token
  match:
    url: "/oauth/token"
  response:
    template: |
      {%- set token = fake_jwt() -%}
      {%- set _ = store_set(key="oauth.token." ~ token, value="valid", ttl_seconds=3600) -%}
      {"access_token": "{{ token }}", "expires_in": 3600}
```

#### GraphQL Helpers (4 functions)

| Function                                                           | Parameters                           | Returns       |
| ------------------------------------------------------------------ | ------------------------------------ | ------------- |
| `graphql_error(message, code, path)`                               | message: string, code/path: optional | Error object  |
| `graphql_field_error(field, message, code)`                        | field: dot notation, message: string | Field error   |
| `graphql_type(name, kind, description, fields)`                    | Various                              | Type object   |
| `graphql_schema(queryType, mutationType, subscriptionType, types)` | Various                              | Schema object |

### Template Filters

| Filter               | Example                                  | Description        |
| -------------------- | ---------------------------------------- | ------------------ |
| `upper`, `lower`     | `{{ name \| upper }}`                    | Case conversion    |
| `default(value="x")` | `{{ query.page \| default(value="1") }}` | Default value      |
| `int`                | `{{ query.limit \| int }}`               | Convert to integer |
| `round(precision=2)` | `{{ price \| round(precision=2) }}`      | Round number       |
| `json_encode()`      | `{{ obj \| json_encode() }}`             | Encode as JSON     |
| `random_choice`      | `{{ ["a","b","c"] \| random_choice }}`   | Random element     |
| `base64_encode`      | `{{ "Hello" \| base64_encode }}`         | Base64 encode      |
| `base64_decode`      | `{{ encoded \| base64_decode }}`         | Base64 decode      |
| `urldecode`          | `{{ "Hello%20World" \| urldecode }}`     | URL decode         |
| `json_decode`        | `{{ '{"a":1}' \| json_decode }}`         | Parse JSON         |

### Loops & Conditionals

```yaml
# Loop (inside a template string)
# {% for i in range(start=0, end=5) %}
#   {"id": {{ i }}}{% if not loop.last %},{% endif %}
# {% endfor %}

# Conditional
# {% if query.premium == "true" %}premium{% else %}free{% endif %}
```

**Note:** Use `| int` filter on query params before math operations.

## Response Patching

Patches modify **upstream** proxied responses (not inline mock bodies). The `patch` field is a **top-level** field on
the mock definition (same level as `match`, `request`, `response`). Using `patch` alone (without `response`) triggers
**PatchUpstream** mode -- the request is forwarded to the real upstream and the response is patched before returning.

**Constraint:** `patch` cannot be combined with a full mock response (`body`, `template`, `json`, `file`).

```yaml
mocks:
- id: patch-example
  match:
    url: "/api/users/:id"
  patch:
    # JSON Patch (RFC 6902)
    operations:
    - op: add # add, replace, remove, copy, move, test
      path: /verified
      value: true

    # JSONPath (simpler)
    jsonpath:
      "$.total_count": 42
      "$.users[0].name": "Test"

    # Regex (for text/HTML)
    regex:
    - pattern: "https://production.example.com"
      replacement: "http://localhost:3000"

    # Headers
    headers:
      add:
        x-mock: "true"
      remove: ["x-frame-options"]
```

### Template Expressions in Patches

Patch values support Tera template expressions, rendered **after** the upstream response returns. This gives access to
both request context (captures, query params, headers, body) and upstream response data (status, headers, body).

```yaml
mocks:
- id: inject-user-id
  match:
    url: "/api/users/:id"
  patch:
    jsonpath:
      "$.injected_id": "{{ captures.id }}"
      "$.request_method": "{{ method }}"
    headers:
      add:
        x-upstream-status: "{{ response.status }}"
        x-user-id: "{{ captures.id }}"
    regex:
    - pattern: "placeholder"
      replacement: "{{ fake_name() }}"
```

Available template variables in patches include everything from normal templates (`method`, `path`, `query`, `headers`,
`body_json`, `captures`, `vars`) plus upstream response data:

| Variable                     | Description                     |
| ---------------------------- | ------------------------------- |
| `response.status`            | Upstream response status code   |
| `response.headers.<name>`    | Upstream response header value  |
| `response.body_json`         | Upstream response body as JSON  |
| `response.body_json.<field>` | Access specific response fields |

Template rendering is zero-cost for literal values -- only strings containing `{{` or `{%` are processed. On render
failure, the literal value is used as a fallback with a warning logged.

## Delay

Add a delay before the response is returned. The `delay` field is a **top-level** field on the mock definition (same
level as `match`, `response`, `patch`). It works in all modes: full mock responses, passthrough with patches, and
delay-only passthrough.

```yaml
mocks:
# Delay on a full mock response
- id: slow-mock
  match:
    url: "/api/slow"
  delay: "500ms"
  response:
    status: 200
    body: '{"slow": true}'

# Delay on a patched upstream response
- id: slow-patch
  match:
    url: "/api/patched"
  delay: "1s"
  patch:
    jsonpath:
      "$.patched": true

# Delay-only passthrough (no response or patch needed)
- id: slow-passthrough
  match:
    url: "/api/upstream"
  delay: "2s"
```

Supported duration formats: `100ms`, `2s`, `500us`.

## WebSocket Mocks

A mock with a top-level `ws:` block answers WebSocket upgrade handshakes instead of producing an HTTP response.
The mock is automatically scoped to `GET` requests carrying `Upgrade: websocket`, so plain GETs on the same path
fall through to other mocks. `response:` may only contribute extra headers; `sse`/`ws` cannot combine with a
response body, `patch`, request transforms, or a top-level `delay`.

```yaml
mocks:
- id: chat
  match: { url: "/ws/chat/:room" }
  ws:
    subprotocol: chat.v1                      # negotiated on the 101 response
    echo: false                               # echo unmatched messages back
    upstream: wss://real.example.com/ws       # optional passthrough target
    on_connect:                               # actions run when a client connects
      - send: { type: welcome }               # objects are JSON-stringified
      - delay: 100ms
      - send_template: '{"room":"{{ captures.room }}"}'
    on_message:                               # first matching rule wins
      - match: { exact: ping }
        actions: [ { send: pong } ]
      - match: { json_path: "$.type", equals: subscribe }
        actions: [ { send_template: '{"ok":true,"got":{{ body_json.type }}}' } ]
      - match: { regex: "^bin:" }
        actions: [ { send_binary: "AAECAwQ=" } ]   # base64 binary frame
      - match: { binary_base64: "AAEC" }           # binary frame equals these bytes
        actions: [ { send: got-exact-bytes } ]
      - match: { binary_prefix_base64: "//4=" }    # binary frame starts with these bytes
        actions: [ { send: got-prefix } ]
      - match: { any: true }
        actions: [ forward ]                  # relay to upstream (requires upstream)
      - match: { exact: bye }
        actions: [ { close: { code: 4000, reason: done } } ]
```

- **Actions**: `send` (string or object), `send_template` (Tera), `send_binary` (base64), `delay`, `echo`,
  `forward` (requires `upstream`), `close` (code 1000..=4999).
- **Message matchers**: exactly one of `exact`, `regex`, `json_path` (+ optional `equals`), `binary_base64`,
  `binary_prefix_base64`, `any`. `exact`/`regex`/`json_path` apply to text frames only; the `binary_*`
  matchers are the byte-frame counterparts (`binary_base64` whole-frame equality, `binary_prefix_base64`
  prefix); `any` matches both frame kinds.
- **Templates** render with the request context; the triggering message is exposed as `{{ body }}` /
  `{{ body_json }}` (the message is the body-analog of an HTTP mock).
- **Passthrough**: with `upstream` set, upstream frames relay to the client; unmatched client messages relay
  upstream when `echo` is off and no rule matches; the `forward` action relays the triggering frame explicitly.
- Script mocks get the MSW-compatible `ws.link(url)` API (see the scripting docs): connection listeners receive
  `{ client, server, params, info }` (`info.protocols` lists the subprotocols the client offered);
  `server.connect()` dials the link's absolute URL and auto-forwards both directions unless a `message`
  listener calls `event.preventDefault()`. Client `close` events carry `{ code, reason }`. `ws.link` also
  accepts a RegExp, tested against the bare path and against `ws(s)://host/path` reconstructions of the
  handshake (MSW's full-href idiom).
- **Scheme matching for RegExp links**: the Node interceptor lane sees the connection's real URL, so a RegExp
  pinning `ws://` never matches a `wss://` connection there (and vice versa). The TCP server lane cannot know
  whether TLS terminated in front of it, so it tests both `ws://` and `wss://` reconstructions — a
  scheme-pinned RegExp may false-positive on that lane. TCP-lane handlers also see the client URL
  reconstructed as `ws://host/path` regardless of the original scheme.
- **Lane dispatch difference**: the Node interceptor lane runs ALL `ws` handlers matching a connection (MSW
  semantics — every matching `ws.link` gets its listeners invoked). The TCP server lane serves a connection
  with the FIRST matching ws mock only (highest priority wins, like HTTP mocks); fanning one socket out to
  multiple handler mocks is not supported there.
- **TCP-lane client objects have no `.socket`**: MSW's `client.socket` exposes the intercepted browser/Node
  `WebSocket` instance. On the TCP server lane no such object exists (the peer is a real network socket), so
  the client handle offers `send`/`close`/`id`/`url` only.
- **Teardown**: removing or hot-reloading a mock closes its live connections with `1001 Going Away` instead of
  letting them keep running on the stale definition.
- **HAR import**: Chrome DevTools captures with `_webSocketMessages` convert into declarative `ws` mocks —
  server frames recorded before the first client message replay in `on_connect` with the recorded inter-frame
  delays, and each client message becomes an `on_message` exact-match (or `binary_base64`) rule replying with
  the frames that followed it. If the same client payload recurs with a different reply, the pairing is
  ambiguous and every server frame folds into the `on_connect` sequence instead.

## SSE Mocks (Server-Sent Events)

A mock with a top-level `sse:` block streams `text/event-stream` playback instead of a buffered body.

```yaml
mocks:
- id: ticker
  match: { url: "/api/ticker", methods: [GET] }
  sse:
    retry: 3000          # initial retry: field (milliseconds)
    keep_alive: 15s      # comment-ping interval for idle connections
    repeat: 3            # integer or "forever" (default 1)
    close_after: true    # false holds the connection open after playback
    events:
      - "hello"                                            # bare string = data-only event
      - { event: price, id: "1", data: { px: 123 } }       # objects serialize to JSON
      - { event: price, data_template: '{"px": {{ fake(type="int", min=1, max=999) }}}', delay: 200ms }
      - { retry: 5000 }                                    # retry-only event
```

- `delay` on an event sleeps before emitting it; `data_template` renders per emission with the request context
  (captures, query, headers, vars, fake functions).
- **Upstream passthrough**: `sse: { upstream: https://real.example.com/stream }` relays the real endpoint's
  frames to the client verbatim (exclusive with every playback field). A failed first dial answers
  `502 Bad Gateway`. Once the stream has opened, the relay follows EventSource reconnect semantics: a dropped
  or ended upstream stream is redialed after the current `retry:` delay (default 3s, updated by `retry:`
  frames) with the last seen `id:` sent as `Last-Event-ID`. An HTTP error status or a non-`text/event-stream`
  content type is terminal (no reconnect), and the pump stops as soon as the client disconnects. The same
  policy backs `server.connect()` in QuickJS script mocks (each drop surfaces as an `error` event before the
  redial); the Node lane's `server.connect()` uses `FerrimockEventSource` with identical behavior.
- Script mocks get the MSW-compatible `sse(path, resolver)` API: the resolver receives
  `{ request, params, cookies, client, server }`; `client.send({ id?, event?, data?, retry? })` emits frames,
  `client.close()` ends the stream, `client.error()` aborts the connection mid-stream. When the handler path
  is an absolute `http(s)://` URL, `server.connect()` dials that real endpoint and forwards its frames to the
  client; listeners on the returned source (`open`, `error`, `message`, or a named event) run first and can
  `event.preventDefault()` to swallow a frame.
- The declarative lane matches on path alone (curl-friendly); the Node `ferrimock` package's `sse()` additionally
  requires `accept: text/event-stream` for strict MSW parity (the accept check applies on the interceptor lane
  only — `FerrimockServer.listen()` serves the same handler curl-style).
- Removing or hot-reloading a mock ends its live streams.

## Request Transforms (Passthrough Mode)

Modify requests before forwarding to upstream. When any `request.*` field is set, the mock operates in **passthrough
mode** -- the request goes to the real upstream.

**Constraint:** Cannot combine with `response.body`, `response.template`, `response.file`, `response.template_file`, or
`response.json`. Use top-level `patch` instead to modify the upstream response.

### Header Modifications

```yaml
request:
  headers:
    add:
      x-trace-id: "{{ fake_uuid() }}"
      x-forwarded-by: "ferrimock"
    remove: ["x-real-ip", "x-debug-mode"]
```

### Query Parameter Modifications

```yaml
request:
  query:
    add:
      debug: "true"
      fields: "id,name"
    remove: ["access_token", "internal_trace"]
```

### Body Patches

```yaml
request:
  body:
    jsonpath:
      "$.metadata.proxied": true
      "$.source": "dev-proxy"
    regex:
    - pattern: "old-value"
      replacement: "new-value"
```

### Pre-Request Delay

```yaml
request:
  delay: "750ms" # Supports: ms, s, us
```

### Path Rewriting

```yaml
request:
  rewrite_path: "/api/v2/users/{{ captures.user_id }}/collaborations/{{ captures.collab_id }}"
```

### Forward to Alternative Upstream

```yaml
request:
  forward_to: "https://staging-api.example.com"
```

### Custom Timeout

```yaml
request:
  timeout: "120s"
```

### Combined Request Transforms + Response Patches

```yaml
mocks:
- id: api-migration
  match:
    methods: ["POST"]
    url: "/api/v1/collaborations/:collab_id"
  request:
    rewrite_path: "/api/v2/collaborations/{{ captures.collab_id }}"
    forward_to: "https://api-v2.example.com"
    timeout: "30s"
    headers:
      add:
        x-api-version: "2.0"
    body:
      jsonpath:
        "$.schema_version": "2.0"
  patch:
    jsonpath:
      "$.api_version": "v1-compat"
    headers:
      add:
        x-api-compat: "v1"
```

See `mocks/examples/` for complete examples.

## Faults

`delay` slows a response down; `network_error` drops the connection instead of answering. Headers commit, then the
body stream errors, so the caller sees a transport failure — `fetch` raises `TypeError`, curl reports an aborted
transfer.

```yaml
- id: boom
  match:
    methods: ["GET"]
    url: "/api/flaky"
  network_error: true

- id: slow-boom
  match:
    methods: ["GET"]
    url: "/api/timeout"
  delay: "2s"          # fail slowly
  network_error: true
```

`network_error` is exclusive with `response`, `patch`, `sse`, `ws`, and request transforms — the connection drops, so
there is no response to shape.

Flakiness needs no dedicated feature: branch on `get_random`.

```yaml
- id: flaky
  match: { methods: ["GET"], url: "/api/orders" }
  response:
    template: |-
      {%- if get_random(start=0, end=100) < 30 -%}
      {"status": 503, "body": {"error": "upstream unavailable"}}
      {%- else -%}
      {"status": 200, "body": {"ok": true}}
      {%- endif -%}
```

Pin the flakiness with `--seed` when you need the same failures every run.

## Stateful Scenarios

State lives in the persistence store, so a mock can answer differently per call without a separate scenario
mechanism. A template that emits `{status, headers, body}` controls the whole response.

```yaml
- id: order-lifecycle
  match: { methods: ["GET"], url: "/api/order" }
  response:
    template: |-
      {%- set n = store_incr(key="order.calls") -%}
      {%- if n == 1 -%}
      {"status": 201, "headers": {"x-state": "created"}, "body": {"state": "created"}}
      {%- elif n == 2 -%}
      {"status": 200, "headers": {"x-state": "processing"}, "body": {"state": "processing"}}
      {%- else -%}
      {"status": 410, "headers": {"x-state": "done"}, "body": {"state": "done"}}
      {%- endif -%}
```

`store_get`, `store_set`, `store_incr`, `store_decr`, `store_set_nx`, `store_get_or_set`, `store_has`, `store_del`,
`store_keys`, `store_ttl` and `store_clear` are all available in templates; `GET /__ferrimock/store` inspects the
state and `DELETE /__ferrimock/store` resets it between tests.

## The Entity World

The persistence store above is untyped scratch state — counters, flags. When the
thing you are mocking has *types* with keys and relations, that belongs in the
**world**: a seeded, relational store the engine owns.

Rule of thumb: if it has a type and a key in the API you are mocking, it is the
world; if it is a counter or a flag for your test, it is the store.

### Where entities come from

A GraphQL schema or an OpenAPI document. Either declares *entities*, not routes
— a `.graphql` has nowhere to say it is served at `https://api.example.com/graphql`
rather than on localhost, so a schema on its own registers nothing and the
loader tells you so.

```yaml
world:
  schemas:
    - filestore.graphql                  # a bare .graphql beside the mocks is picked up too
    - filestore-content.openapi.yaml     # and so is a `.openapi.yaml` / `.openapi.json`
  seed: 42                   # same seed, same world; defaults to --seed
  count: 12                  # instances per entity when the entity does not say
  counts: { User: 25, Folder: 200 }
  lenient: false             # repair malformed descriptions rather than refusing
  cascade_delete: true       # a delete takes dependent records with it
```

Both front ends compile into the *same* graph, so a `User` a GraphQL schema
declares and a `User` an OpenAPI document describes are one `User` with one set
of instances. Their *fields* are merged, not replaced: the REST document knows
about `email`, the GraphQL schema knows about `karma`, and the instances carry
both — otherwise whichever schema loaded second would leave the other serving
payloads its own schema rejects. The first declaration wins a conflict, except
where the later one says strictly more (a link beats the scalar key carrying
it). Entity names declared by more than one schema are reported by
`ferrimock world explain`.

There is one world per process, so several collections may add schemas to it but
only one may set the seed — a second, different value is refused by file name.
The *same* file setting a different value is an edit, not a disagreement, so a
hot reload applies it.

`cascade_delete: false` refuses a delete that would orphan children instead of
removing them, which is what an API enforcing referential integrity does; the
refusal is a 409, and the children are still there.

### Saying what a field should hold

A schema types a field as `String` and stops there. Which string it is stays a
product decision, so `world:` takes it:

```yaml
world:
  schemas: [api.yaml]

  scalars:                    # by a kind of value
    Money:    { float: { min: 1, max: 999.99 } }
    date-time: timestamp

  fields:                     # by a place
    User.email:   email
    Folder.name:  headline
    Order.status: { one_of: [pending, shipped, cancelled] }
    Order.total:  { float: { min: 5, max: 500 } }
    User.badge:   "{{ fake_word() | upper }}"
    "*.slug":     slug
```

`Entity.field` beats `*.field` beats `scalars:`, because the more precisely a
rule names its target the likelier it is to be the one that was meant.

`scalars:` matches a **GraphQL custom scalar** by name and an **OpenAPI
`format`** by name — both say "this is a Money" without naming a field, so both
front ends are covered by one entry.

A value is either a generator name or, when it looks like one, a Tera template.
The named form resolves at load and costs nothing per request; the template is
the escape hatch, rendered inside that field's own seeded stream so it stays as
reproducible as everything around it, and only the fields that ask for it pay
for it. Named generators: `uuid`, `email`, `username`, `person_name`,
`headline`, `paragraph`, `url`, `image_url`, `ip`, `phone`, `filename`,
`mime_type`, `token`, `etag`, `numeric_id`, `api_endpoint`, `timestamp`, `date`,
`boolean`, `word`, `slug`. Shapes: `{ int: {min,max} }`, `{ float: {min,max} }`,
`{ one_of: [...] }`, `{ constant: ... }`, `{ pattern: "..." }`.

Rules are resolved **into the entity graph** rather than consulted per read,
which is what makes them compatible with the rest of the world: values still
derive from `(seed, entity, ordinal, field path)`, a record still builds without
its neighbours, and a record a client *created* goes through the same generator
as a seeded one — so an override applies to both without knowing either exists.

Two things a rule may not touch, because the store owns their values: a **key
field**, and a field **carrying a relation**. Overriding either gives a world
whose keys do not address it or whose links resolve to nothing, so both are
refused by name. A rule that matches nothing is reported the same way — a typo
or a renamed field should surface at load, not from a payload that looks almost
right. Both show up in `ferrimock world explain` and as a warning when the
collection loads.

`--watch` watches the schemas alongside the collections, so editing a `.graphql`
or a document rebuilds the world and re-mounts the mocks serving it. A field
removed from a schema is removed from the world: each source's declaration is
kept apart, so a reload replaces that source's contribution rather than adding to
whatever was there before.

A bare `.yaml` or `.json` is never auto-detected as a document: those are mock
collections, and sniffing a file's contents to tell them apart breaks silently.
An OpenAPI document picked up from a mocks directory says so in its name
(`*.openapi.yaml`, `*.openapi.yml`, `*.openapi.json`); a file named under
`world.schemas` is loaded whatever it is called, read as GraphQL for
`.graphql`/`.gql` and as OpenAPI for `.yaml`/`.yml`/`.json`.

OpenAPI 3.0 and 3.1 are both read. Swagger 2.0 is refused by name rather than
half-understood — convert it first.

### What an OpenAPI document says, and what has to be inferred

A GraphQL schema states which types have identity. A document does not, so
identity is read off the shape of the surface. Every fact carries the rule that
produced it and how much to trust it, and `ferrimock world explain` prints both:

| rule | what it reads |
| --- | --- |
| `collection-item-pair` | `/folders` returning `[Folder]` beside `/folders/{folder_id}` returning `Folder` — so `Folder` has identity, keyed by the field that parameter addresses. A path addressing one thing by several parameters that each name a field — `/repos/{owner}/{repo}` — is keyed by all of them, because two owners can each have a repo called `docs` |
| `schema-ref` | a `$ref` from one entity's schema into another |
| `path-nesting` | `/folders/{folder_id}/items` — the child carries the parent key |
| `spec-link` | a `links` object, which states the relation and the carrying field outright |
| `foreign-key-name` | `user_id` where `User` is an entity. A guess, reported as one. When the schema already links to that entity — a `$ref`'d `customer` beside `user_id` — the scalar becomes that link's *carrier* rather than a second link, so the key and the object it is written beside always name one instance |
| `vendor-extension` | whatever a `ConsolidationProfile` reads out of `x-` keys |

Name matching matches *names*, never meanings: `owner_id` finds an entity called
`Owner`, and nothing teaches the engine that an owner is a `User`. That is what
a profile is for: `ConsolidationProfile::spec_relation` is asked about every
field carrying `x-` keys, and `pagination_dialect` names what this API calls a
limit and an offset. Domain knowledge lives there, never in the engine.

### Serving it

`serve:` is a mode alongside `response:`, `patch:`, `sse:` and `ws:` — not a
response body, but a protocol behavior bound to a matched URL. `match` says
where the API answers; `serve` says which schema answers there.

```yaml
mocks:
  - id: filestore-graphql
    match:
      POST: https://api.example.com/graphql
    serve: graphql
```

An OpenAPI document mounts the same way, with `serve: rest`. The mock supplies
the base URL and the document supplies the rest of every path, so `match` names
no method — the operations carry their own:

```yaml
  - id: filestore-rest
    match:
      url: https://api.example.com/2.0     # base; operations supply path and method
    serve: rest
```

With more than one schema of a protocol in the world, name which:

```yaml
    serve: { protocol: graphql, schema: schemas/filestore-internal.graphql }
    serve: { protocol: rest, schema: schemas/filestore-content.openapi.yaml }
```

Mounting is N-to-M: two schemas at two URLs, or the same schema at two URLs.
Everything per-endpoint is per-mock — `priority`, `scope`, `delay`, extra
`response.headers`, header matchers.

### What `serve: rest` answers

Each operation is classified once, at mount time:

| operation | behavior |
| --- | --- |
| `GET /folders/{id}` | read one by key; a key nobody stored is a 404, not an invented record |
| `GET /folders` | read a page, in whatever envelope the response declared |
| `GET /folders/{id}/items` | read the parent's children through the relation — whether inference put it there or the schema declared `items` on `Folder` inline |
| `POST /folders` | create, answering with the status the document declared |
| `PUT` / `PATCH` `/folders/{id}` | replace / merge |
| `DELETE /folders/{id}` | remove |
| anything else | answered from the declared response shape, and *counted* |

That last row is the honest one. `ferrimock world explain` leads with how many
operations are backed by the world and names the ones that are not; a mock that
invents data for half an API must not look like one that does not.

Query parameters are read as the world's own query: pagination
(`limit`/`offset`/`page`, plus whatever a profile's pagination dialect names),
`sort=-name`, and any parameter naming a field of the entity as a filter.
`?size[gt]=900` is a spelling of the operator syntax the world already
takes — `eq`, `ne`, `in`, `gt`, `gte`, `lt`, `lte`, `contains` — not a filter
language of its own. When a document declares its query parameters, only those
filter; a document that declares none leaves every field open.

`POST /folders/{id}/copy` is deliberately *not* a creation. Only a collection
path creates, or a sub-collection the graph already knows about
(`POST /folders/{id}/items`); an action on one item drops to the bottom rung
rather than inserting a record nobody asked for.

### What a generated value looks like

A field is answered by the strictest thing the schema said about it, in this
order:

1. a declared `pattern` — `^[A-Z]{3}-[0-9]{4}$` answers `SWY-4324`. A realistic
   value that already satisfies the pattern is kept, so a permissive `^.+$` does
   not replace an email with noise. Length bounds are honoured alongside it: the
   walk is retried with a wider repetition budget until it satisfies both;
2. a declared `format`, then the field's name, then its type — `created_at` is a
   timestamp, `email` is an email, `slug` is a slug rather than a sentence;
3. `minimum` / `maximum` / `minLength` / `maxLength`, which are respected rather
   than approximated.

Text is composed rather than lorem. A `title`, `name` or `description` is read
by a person, and `perferendis non adipisci asperiores` is what makes a mocked
screen look mocked; the generator builds short noun phrases and real sentences
from a generic product vocabulary instead. A profile that knows the domain can
answer ahead of it.

What *owns* a field settles the ambiguous names: `name` on a `User` is a
person's, on a `File` it is a filename, and on a `Folder` it is a folder's.
Explicit spellings — `first_name`, `display_name` — are a person's wherever they
appear.

Timestamps within one record happen in the order their names say. Every field is
still drawn independently, which is what keeps a record derivable without its
neighbours; the values a record already has are simply dealt back out in
lifecycle order, so `created_at <= updated_at <= archived_at` holds without any
field depending on another. Only values that compare are reordered — a date is
never sorted against an instant.

A key is rendered as the kind the schema declared it. `id: { type: integer }` is
keyed `1, 2, 3` and answers `{"id": 1}` — not a uuid, and not `"1"` — so
`GET /users/1` is the request a client would actually make. The same holds for a
foreign key: it carries the target's key in the target's declared kind.

Writes are part of the world, not layered beside it. A created record carries
the same links a seeded one does, and it joins the collections it points at — a
post written against a user appears among that user's posts. The link may be
named however the caller's schema names it: `{"author": "<id>"}`,
`{"author": {"id": "<id>"}}`, `{"authorId": "<id>"}` and the carrier field
itself all mean the same link.

### How children are shared out

Not evenly. Each parent draws a weight and the children are cut in proportion,
so one folder holds four hundred files and several hold none — which is the
shape real data has, and the shape a client's pagination and empty states have
to survive.

Because the answer is a *range* rather than a hash per child, two things follow.
Reading one parent's children touches only that parent's children instead of
filtering every child in the world. And counting them is arithmetic: a field
named `item_count` beside an `items` link is answered with the number the list
endpoint would return, adjusted for whatever has been written since. A
`*_count` whose stem names no relation — `word_count` — is left as an ordinary
number.

### Overriding part of it

A schema-derived route sits at priority 50, below the default 100, so an
ordinary mock outranks it. Nothing special is involved.

```yaml
  - id: quota-exceeded
    match:
      POST: https://api.example.com/graphql
      graphql: { mutation: CreateFolder }      # this operation only
    response:
      json: { errors: [{ message: Storage quota exceeded }] }
```

In JS, a handler that returns `undefined` declines and falls through to the
schema, so an override can be conditional.

GraphQL mounts as one mock matching any operation — the operation name is chosen
by the client, not the schema, so the backend cannot enumerate them in advance.
That means `verify()` on the mount asserts the endpoint's total; assert on the
override mock for a per-operation count.

An OpenAPI document is the opposite: it *designs* its endpoints, so it expands
to one mock per operation, named `{mock-id}#{operationId}`. Coverage names the
endpoints, `verify("filestore-rest#getFolder", Exactly(1))` works, and overriding one
endpoint is an ordinary higher-priority mock at that path:

```yaml
  - id: uploads-down
    match:
      POST: https://api.example.com/2.0/files/content
    response:
      status: 503
```

### Reading and writing the same entities

From a template:

```yaml
response:
  template: |-
    {%- set page = entity_list(type="Folder", filter={"owner": captures.userId}, sort="-createdAt", limit=25) -%}
    {"entries": {{ page.records | json_encode }}, "total_count": {{ page.total }}}
```

`entity_get`, `entity_list`, `entity_related`, `entity_count`, `entity_create`,
`entity_update`, `entity_delete` and `entity_types` are available in templates.

From JS — the same surface on Node and on the embedded QuickJS runtime:

```ts
import { http, HttpResponse, world } from 'ferrimock'

http.post('https://api.example.com/2.0/users', async ({ request }) => {
  const { name } = await request.json()
  const user = world.create('User', { name })   // the schema's `users` query sees it
  return HttpResponse.json(user, { status: 201 })
})
```

`world.types()`, `count`, `get`, `list`, `related`, `create`, `update`,
`replace`, `delete`, `reset`, `pendingWrites`. `get` returns `undefined` for a
miss. Filters take an operator object where you need one:
`{ filter: { age: { gt: 30 } } }` — `eq`, `ne`, `in`, `gt`, `gte`, `lt`, `lte`,
`contains`. Sort with `-field` for descending.

Over HTTP, on the standalone server:

```bash
GET    /__mock/world                    # entities, seed, pending writes
GET    /__mock/world/User?limit=25&sort=-name&status=active
GET    /__mock/world/User/{key}
POST   /__mock/world/User               # create
PATCH  /__mock/world/User/{key}         # merge fields into one instance
DELETE /__mock/world/User/{key}         # 409 when `cascade_delete: false` refuses it
DELETE /__mock/world                    # reset to the seeded world
```

An embedder mounting the management API gets the same under
`/__ferrimock/world`, plus `PATCH` and `POST /__ferrimock/world/reset`.

### Determinism and test isolation

The world is deterministic given the seed; the *state* is deterministic given
the seed plus the sequence of writes. Field values derive from seed + entity +
ordinal + field path, so they are the same value however and whenever they are
asked for, and adding a schema does not disturb entities already in play.

Writes are shared, which is the point — and means they leak between tests unless
you clear them. `DELETE /__mock/world` (or `world.reset()`) drops every write and
leaves exactly what the seed derives; `world.pendingWrites()` shows whether any
are outstanding.

## Verifying What Ran

Every match is counted — no setup, no spy inside a resolver, and it works for declarative mocks that have no
resolver at all.

```rust
use ferrimock::engine::registry::Expected;

registry.match_count("get-user");                       // 3
registry.verify("get-user", Expected::Exactly(3))?;     // Ok(3)
registry.verify("get-post", Expected::Never)?;          // Ok(0)
registry.reset_match_counts();                          // between tests
```

A failed verification names the mock, the expectation and the reality — and says so when the id matches no
registered mock, which otherwise reads as an innocent `expected 1, got 0`:

```text
mock "get-usr" expected to serve exactly 1 request(s), served 0 (no mock is registered with that id)
```

Over HTTP:

| Endpoint | Method | Description |
| -------- | ------ | ----------- |
| `/__ferrimock/calls` | GET | Counts for every mock that served a request, busiest first |
| `/__ferrimock/calls` | DELETE | Reset all counts |
| `/__ferrimock/calls/{id}` | GET | One mock's count |
| `/__ferrimock/calls/{id}?times=2` | GET | Verify; answers 409 when it fails, so CI can assert on the status alone |

`?at_least=`, `?at_most=` and `?never=true` are also accepted. The standalone `mock serve` exposes the same counts at
`/__mock/calls`, and `/__mock/list` carries a `match_count` per mock.

From Node:

```ts
server.matchCount('get-user')   // 3
server.matchCounts()            // [{ mockId: 'get-user', count: 3 }]
server.resetMatchCounts()
server.listHandlers()           // each entry carries matchCount
```

Note: counting happens where a mock is *served*. The Node interceptor's WebSocket lane dispatches a connection to
every matching `ws` handler by design and is deliberately side-effect free, so those do not count.

## Why Didn't My Mock Match?

The engine can replay a request against every mock and report the verdict criterion by criterion. Criteria are
evaluated through the same predicates the hot path uses, so an explanation can never disagree with a real match.

```bash
ferrimock mock test -m GET /api/users/123 --debug
```

```text
Mock: accept-json (priority: 200)
  ✗ NO MATCH
  [+] method matches GET
  [+] url matches exact "/api/users/123"
  [-] header accept expected regex "application/json", got (absent)

Closest: accept-json (priority 200): header accept expected regex "application/json", got (absent)
```

A consumed `once` mock shows as `[disabled]` with every criterion still passing — the usual cause of "it worked the
first time".

The standalone server puts the same explanation in its 404 body:

```json
{
  "error": "No matching mock found",
  "method": "GET",
  "path": "/api/secure/data",
  "explanation": "GET /api/secure/data matched no mock; closest: bearer-token-auth (priority 200): header authorization expected regex \"^Bearer\\s+...\", got (absent)",
  "nearMisses": [{ "mockId": "bearer-token-auth", "priority": 200, "enabled": true, "failed": ["..."] }]
}
```

Pass `--no-explain` to `mock serve` to drop it. Explaining a miss walks the whole registry, so embedders get it only
on request:

```rust
let matcher = MockMatcher::new(registry);
matcher.set_explain_unmatched(true);

// Or build the report directly — never consumes a `once` mock, records a
// call, or touches the match cache.
let report = matcher.explain(&Method::GET, "/api/users/1", None, &headers, None);
println!("{}", report.summary());
for attempt in report.near_misses(3) {
    println!("{attempt}");
}
```

## Coverage And Unmatched Requests

Match counts answer "did this mock run". Two run-level reports answer the questions a replayed corpus actually raises:
which of the loaded mocks never ran, and which requests the corpus could not serve at all.

```rust
let coverage = registry.coverage();
coverage.percent_covered();     // 76.19
coverage.served;                // busiest first
coverage.unused;                // highest priority first, each naming its source_file
```

`coverage()` reports the mocks currently registered, so counts left behind by a mock that was removed — a hot reload, a
dropped scope — do not appear: a coverage report names files a reader can go and fix. A consumed `once` mock reads as
served with `enabled: false`, which is what separates it from one that never ran.

Unmatched requests are accumulated only when asked for, because a proxy that forwards misses upstream would otherwise
pay an allocation on every request:

```rust
matcher.set_track_unmatched(true);

let report = registry.unmatched_requests();
report.requests;    // distinct request lines, most frequent first
report.total;       // every miss, including any not retained
report.dropped;     // non-zero means `requests` is an undercount
registry.reset_unmatched();
```

Repeats of one request line fold into a single entry with a `count`, so a replay run polling a missing endpoint does not
bury the rest. The query string is part of the line: a changed query is the usual difference between a hit and a miss.
Past 1000 distinct lines further misses are counted but not retained, and `dropped` says how many.

Entries carry no explanation. Mocks reload, so a stored one goes stale — post a line back to `/inspect` to find out why
it missed, and get an answer that reflects the corpus as it is now.

Over HTTP:

| Endpoint | Method | Description |
| -------- | ------ | ----------- |
| `/__ferrimock/coverage` | GET | Served and unused mocks, with `totalMocks` and `percentCovered` |
| `/__ferrimock/unmatched` | GET | Request lines that matched nothing, most frequent first |
| `/__ferrimock/unmatched` | DELETE | Forget them, to scope the next report to one test |

`mock serve` exposes the same two at `/__mock/coverage` and `/__mock/unmatched`, and tracks unmatched requests from the
start — nothing sits downstream of a standalone server, so every miss there is a real one.

## What To Widen

A mock recorded from one URL matches that URL. The next run asks for a different id, misses, and the corpus has to be
re-recorded. Cross-referencing the two reports above says which mocks were *one generalization* away from serving a
miss:

```rust
let unmatched = registry.unmatched_requests();
for s in ferrimock::engine::suggest(&matcher, &unmatched.requests) {
    println!("{} ({:?}): {} -> {}  [{} requests]",
        s.mock_id, s.source_file, s.current, s.proposed, s.request_count);
}
```

```text
get-user (mocks/users.yaml): /api/users/123 -> /api/users/:id  [47 requests]
folder-items (mocks/folders.yaml): format=full -> format=<any>  [12 requests]
```

Over HTTP at `/__ferrimock/suggestions` (and `/__mock/suggestions` on `mock serve`), which also returns `uncovered` —
the request lines no widening reaches, so they need a new mock rather than an edit.

Proposals are ordered by how many requests each recovers, so the first line is the single edit that buys the most.

**What is deliberately not suggested.** The report is only useful if applying it works, so a proposal is made only when
one criterion rejected the request and widening it is safe:

- A request rejected on *two* criteria gets nothing — widening the url while the query still rejects it would be a
  half-fix reported as a fix.
- A **method** mismatch is never proposed. Serving `POST` from a mock recorded for `GET` changes what the mock means.
- **Headers** and **bodies** are never proposed against: the unmatched log keeps request lines, not payloads, so those
  rejections cannot be re-evaluated. Post the line to `/inspect` instead.
- A **disabled** mock is skipped — re-enabling it is the fix, and coverage already reports it as disabled.
- `/folder/0` is **not** a widening of `/folder/0/items`. Two paths that differ structurally address different
  endpoints, and pointing a reader at the wrong file is worse than saying nothing.

Only pinned values are widened: a query matcher already spelled as a regex, `present` or `absent` is left alone, since
each was written to discriminate and relaxing it would break what the mock exists for.

## Recording & Playback

```bash
ferrimock mock record --port 3006                          # Start recording proxy
ferrimock mock record --format har                         # HAR format
ferrimock mock convert recording.har mocks.yaml --matching pattern  # Convert
ferrimock mock consolidate large.json optimized.yaml       # Reduce 70-90%
```

### Recording Config

```yaml
mock:
  recording:
    filter_url: "^/api/.*"
    capture_success_only: true
    exclude_static: true
```

Pattern detection auto-converts IDs (`/users/123` -> `/users/\d+`) and UUIDs.

### HAR File Import

`mock convert` imports HAR files (from browser DevTools or other tools) into replay-ready mock collections. By default
it applies several transformations to produce clean, proxy-compatible output:

**URL normalization** -- Absolute URLs (`https://api.example.com/v2/users/me`) are converted to relative paths
(`/v2/users/me`) so the mock engine can match them.

**Domain filtering** -- By default, all domains are included. Use `--domains api.example.com` to limit to specific
domains, or `--extra-domains staging.example.com` to add additional domains to the default set.

**Static asset filtering** -- Requests for `.js`, `.css`, `.png`, `.woff2` and other static files are excluded. Use
`--static-assets` to include them.

**Header stripping** -- Sensitive headers (`Authorization`, `Cookie`, `Set-Cookie`) and infrastructure headers (`date`,
`server`, `x-envoy-*`, `alt-svc`) are removed. Content headers like `content-type` are preserved.

**Query param sanitization** -- Sensitive query parameters (`access_token`, `api_key`, `token`) are stripped from URLs.
Surviving parameters keep the exact spelling they were recorded with, percent-encoding included: a mock has to match the
request it was made from, and rebuilding the query from decoded pairs turns `cursor=eyJwIjoxfQ%3D%3D` into
`cursor=eyJwIjoxfQ==`, which no longer matches anything. When a credential is removed from the middle of a query, the
parameters around it move out of the URL and into `match.query` matchers, so the mock ignores the token rather than
demanding a request without one.

**Body extraction** -- With `--extract-bodies`, large response bodies (>100KB or binary content types) are saved to
separate files in a `bodies/` directory rather than inlined in the YAML/JSON. Adjust the threshold with
`--body-threshold-kb`.

**Specificity ranking** -- A recording of a bare endpoint gets a lower `priority` than one that pins a query. An exact
pattern naming no query still matches a request that carries one, so without this a recording of `/collaborators` would
answer every `/collaborators?sortColumn=...` call recorded beside it.

**Ordered replay of repeated requests** -- A trace records a conversation, not a table: the same call is answered
differently as the session moves on -- a listing before and after an upload, a job that is pending and then done. Where
a recording answered one request several ways, the mocks for it are chained with `once`, so replaying hands them back in
the order they were recorded and the last one keeps answering afterwards. Repeats that were answered identically are
left as a single mock. Set `sequence_repeated_requests: false` in `HarLoadOptions` to get the old behaviour, where the
first recording answers every time and the rest are unreachable.

```bash
# Basic: import HAR to clean mock collection
ferrimock mock convert traffic.har mocks.yaml

# Include all domains
ferrimock mock convert traffic.har mocks.yaml --all-domains

# Keep absolute URLs, extract large bodies
ferrimock mock convert traffic.har mocks.yaml --absolute-urls --extract-bodies

# Answer every repeat with the first recording, rather than replaying in order
ferrimock mock convert traffic.har mocks.yaml --flatten-repeats

# Full raw import (no filtering or normalization)
ferrimock mock convert traffic.har mocks.yaml \
  --absolute-urls --all-domains --static-assets \
  --keep-sensitive-headers --keep-infra-headers --browser-headers
```

## Mock Management API

REST API for runtime control (prefix: `/__ferrimock/`).

| Endpoint              | Method               | Description                                    |
| --------------------- | -------------------- | ---------------------------------------------- |
| `/status`             | GET                  | System status                                  |
| `/enable`, `/disable` | POST                 | Toggle system                                  |
| `/reload`             | POST                 | Reload from disk                               |
| `/mocks`              | GET                  | List mocks (filter, sort, paginate)            |
| `/mocks`              | POST                 | Create mock                                    |
| `/mocks`              | DELETE               | Delete by scope/filter (`?scope=`, `?filter=`) |
| `/mocks/:id`          | GET/PUT/PATCH/DELETE | CRUD operations                                |
| `/bulk`               | POST                 | Bulk operations (atomic)                       |
| `/inspect`            | POST                 | Test request matching                          |
| `/store/:key`         | GET/POST/DELETE      | Store operations (untyped key/value)           |
| `/world`              | GET                  | Entities, seed, pending writes                 |
| `/world/reset`        | POST                 | Drop every write, keep the seeded world        |
| `/world/:entity`      | GET/POST             | List (`?limit=&skip=&sort=&<field>=`) / create |
| `/world/:entity/:key` | GET/PATCH/DELETE     | Read, merge fields, remove                     |

### Test Integration (Playwright)

```typescript
const mockApi = 'http://localhost:3006/__ferrimock';

test.beforeEach(async ({ request }) => {
  await request.post(`${mockApi}/mocks`, {
    data: { config: { id: 'test-mock', match: { url: '/api/test' }, response: { body: '{}' } } }
  });
});

test.afterEach(async ({ request }) => {
  await request.delete(`${mockApi}/mocks/test-mock`);
});
```

### Programmatic SDK (`ferrimock-mock-api`)

A high-level TypeScript package providing a fluent mock builder and test isolation helpers. Wraps the REST API so you
never need to construct raw HTTP requests.

```bash
npm install ferrimock-mock-api
```

#### MockBuilder

Fluent builder for constructing mock configurations. Static factories for each HTTP method, chainable methods for all
options, and auto-generated deterministic IDs. Covers the full mock config surface: match refinements, response types,
request transforms, response patches, and template variables.

```typescript
import { MockBuilder } from "ferrimock-mock-api";

// Simple -- auto-generates ID "get-api-users-id"
MockBuilder.get("/api/users/:id")
  .respondWithJson(200, { id: "123", name: "Test User" })
  .build();

// Structured response config (any combination of status, body, json, template, file, headers)
MockBuilder.get("/api/data")
  .respond({ status: 200, json: { items: [] }, headers: { "X-Total": "0" } })
  .build();

// Template response with captures
MockBuilder.get("/api/users/:id")
  .respondWithTemplate(200, '{"id":"{{ captures.id }}","name":"{{ fake_name() }}"}')
  .build();

// Match refinements -- header, query, body conditions
MockBuilder.post("/api/search")
  .matchHeader("Content-Type", "application/json")
  .matchQuery("limit", "10")
  .matchBody("query", "test")
  .respondWithJson(200, { results: [] })
  .build();

// Response headers (unambiguous -- withResponseHeader, not withHeader)
MockBuilder.get("/api/data")
  .respondWith(200, "ok")
  .withResponseHeader("X-Cache", "HIT")
  .withResponseHeader("X-Request-Id", "abc")
  .build();

// Template variables (accessible as {{ vars.key }} in templates)
MockBuilder.get("/api/users/:id")
  .withVars({ defaultRole: "viewer" })
  .respondWithTemplate(200, '{"role":"{{ vars.defaultRole }}"}')
  .build();

// Request transforms (passthrough mode -- forward to different upstream)
MockBuilder.get("/api/v1/:path")
  .forwardTo("https://staging.example.com")
  .rewritePath("/api/v2/{{ captures.path }}")
  .build();

// Response patches (modify upstream response before returning)
MockBuilder.get("/api/users/:id")
  .patchJsonPath({ "$.name": "overridden", "$.role": "admin" })
  .build();

// Clone and modify existing configs
const base = MockBuilder.get("/api/users").respondWithJson(200, { users: [] }).build();
MockBuilder.from(base).withScope("test-admin").withPriority(300).build();
```

**Factories:** `get()`, `post()`, `put()`, `patch()`, `delete()`, `method(m, url)`, `match(config)`, `from(config)`.

**Response:** `respond(config)`, `respondWith(status, body?)`, `respondWithJson(status, json)`,
`respondWithTemplate(status, template)`, `respondWithFile(status, path)`, `respondWithTemplateFile(status, path)`,
`withResponseHeader(name, value)`.

**Match refinements:** `matchHeader(name, value)`, `matchQuery(name, value)`, `matchBody(path, value)`.

**Passthrough:** `forwardTo(url)`, `rewritePath(path)`, `transformRequest(config)`.

**Response patches:** `patchJsonPath(patches)`, `patchResponse(config)`.

**Metadata:** `withId()`, `withScope()`, `withPriority()`, `withDelay()`, `withDescription()`, `withVars()`,
`enabled()`.

#### MockManager

Orchestrates mock lifecycle -- create, bulk setup, teardown, health checks. Accepts both `MockCreateConfig` objects and
`MockBuilder` instances directly (calls `.build()` automatically).

```typescript
import { MockManager, MockBuilder } from "ferrimock-mock-api";

const manager = new MockManager({
  baseUrl: "http://localhost:3006",  // default
  defaultScope: "my-test-suite",     // auto-applied to all mocks
  fixturesDir: "./tests/fixtures",   // base dir for respondWithFile/respondWithTemplateFile
});

// Wait for mock server to be ready
await manager.waitForReady(10_000);

// Create -- accepts builders directly, no .build() needed
await manager.create(
  MockBuilder.get("/api/health").respondWithJson(200, { status: "ok" })
);

// Atomic bulk setup -- also accepts builders
await manager.setup([
  MockBuilder.get("/api/users").respondWithJson(200, { users: [] }),
  MockBuilder.get("/api/users/:id").respondWithJson(200, { id: "1" }),
  MockBuilder.post("/api/users").respondWith(201),
]);

// File-based responses -- relative paths resolved against fixturesDir
await manager.create(
  MockBuilder.get("/api/data").respondWithFile(200, "data.json")
  // Resolves to: /abs/path/to/tests/fixtures/data.json
);

// Verify a mock exists and is enabled
await manager.verify("get-api-users");  // true

// Tear down all tracked mocks (by scope if defaultScope set, otherwise by ID)
await manager.teardown();
```

The `fixturesDir` option resolves relative file paths in `respondWithFile()` and `respondWithTemplateFile()` to absolute
paths before sending to the mock server API (which requires absolute paths for API-created mocks). If not set, relative
paths resolve against `process.cwd()`. Absolute paths are always left unchanged.

#### MockScope and `withMockScope`

Test isolation via scoped mocks. Each scope tags its mocks with a unique name and cleans them all up in a single DELETE
call.

```typescript
import { MockManager, MockScope, MockBuilder, withMockScope } from "ferrimock-mock-api";

const manager = new MockManager();

// Manual scope
const scope = new MockScope(manager, "test-login-flow");
await scope.add(
  MockBuilder.get("/api/me").respondWithJson(200, { id: "user-1" }),
  MockBuilder.post("/api/login").respondWith(200),
);
// ... run test ...
await scope.cleanup();  // removes all mocks in this scope

// Auto-cleanup with withMockScope (recommended)
await withMockScope(manager, "test-file-upload", async (scope) => {
  await scope.add(
    MockBuilder.post("/api/files").respondWithJson(201, { id: "file-1" }),
  );
  // ... test code ...
});
// scope.cleanup() called automatically, even if the test throws
```

#### Declarative Fixtures (`mockFixture`)

Reusable mock sets that can be installed/uninstalled across tests without rebuilding configs each time.

```typescript
import { MockManager, MockBuilder, mockFixture } from "ferrimock-mock-api";

// Define once, reuse everywhere
const userServiceMocks = mockFixture("user-service", [
  MockBuilder.get("/api/users").respondWithJson(200, { users: [] }),
  MockBuilder.get("/api/users/:id").respondWithJson(200, { id: "1", name: "Test" }),
  MockBuilder.post("/api/users").respondWith(201),
]);

const manager = new MockManager();

// In test setup:
await userServiceMocks.install(manager);

// In test teardown:
await userServiceMocks.uninstall();
```

#### Integration with Test Frameworks

```typescript
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { MockManager, MockBuilder, withMockScope, mockFixture } from "ferrimock-mock-api";

const manager = new MockManager({ defaultScope: "integration-tests" });

// Shared fixture for all tests
const authMocks = mockFixture("auth", [
  MockBuilder.get("/api/me").respondWithJson(200, { id: "user-1" }),
]);

beforeAll(async () => {
  await manager.waitForReady();
  await authMocks.install(manager);
});

afterAll(async () => {
  await authMocks.uninstall();
  await manager.teardown();
});

test("creates a user", async () => {
  await withMockScope(manager, "create-user", async (scope) => {
    await scope.add(
      MockBuilder.post("/api/users").respondWithJson(201, { id: "new-user" }),
    );

    const res = await fetch("http://localhost:3006/api/users", { method: "POST" });
    expect(res.status).toBe(201);
  });
});
```

## Proxy Integration

To use ferrimock with an existing reverse proxy, configure the proxy to forward requests to the ferrimock mock server
(default port 3006). Ferrimock can operate in several modes when integrated with a proxy:

| Mode               | Behavior                                      | Use Case                         |
| ------------------ | --------------------------------------------- | -------------------------------- |
| `hybrid` (default) | Try mock first, fallback to upstream          | Development with partial mocking |
| `selective`        | Only mock patterns matching `--mock-patterns` | Target specific endpoints        |
| `full`             | Mock only, 501 if no match                    | Offline development              |

```bash
ferrimock mock serve --mock-mode hybrid
ferrimock mock serve --mock-mode selective --mock-patterns "^/api/files,^/api/folders"
ferrimock mock serve --mock-mode full
ferrimock mock serve --no-passthrough  # Strict, no fallback
```

## File Organization

```
mocks/
├── collections/     # Auto-loaded (.yaml, .json, .har)
├── data/            # Referenced files
├── templates/       # Tera templates
└── recordings/      # Recorded sessions
```

Mock collections are loaded from the `collections/` directory. Files can be YAML, JSON, or HAR format. Subdirectories
are scanned recursively.

## Configuration

```bash
ferrimock mock serve --watch --log-matches
```

```yaml
mock:
  enabled: true
  collections_dir: "./mocks/collections"
  hot_reload: true
  log_matches: true
```

| Env Variable       | Description           |
| ------------------ | --------------------- |
| `MOCK`             | Enable mocks          |
| `MOCKS_DIR`        | Collections directory |
| `MOCK_LOG_MATCHES` | Log matched mocks     |

## Common Patterns

### REST CRUD

```yaml
mocks:
- id: list-users
  match:
    method: GET
    url: "/api/users"
  response:
    template: '{"users": [{% for i in range(start=0, end=5) %}{"id": {{ i }}}{% if not loop.last %},{% endif %}{% endfor %}]}'

- id: get-user
  match:
    url: "/api/users/:id"
  response:
    template: '{"id": "{{ captures.id }}", "name": "{{ fake_name() }}"}'
```

### Error Responses

```yaml
mocks:
- id: rate-limit
  match:
    url: "/api/limited"
  response:
    status: 429
    headers:
      retry-after: "60"
    body: '{"error": "rate_limit_exceeded"}'
```

### Stateful Counter

```yaml
mocks:
- id: request-counter
  match:
    url: "/api/stats"
  response:
    template: |
      {
        "request_count": {{ store_incr(key="api_requests") }},
        "session_id": "{{ store_get_or_set(key="session", default=fake_uuid()) }}"
      }
```

### Dynamic User Based on ID

```yaml
mocks:
- id: get-user-dynamic
  match:
    url: "/api/users/:id"
  response:
    template: |
      {
        "id": "{{ captures.id }}",
        "name": "{{ fake_name() }}",
        "email": "{{ fake_email() }}",
        "created_at": "{{ fake_iso_date() }}",
        "avatar": "{{ fake_avatar(initials=fake_first_name() | truncate(length=1, end='') ~ fake_last_name() | truncate(length=1, end='')) }}"
      }
```

## Interactive Mock Creation

Launch the wizard with `--interactive` or `-I`:

```bash
ferrimock mock create --interactive
ferrimock mock create -I
ferrimock mock create  # No URL triggers interactive mode
```

**Wizard steps:**

1. **Request Matching** - URL pattern (auto-detected), methods, header/query/body matchers
2. **Response Config** - Status code, content-type
3. **Response Body** - Smart template selection (user, list, item, create, update, error)
4. **Behavior** - Response delay
5. **Metadata** - Mock ID, priority, collection, output path
6. **Review** - Preview and confirm before saving

Template types auto-generate appropriate fake data:

| Template | Use Case                   |
| -------- | -------------------------- |
| Auto     | Detect from URL pattern    |
| User     | User/profile responses     |
| List     | Paginated list responses   |
| Item     | Single resource GET        |
| Create   | POST creation responses    |
| Update   | PUT/PATCH responses        |
| Delete   | DELETE responses           |
| Error    | Error responses (4xx, 5xx) |

## Standalone Mock Server

Run a lightweight mock server without any proxy overhead. Perfect for frontend development or CI/CD.

```bash
# Start mock server on default port (3006)
ferrimock mock serve

# With custom port and hot reload
ferrimock mock serve --port 3006 --watch

# With CORS for frontend development
ferrimock mock serve --cors --verbose

# From specific mock directory
ferrimock mock serve --mocks ./mocks/api/

# Enable template rendering endpoint
ferrimock mock serve --enable-render-endpoint
```

### Server Options

| Option                     | Description                          |
| -------------------------- | ------------------------------------ |
| `-p, --port <PORT>`        | Port to listen on (default: 3006)    |
| `--host <HOST>`            | Host to bind to (default: 127.0.0.1) |
| `-m, --mocks <DIR>`        | Mock collections directory           |
| `-w, --watch`              | Hot-reload on file changes           |
| `--cors`                   | Enable CORS headers                  |
| `--enable-render-endpoint` | Enable `/__mock/render` endpoint     |
| `-v, --verbose`            | Verbose request logging              |
| `-o, --open`               | Open browser to server URL           |

### Server Endpoints

| Endpoint         | Method | Description                  |
| ---------------- | ------ | ---------------------------- |
| `/*`             | ANY    | Mock matching (all paths)    |
| `/__mock/status` | GET    | Server status and info       |
| `/__mock/render` | POST   | Render template with context |
| `/__mock/list`   | GET    | List all loaded mocks        |

**Template rendering endpoint** (when enabled):

```bash
curl -X POST http://localhost:3006/__mock/render \
  -H "Content-Type: application/json" \
  -d '{"template": "{\"name\": \"{{ fake_name() }}\"}"}'
```

## CLI Commands

```bash
ferrimock mock create --interactive           # Step-by-step wizard
ferrimock mock create "/api/users/:id" -t    # Quick with template
ferrimock mock list -v
ferrimock mock test -m GET /api/users/123
ferrimock mock test -m GET /api/users/123 --render  # With response preview
ferrimock mock test -m GET /api/users/123 --debug   # Debug matching
ferrimock mock serve --watch                  # Standalone server
ferrimock mock validate
ferrimock mock format                          # Format all mocks
ferrimock mock format --check                  # Check without modifying
ferrimock mock reload
ferrimock mock convert traffic.har mocks.yaml
ferrimock mock consolidate large.json small.yaml
```

## Enhanced Mock Testing

Test mock matching with full request simulation:

```bash
# Basic matching test
ferrimock mock test -m GET /api/users/123

# With rendered response preview
ferrimock mock test -m GET /api/users/123 --render

# With headers
ferrimock mock test -m POST /api/users \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer token"

# With request body
ferrimock mock test -m POST /api/users --body '{"name": "John"}'

# Load body from file
ferrimock mock test -m POST /api/users --body @request.json

# Debug mode - shows why each mock matched or didn't match
ferrimock mock test -m GET /api/users/123 --debug
```

### Test Options

| Option         | Description                                   |
| -------------- | --------------------------------------------- |
| `-m, --method` | HTTP method (default: GET)                    |
| `-q, --query`  | Query string                                  |
| `-H, --header` | Request header (repeatable)                   |
| `-b, --body`   | Request body (JSON or @file)                  |
| `-r, --render` | Render and display mock response              |
| `-d, --debug`  | Show detailed matching analysis for all mocks |

## Formatting and Validation

### Formatting

Format mock files with consistent key ordering (id, match, response) and structure:

```bash
ferrimock mock format mocks/collections/          # Format all files
ferrimock mock format mocks/api.yaml              # Format single file
ferrimock mock format --check mocks/              # Check without modifying (CI)
```

Stdin mode for editor integrations (reads from stdin, writes formatted output to stdout):

```bash
cat mock.yaml | ferrimock mock format --stdin --file-format yaml
cat mock.json | ferrimock mock format --stdin --file-format json
```

### Validation

Validate mock configuration files for errors (invalid patterns, missing fields, template syntax):

```bash
ferrimock mock validate mocks/collections/        # Validate all files
ferrimock mock validate mocks/api.yaml            # Validate single file
ferrimock mock validate mocks/ --format json      # Machine-readable output
```

Stdin mode for editor integrations (validates buffer content without requiring a file on disk):

```bash
cat mock.yaml | ferrimock mock validate --stdin --file-format yaml --format json
```

## Debugging

```bash
RUST_LOG=debug ferrimock mock serve --log-matches
curl http://localhost:3006/__ferrimock/status
ferrimock mock test -m GET /api/users/123 --debug
```

### Common Issues

| Issue                  | Solution                                                                        |
| ---------------------- | ------------------------------------------------------------------------------- |
| Template not rendering | Check `{{ }}` braces, variable names                                            |
| Mock not matching      | Use inspector, check priority, verify URL pattern                               |
| Store values empty     | Check TTL, use unique keys                                                      |
| Patches not working    | `patch` is top-level (not under `response`), only applies to upstream responses |
| Delay not working      | `delay` is top-level (not under `response`), supports `ms`, `s`, `us` suffixes  |

## Tips

1. Use Express-style URLs (`/users/:id`) not regex
2. Always `| int` query params before math
3. Use `loop.last` to avoid trailing commas in JSON
4. Higher priority = more specific patterns
5. Set store TTLs to prevent memory leaks
6. Enable hot reload (`--watch`) during development

See `mocks/examples/` for complete patterns.
