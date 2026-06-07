// Fair mockpit-vs-MSW benchmark.
//
// Methodology (transparent on purpose):
//   - Runs under Node (MSW's target runtime), using Node's global fetch (undici).
//   - One interceptor active at a time (mockpit applied + disposed, then MSW).
//   - Identical routes + response payloads for both libraries.
//   - WARMUP iterations (JIT), then SAMPLES batches of BATCH sequential awaits;
//     report the MEDIAN per-op time (robust to GC/noise) plus min/max spread.
//   - Handler scenarios run a JS callback in BOTH libs (apples-to-apples).
//   - The "static (declarative)" row is mockpit's Rust fast path (no JS handler);
//     MSW always runs a JS handler, so that row is labeled as such.
//
// Run:  node packages/core/bench/vs-msw.mjs   (from repo root)

import { MockpitInterceptor } from "@mockpit/core";
import { http as mock, MockResponse } from "@mockpit/node";
import { setupServer } from "msw/node";
import { http as mswHttp, HttpResponse } from "msw";
import { faker } from "@faker-js/faker";

const WARMUP = 5000;
const SAMPLES = 25;
const BATCH = 1000;

async function bench(fn) {
  for (let i = 0; i < WARMUP; i++) await fn();
  const perOp = [];
  for (let s = 0; s < SAMPLES; s++) {
    const t0 = performance.now();
    for (let i = 0; i < BATCH; i++) await fn();
    perOp.push((performance.now() - t0) / BATCH); // ms/op
  }
  perOp.sort((a, b) => a - b);
  const median = perOp[perOp.length >> 1];
  return {
    ops: Math.round(1000 / median),
    us: median * 1000,
    minUs: perOp[0] * 1000,
    maxUs: perOp[perOp.length - 1] * 1000,
  };
}

// Mocked routes are intercepted in-process, so the host never needs to exist.
const BASE = "http://localhost";

// Scenarios: each registers the same behavior in both libs.
const scenarios = {
  "static JSON (mockpit: declarative fast path)": {
    mockpit: (ic) => ic.addMock({
      id: "s", match: { method: "GET", url: "/static" },
      response: { status: 200, headers: { "content-type": "application/json" }, body: '{"ok":true}' },
    }),
    msw: () => mswHttp.get(`${BASE}/static`, () => HttpResponse.json({ ok: true })),
    call: () => fetch(`${BASE}/static`),
    check: (j) => j.ok === true,
  },
  "GET handler + path param (JS handler both)": {
    mockpit: (ic) => ic.useHandlers([mock.get("/users/:id", async (req) => MockResponse.json({ id: req.param("id") }))]),
    msw: () => mswHttp.get(`${BASE}/users/:id`, ({ params }) => HttpResponse.json({ id: params.id })),
    call: () => fetch(`${BASE}/users/42`),
    check: (j) => j.id === "42",
  },
  "POST + JSON body echo (JS handler both)": {
    mockpit: (ic) => ic.useHandlers([mock.post("/echo", async (req) => MockResponse.json({ got: req.bodyJson?.name }))]),
    msw: () => mswHttp.post(`${BASE}/echo`, async ({ request }) => HttpResponse.json({ got: (await request.json()).name })),
    call: () => fetch(`${BASE}/echo`, { method: "POST", headers: { "content-type": "application/json" }, body: '{"name":"ada"}' }),
    check: (j) => j.got === "ada",
  },
  "dynamic fake data (mockpit template vs MSW+faker)": {
    mockpit: (ic) => ic.addMock({
      id: "d", match: { method: "GET", url: "/fake" },
      response: { status: 200, headers: { "content-type": "application/json" }, template: '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}"}' },
    }),
    msw: () => mswHttp.get(`${BASE}/fake`, () => HttpResponse.json({ id: faker.string.uuid(), name: faker.person.fullName() })),
    call: () => fetch(`${BASE}/fake`),
    check: (j) => typeof j.id === "string" && j.name.length > 0,
  },
};

function fmt(r) {
  return `${r.ops.toString().padStart(8)} ops/s  ${r.us.toFixed(2).padStart(7)}us  (min ${r.minUs.toFixed(2)} / max ${r.maxUs.toFixed(2)})`;
}

console.log(`\nNode ${process.version} — WARMUP=${WARMUP}, SAMPLES=${SAMPLES}, BATCH=${BATCH}, median per-op\n`);

const results = {};

// ---- mockpit ----
{
  const ic = new MockpitInterceptor();
  for (const sc of Object.values(scenarios)) {
    await sc.mockpit(ic);
  }
  ic.apply({ onUnhandledRequest: "bypass" });
  for (const [name, sc] of Object.entries(scenarios)) {
    const j = await (await sc.call()).json();
    if (!sc.check(j)) throw new Error(`mockpit did not mock "${name}": ${JSON.stringify(j)}`);
  }
  console.log("mockpit: all scenarios return correct mocked data ✓");
  results.mockpit = {};
  for (const [name, sc] of Object.entries(scenarios)) {
    results.mockpit[name] = await bench(sc.call);
  }
  ic.dispose();
}

// ---- MSW ----
{
  const handlers = Object.entries(scenarios)
    .map(([, sc]) => sc.msw())
    .filter(Boolean);
  const server = setupServer(...handlers);
  server.listen({ onUnhandledRequest: "bypass" });
  for (const [name, sc] of Object.entries(scenarios)) {
    const j = await (await sc.call()).json();
    if (!sc.check(j)) throw new Error(`MSW did not mock "${name}": ${JSON.stringify(j)}`);
  }
  console.log("msw:     all scenarios return correct mocked data ✓\n");
  results.msw = {};
  for (const [name, sc] of Object.entries(scenarios)) {
    results.msw[name] = await bench(sc.call);
  }
  server.close();
}

console.log("scenario".padEnd(48) + " | mockpit / MSW (speedup)\n" + "-".repeat(110));
for (const name of Object.keys(scenarios)) {
  const m = results.mockpit[name];
  const w = results.msw[name];
  const speed = (m.ops / w.ops).toFixed(2);
  console.log(name.padEnd(48));
  console.log("  mockpit:  " + fmt(m));
  console.log("  msw:      " + fmt(w));
  console.log(`  => mockpit ${speed}x MSW\n`);
}
