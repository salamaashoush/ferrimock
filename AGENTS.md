# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Mockpit is a high-performance HTTP mocking engine for Node.js, powered by Rust via NAPI. It provides an MSW-compatible API that is 3-4x faster than MSW, plus declarative YAML/JSON mocks with Tera template rendering and 115+ fake data generators.

## Workspace Structure

Monorepo with Cargo workspace (3 Rust crates) + bun workspaces (3 JS packages).

### Rust Crates

**mockpit** (library) -- Core mock engine:
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

**mockpit-napi** (cdylib) -- Node.js NAPI bindings:
- `http_ns.rs` - `http.get/post/put/delete/patch/head/options/all` with RegExp support
- `graphql_ns.rs` - `graphql.query/mutation/operation`
- `response_ns.rs` - `MockResponse.json/text/html/xml/arrayBuffer/empty/error`
- `handler_bridge.rs` - HandlerFn (TSFN for server) + FunctionRef (direct call for interceptor)
- `request_context.rs` - MockpitRequest with lazy getters (params, headers, cookies, body)
- `server.rs` - MockpitServer with FunctionRef-optimized matchRequest, use/resetHandlers/listHandlers
- `fake_ns.rs` - 115+ fake data generators exposed to JS

**mockpit-cli** (binary) -- CLI for mock management and fake data generation.

### JavaScript Packages

**@mockpit/core** -- Main user-facing package:
- `interceptor.ts` - MockpitInterceptor (patches fetch/XHR), lifecycle events, boundary, onUnhandledRequest
- `msw-compat.ts` - delay(), passthrough(), bypass() utilities
- `events.ts` - LifecycleEvents emitter (request:start/match/unhandled/end, response:mocked/bypass)
- `graphql-link.ts` - URL-scoped GraphQL handlers (graphqlLink)
- `config.ts` / `loader.ts` - Config loading

**@mockpit/cli** -- CLI wrapper (delegates to Rust binary).

**@mockpit/playwright** -- Playwright fixture adapter.

## Essential Commands

```bash
# Rust
cargo check --workspace                          # Fast compile check
cargo test -p mockpit --lib                       # Run Rust tests (607 tests)
cargo check -p mockpit-napi                       # Check NAPI bindings

# Build native module
cd crates/mockpit-napi && bunx @napi-rs/cli build --platform --release

# JavaScript tests
bun test ./packages/core/test/                    # All JS tests
bun test ./packages/core/test/msw-compat.test.ts  # MSW compatibility tests
bun test ./packages/core/test/interceptor.test.ts # Interceptor + benchmarks
bun test ./crates/mockpit-napi/test/              # NAPI binding tests
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

### MSW API Compatibility

Implemented:
- `http.get/post/put/delete/patch/head/options/all` with string and RegExp paths
- `graphql.query/mutation/operation` + `graphqlLink(url)`
- `MockResponse.json/text/html/xml/arrayBuffer/empty/error`
- `delay()`, `passthrough()`, `bypass()`
- Request context: params, headers, cookies, body, bodyJson, query, requestId
- `server.use()`, `resetHandlers()`, `restoreHandlers()`, `listHandlers()`
- `server.boundary()`, lifecycle events, `onUnhandledRequest` strategies
- One-time handlers (`once: true`)

## Code Standards

- Idiomatic Rust with zero-cost abstractions
- `anyhow::Result` for application code
- `unsafe` denied in mockpit-napi (except marked `#[allow(unsafe_code)]` for NAPI FFI)
- FxHashMap for performance-critical paths (not std HashMap)
- All new code must include tests
- Run `cargo test -p mockpit --lib` and `bun test` before committing
