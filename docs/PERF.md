# Mockpit performance

Release build, bun 1.4, linux-x64. req/s (higher better) / µs/req (lower better).
Reproduce:
- `cd crates/mockpit-napi && bun test test/profile.test.ts` (Rust server path)
- `cd packages/core && bun test test/profile-interceptor.test.ts` (interceptor path vs MSW)
- `cargo bench --bench mock_performance` (in-process matcher microbenchmarks)

## Interceptor throughput vs MSW

| scenario                            | before | after  | MSW   |
|-------------------------------------|--------|--------|-------|
| matchRequest (declarative)          | 122357 | 173867 | —     |
| matchRequest (JS handler)           | 90662  | 115370 | —     |
| full flow (declarative)             | 93503  | ~131k  | 25–27k|
| full flow (JS handler)              | 66279  | ~99k   | 25–27k|

Declarative full-flow ≈ **4.8× MSW**, JS handler ≈ **3.6× MSW** (was 3.7× / 2.6×).
The interceptor numbers are run-to-run noisy (±25%); the matcher microbenchmarks below are stable.

## Fair benchmark vs MSW (Node)

`packages/core/bench/vs-msw.mjs` — run `bun run --filter @mockpit/core bench:msw` (or
`node packages/core/bench/vs-msw.mjs`). Runs under **Node** (MSW's target), one interceptor active
at a time, identical routes/payloads, both verified to return the correct mocked data before timing.
Warmup + median-of-25 batches.

| scenario                              | mockpit     | MSW        | speedup |
|---------------------------------------|-------------|------------|---------|
| static JSON (mockpit declarative)     | ~69k ops/s  | ~6.6k ops/s| ~10×    |
| GET handler + path param (JS both)    | ~44k        | ~2.6k      | ~16×    |
| POST + JSON body (JS both)            | ~26k        | ~1.5k      | ~17×    |
| dynamic fake (mockpit tpl vs MSW+faker)| ~63k       | ~1k        | ~60×    |

**Why the gap, and caveats (read these):**
- mockpit's `fetch` path patches `globalThis.fetch` and matches in Rust — it short-circuits *before*
  the request descends into undici. MSW intercepts at the undici/request layer for universal coverage
  (also catches `http`/axios). So the gap is largest on the `fetch` path; for `http.request` mockpit
  uses the same `@mswjs/interceptors` and the advantage narrows to Rust matching.
- Micro-benchmark: sequential, body not consumed (equal for both). A real test suite's network/render/
  assertion time dwarfs interception, so end-to-end suite speedup is far smaller than these ratios.
- The fake-data row is dominated by faker.js being slow vs Rust fake generators — realistic
  (MSW users reach for faker) but not an interception-cost comparison.

## Optimizations

### Reuse one matcher per server (interceptor hot path)
`MockMatcher` was rebuilt per request (`MockMatcher::new(registry.clone())`), allocating a fresh
1000-entry LRU and never warming it. Now one matcher is stored on `MockpitServer` and reused via a
cheap Arc clone; its cache is cleared on every mock mutation. **+41% declarative, +50% JS handler.**

### Share the enabled-mocks cache by Arc, not by Vec clone
`SortedMocksCache` holds `Arc<Vec<Arc<MockDefinition>>>`; the slow-path scan and index rebuild clone
a refcount instead of the whole Vec.

### One exact-index check per request + single-flight rebuild
Folded `has_conditional_mocks()` + `try_exact_match()` into a single `ensure_exact_index()` call,
and guarded rebuilds with a double-checked mutex so a stale index under concurrent load is rebuilt
once, not by every request. Criterion: complex-match slow path −5%, fast path −3%, concurrent −8%.

### Parse the request body JSON once per request
The body was parsed by every graphql + JSON-body matcher, re-run per candidate during the linear
scan (O(candidates) parses). It is now parsed once in `find_match` and shared via `Option<&Value>`.

### Lazy request context for templates
`ResponseGenerator` records at load time whether a template references `headers`/`body`; the
per-request context then skips building the header map and parsing the body when unused. Building a
context for a 10-header request that the template ignores: **673 ns → 33 ns**. Declarative template
HTTP throughput ≈ 65k req/s (vs 69k static).

### Lean default build
`axum`/`tower-http` are optional, pulled only by the `server` feature; the default `engine` build
links **zero axum crates** (`cargo tree`-verified), so embedders that only need request matching
don't compile the HTTP stack.

## Correctness & API changes (relevant to perf)

- **Binary-safe bodies.** Response bodies crossed the NAPI boundary as `String`
  (`from_utf8_lossy`), corrupting binary responses. They now cross as `Uint8Array` end-to-end,
  which also removes the per-response UTF-8 validate/encode.
- **Interceptor passthrough.** Unmatched requests re-send the constructed `Request` (fixing a
  consumed-stream bug) and only buffer the body when a registered mock can match on it; zero-mock
  requests skip all match work. Hot-path flags are cached JS-side (no per-request NAPI getter).
- **Node `http`/`https` interception** via `@mswjs/interceptors` (axios/got/node-fetch v2), routed
  through the same Rust matcher; global `fetch` and XHR keep the hand-rolled fast paths.
- **AbortSignal / redirects / XHR** handled on the fetch path; XHR honors `responseType`.
- **Typed errors.** The library no longer depends on `anyhow` or returns `Result<_, String>`; all
  fallible APIs return a `thiserror`-based `MockpitError` (with a `Context` extension and `mp_err!`/
  `mp_bail!` macros). Consumers can match on failure modes; the CLI keeps `anyhow` and `?`-coerces.
- **CLI `--quiet` + config.** `--quiet` suppresses decorative output (banners/progress/status) via a
  `say!` macro while preserving data output (fake values, JSON, results) and errors. `mockpit.toml`
  is honored: `collections_dir`/`port`/`host` resolve through `config::*` helpers (env > config file
  > default) instead of being parsed and discarded.
- **No duplicate implementations.** Four behaviors that the CLI had reimplemented are now
  single-sourced in `services::*`, with the CLI (and NAPI) delegating:
  - mock-response building → `services::serve::respond` (NAPI `listen` + CLI),
  - mock generation → `services::create` (CLI `create` + interactive wizard),
  - fake-data generation → `services::fake_data::generate_single` (superset alias table),
  - mock-file formatting → `services::format` (domain-aware key ordering + body expansion).

  The response id header is `X-Mock-Id` everywhere. Also fixed: the `store` template tests raced on
  the shared global store under parallel `cargo test` — now `#[serial]`, so `cargo test` is green.
