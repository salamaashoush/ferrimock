#!/usr/bin/env node
// Measures one ferrimock checkout's interceptor cost, so two checkouts can be
// compared directly. Run with bun:
//
//   BENCH_ROOT=/path/to/checkout bun regression.mjs
//
// Prints JSON on stdout so the caller can diff two runs. Deliberately does not
// import MSW: comparing ferrimock against itself across commits is the signal,
// and loading a second interceptor into the process would perturb it.
import path from "node:path";

const ROOT = process.env.BENCH_ROOT ?? path.dirname(path.dirname(new URL(import.meta.url).pathname));
const ITERATIONS = Number(process.env.BENCH_ITERATIONS ?? 20_000);
const WARMUP = Number(process.env.BENCH_WARMUP_ITERATIONS ?? 3_000);
const REPEATS = Number(process.env.BENCH_REPEATS ?? 5);

const api = await import(path.join(ROOT, "packages", "core", "src", "index.ts"));
const { setupServer } = await import(path.join(ROOT, "packages", "core", "src", "node.ts"));
const { http, HttpResponse, graphql } = api;

const cases = [
  {
    id: "static",
    handlers: () => [http.get("http://bench.test/api/static", () => HttpResponse.json({ id: 1, name: "John Smith" }))],
    request: () => ["http://bench.test/api/static", undefined],
  },
  {
    id: "path-param",
    handlers: () => [
      http.get("http://bench.test/api/users/:id", ({ params }) => HttpResponse.json({ id: params.id })),
    ],
    request: () => ["http://bench.test/api/users/42", undefined],
  },
  {
    id: "fallthrough",
    handlers: () => [
      http.get("http://bench.test/api/deep", () => undefined),
      http.get("http://bench.test/api/deep", () => undefined),
      http.get("http://bench.test/api/deep", () => HttpResponse.json({ depth: 3 })),
    ],
    request: () => ["http://bench.test/api/deep", undefined],
  },
  {
    id: "graphql",
    handlers: () => [graphql.query("GetUser", ({ variables }) => HttpResponse.json({ data: { id: variables.id } }))],
    request: () => [
      "http://bench.test/graphql",
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ query: "query GetUser($id: ID!) { user(id: $id) { id } }", variables: { id: "42" } }),
      },
    ],
  },
];

const results = { root: ROOT, iterations: ITERATIONS, repeats: REPEATS, cases: {} };

for (const testCase of cases) {
  const server = setupServer(...testCase.handlers());
  server.listen({ onUnhandledRequest: "error" });
  const [url, init] = testCase.request();

  try {
    for (let i = 0; i < WARMUP; i += 1) {
      const res = await fetch(url, init);
      await res.arrayBuffer();
    }

    const samples = [];
    for (let repeat = 0; repeat < REPEATS; repeat += 1) {
      const started = process.hrtime.bigint();
      for (let i = 0; i < ITERATIONS; i += 1) {
        const res = await fetch(url, init);
        await res.arrayBuffer();
      }
      samples.push(Number(process.hrtime.bigint() - started) / ITERATIONS / 1000);
    }
    samples.sort((a, b) => a - b);
    results.cases[testCase.id] = {
      bestUs: Math.round(samples[0] * 100) / 100,
      medianUs: Math.round(samples[Math.floor(samples.length / 2)] * 100) / 100,
    };
  } finally {
    server.close();
  }
}

console.log(JSON.stringify(results, null, 2));
