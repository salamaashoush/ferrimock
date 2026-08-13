# Benchmarks

Cross-tool comparison of ferrimock against the mock servers it competes with, plus the
in-process comparison against MSW. Everything here is reproducible; the numbers in
`results/` come from running these scripts, not from any vendor's marketing page.

## Running

```bash
cd benchmarks
npm install                                   # Mockoon CLI, Prism, json-server, MSW
cargo build --release -p ferrimock-cli        # from the repo root
(cd baseline && CARGO_TARGET_DIR=../../target cargo build --release)

node servers.mjs                    # standalone mock servers, driven by `oha`
node ergonomics.mjs                 # config weight + capability matrix
node fair.mjs                       # ferrimock vs MSW, in-process, bun
BENCH_RUNTIME=node node fair.mjs    # same, under node
```

For the in-process comparison, build the native addon first:

```bash
cd crates/ferrimock-napi && bunx @napi-rs/cli build --platform --release
```

`oha` is the load generator (`brew install oha`). WireMock is opt-in because it needs
Docker: `BENCH_WIREMOCK=1 node servers.mjs`.

Knobs: `BENCH_DURATION` (default `5s`), `BENCH_CONNECTIONS` (`50`), `BENCH_WARMUP` (`1s`),
`BENCH_ONLY=ferrimock,mockoon` to restrict the run.

## Reading the results

**The throughput column has a ceiling.** `baseline/` is a bare axum handler returning a
constant string — the fastest anything answers on the host. On the development machine
these numbers were taken on, it tops out around 43k rps, and ferrimock sits within a few
percent of it. Above roughly 40k rps the benchmark is measuring the host's loopback and
the load generator, not the tool. Two `oha` processes against one server totalled only
~8% more than one, which confirms the ceiling is not the client alone.

So: throughput separates the Node-based tools from the native ones and says nothing about
headroom beyond that. **Single-connection p50 latency is the honest per-request figure** —
with one request in flight there is no queueing, so it isolates what the tool costs.

**Every tool serves the same seven endpoints** from its own idiomatic config in
`fixtures/`. Where a tool addresses an endpoint at a different path (json-server's
resource model), `paths` in `lib/tools.mjs` maps it. Where a tool structurally cannot
express a scenario, that is recorded as "not expressible" rather than as a failed
request against a URL it was never going to serve.

**Fairness notes.** Mockoon runs with `--disable-admin-api` and `--disable-log-to-file`;
Prism runs with `-v silent` and in its default static mode. It would be misleading to
benchmark a tool's console output rather than its serving.

Three harness bugs were found and fixed while building this, all of which made a
competitor look worse than it is. They are recorded here because the same traps are easy
to fall back into:

- Prism was passed `-d`, which is `--dynamic`, not `--document`. It answered 500 on every
  endpoint that had an example but no schema.
- json-server was probed at paths its resource model never serves. It now uses its own
  paths via `paths` in `lib/tools.mjs`, and the scenarios it structurally cannot express
  are labelled as such instead of counted as failures.
- Server stdout was piped into the harness. Tools that log a line per request — Mockoon
  does — were throttled by the benchmark draining that pipe. stdout now goes to
  `ignore`. Before the fix Mockoon measured ~320 rps; after it, ~2,700.
- **Both interceptors were loaded into one process.** Whichever loads second is
  penalised heavily: MSW measures 28.9us alone and 232.5us when it follows ferrimock,
  an 8x swing decided purely by test order. This is enough to manufacture any ratio
  you want in either direction, and it is exactly what produced the 2.5-3.4x figures
  that sat in the project README. `fair.mjs` now runs each library in its own process
  and alternates which goes first; `interceptor.mjs` and `node-vs-bun.mjs` are retired
  stubs that refuse to run.

A 2xx no longer counts as support on its own: scenarios carry an `expect` predicate, so a
tool that answers a header-conditional route with the same canned example regardless of
the header is recorded as not supporting it.

## What is measured where

| Script | Measures |
| --- | --- |
| `servers.mjs` | rps, p50/p95/p99 under load, single-connection p50, startup to first response, RSS idle and under load |
| `fair.mjs` | per-request cost of a mocked `fetch()`, ferrimock vs MSW, one library per process |
| `isolated.mjs` | one library, one scenario, one process — the unit `fair.mjs` spawns |
| `ergonomics.mjs` | lines of config for the same API, probed scenario support, declared capabilities with sources |

The declared-capability rows in `ergonomics.mjs` are the only hand-maintained claims, and
each carries a source note. Recheck them when a tool ships a new major version.
