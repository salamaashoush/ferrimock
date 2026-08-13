// Measures exactly ONE library in this process. Loading two interceptors into
// the same process contaminates the second one, so cross-library numbers must
// come from separate processes.
import path from "node:path";
const REPO = path.dirname(path.dirname(new URL(import.meta.url).pathname));
const LIB = process.env.BENCH_LIB;
const SCENARIO = process.env.BENCH_SCENARIO ?? "static";
const N = Number(process.env.BENCH_N ?? 2000);
const WARMUP = Number(process.env.BENCH_WARMUP ?? 500);
const REPEATS = Number(process.env.BENCH_REPEATS ?? 3);

let setupServer, http, HttpResponse, gen;
const underBun = typeof Bun !== "undefined";

if (LIB === "ferrimock") {
  // Node cannot import the TypeScript entry points; it gets the built dist,
  // which is also what a published consumer loads.
  const entry = underBun
    ? path.join(REPO, "packages", "core", "src", "index.ts")
    : path.join(REPO, "packages", "core", "dist", "index.mjs");
  const core = await import(entry);
  ({ setupServer, http, HttpResponse } = core);
  // The napi package is a workspace member and is not resolvable from here.
  const { fake } = await import(path.join(REPO, "crates", "ferrimock-napi", "index.js"));
  gen = () => ({ id: fake.uuid(), name: fake.name(), email: fake.email() });
} else {
  const msw = await import("msw");
  ({ http, HttpResponse } = msw);
  ({ setupServer } = await import("msw/node"));
  const { faker } = await import("@faker-js/faker");
  gen = () => ({ id: faker.string.uuid(), name: faker.person.fullName(), email: faker.internet.email() });
}

const handlers = {
  static: [http.get("http://bench.test/api/bench", () => HttpResponse.json({ id: "123", name: "John" }))],
  params: [http.get("http://bench.test/api/users/:id", ({ params }) => HttpResponse.json({ id: params.id }))],
  fake: [http.get("http://bench.test/api/bench", () => HttpResponse.json(gen()))],
};
const urls = {
  static: "http://bench.test/api/bench",
  params: "http://bench.test/api/users/42",
  fake: "http://bench.test/api/bench",
};

const server = setupServer(...handlers[SCENARIO]);
server.listen({ onUnhandledRequest: "error" });
const url = urls[SCENARIO];

for (let i = 0; i < WARMUP; i++) await (await fetch(url)).arrayBuffer();
let best = Infinity;
for (let r = 0; r < REPEATS; r++) {
  const t0 = performance.now();
  for (let i = 0; i < N; i++) await (await fetch(url)).arrayBuffer();
  const us = ((performance.now() - t0) / N) * 1000;
  if (us < best) best = us;
}
server.close();
console.log(JSON.stringify({ lib: LIB, scenario: SCENARIO, usPerReq: +best.toFixed(2) }));
