# Research Prompt: Optimize NAPI ThreadsafeFunction Overhead for mockpit

## Context

mockpit is a Rust-powered HTTP mocking engine exposed to Node.js via napi-rs. We have a fetch interceptor that patches `globalThis.fetch` and routes requests through the Rust mock engine via NAPI.

**Current performance (isolated, no MSW interference):**
- Declarative mocks (pure Rust matching + response gen): **12us/req** -- 2.5x faster than MSW
- JS handler mocks (Rust matching + TSFN → JS handler → TSFN → Rust): **28us/req** -- parity with MSW
- The **22us TSFN round-trip** is the bottleneck for JS handlers

**The flow for JS handler mocks:**
```
fetch() intercepted in JS
  → await matchRequest() -- async NAPI call to Rust
    → Rust MockMatcher finds the handler mock (BodySource::Handler)
    → Rust calls HandlerFn which is a ThreadsafeFunction
      → TSFN queues callback to Node.js event loop (~5us)
      → Node.js picks up callback, runs JS handler (~10us)
      → Promise resolves, result sent back to Rust (~7us)
    → Rust builds response
  → NAPI returns result to JS
→ new Response() built from result
```

**Goal:** Reduce the 22us TSFN overhead to make JS handlers significantly faster than MSW (target: <15us total for JS handler, making us 2x+ faster than MSW's 39us).

## What to Research

### 1. Oxc/Oxlint JS Plugin Architecture
Clone https://github.com/oxc-project/oxc and study:
- How they handle JS plugins called from Rust at high frequency
- Their "raw transfer" pattern that shares Rust's native memory layout with JS
- Lazy AST deserialization approach
- How they avoid serialization overhead
- Look at `crates/oxc_linter_plugin/`, `napi/` directories
- Read their blog posts: https://oxc.rs/blog/2025-10-09-oxlint-js-plugins.html and https://oxc.rs/blog/2026-03-11-oxlint-js-plugins-alpha

### 2. Encore.ts Rust Runtime
Research https://github.com/encoredev/encore and their blog https://encore.dev/blog/rust-runtime:
- They forked napi-rs ThreadSafeFunction to support return values from JS callbacks
- They detect Promises and chain `.then()` back to tokio channels
- They achieved 9x Express.js throughput
- How do they minimize the Rust↔JS crossing overhead?

### 3. napi-rs Internals
Clone https://github.com/napi-rs/napi-rs and study:
- How `ThreadsafeFunction.call_async()` works internally
- The difference between `call()`, `call_async()`, and `call_with_return_value()`
- How the UV event loop wakeup works
- Can we use `FunctionRef` + `spawn_future_with_callback` instead of TSFN for the interceptor case (we're already on the JS thread)?
- The `callee_handled` parameter -- what does false vs true actually do?
- napi-rs 3 Function API: https://napi.rs/blog/function-and-callbacks
- Look at `crates/napi/src/threadsafe_function.rs`

### 4. Alternative: Avoid TSFN Entirely for Interceptor Mode
The key insight: in interceptor mode, `matchRequest()` is called FROM the JS thread. The async NAPI function runs the Rust code on the tokio runtime, but when it hits a JS handler, it uses TSFN to call back to JS. But we're already ON the JS thread!

Research if there's a way to:
- Detect that we're on the JS main thread and call the function directly
- Use `FunctionRef` (lightweight, main-thread only) instead of `ThreadsafeFunction`
- Use napi's `Env` to call functions synchronously when on the main thread
- Keep a JS-side function reference that Rust can invoke without TSFN

### 5. Rspack's Approach
Clone https://github.com/nicolo-ribaudo/rspack (or official repo) and study:
- How they handle webpack plugin hooks called from Rust
- Their `JsCompiler` wrapper pattern
- How they handle the high-frequency callback case (loader callbacks, plugin hooks)
- Any custom ThreadsafeFunction optimizations

### 6. Potential Optimizations to Implement

**a) FunctionRef for same-thread calls:**
When `matchRequest()` is called from JS (interceptor mode), the handler could be called via `FunctionRef` (same-thread, no queue) instead of `ThreadsafeFunction` (cross-thread, queue + wakeup).

**b) Sync NAPI path for handlers:**
If the handler is synchronous (returns a value, not a Promise), we could use a sync NAPI call path instead of async. Most handlers are `() => MockResponse.json(...)` -- they don't actually need async.

**c) Pre-resolved handlers:**
At registration time, if a handler is a pure function that always returns the same response (detected via a single trial call or explicit opt-in), convert it to `BodySource::Inline` and skip the TSFN entirely.

**d) Batched TSFN calls:**
Instead of one TSFN call per request, batch multiple pending requests and call JS once with an array. JS processes all of them and returns an array of responses.

**e) SharedArrayBuffer / Zero-copy:**
Pass request data via SharedArrayBuffer instead of creating napi objects. JS reads directly from shared memory. Similar to oxc's "raw transfer" pattern.

**f) Custom TSFN fork (Encore.ts approach):**
Fork napi-rs's ThreadsafeFunction to add return value support without the Promise wrapper overhead.

## Current Code to Study

- Handler bridge: `crates/mockpit-napi/src/handler_bridge.rs`
- Request context: `crates/mockpit-napi/src/request_context.rs`
- matchRequest: `crates/mockpit-napi/src/server.rs` (search for `match_request`)
- Core HandlerFn type: `crates/mockpit/src/types/mod.rs` (search for `HandlerFn`)
- Interceptor: `packages/core/src/interceptor.ts`
- Profile test: `packages/core/test/profile-interceptor.test.ts`

## Benchmark to Beat

```
Current:
  matchRequest() declarative:  12.5us
  matchRequest() JS handler:   34.9us  (22us TSFN overhead)
  MSW full flow:               39us

Target:
  matchRequest() JS handler:   <15us   (eliminate or minimize TSFN)
  Full interceptor JS handler: <20us   (2x faster than MSW)
```

## Key Questions to Answer

1. Can we avoid TSFN entirely when `matchRequest()` is called from the JS thread?
2. What does Oxc's "raw transfer" pattern actually do and can we use it?
3. How did Encore.ts fork TSFN to get return values without Promise overhead?
4. Is there a sync path through napi-rs for calling JS functions from Rust when we know we're on the main thread?
5. Can we detect at registration time which handlers are pure/static and pre-compute their responses?
