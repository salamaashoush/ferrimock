import { describe, it, expect } from "bun:test";
import { FerrimockServer, http, HttpResponse, fake } from "../index.js";

const N = 2000;

describe("overhead profiling", () => {
  // Baseline: just measure fetch overhead (no mock server involved)
  it("baseline: raw HTTP fetch to a minimal Bun server", async () => {
    const bunServer = Bun.serve({
      port: 0,
      fetch() {
        return new Response('{"ok":true}', {
          headers: { "content-type": "application/json" },
        });
      },
    });

    const url = `http://127.0.0.1:${bunServer.port}`;
    for (let i = 0; i < 100; i++) await fetch(`${url}/warmup`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    console.log(`\n  Bun server baseline:      ${rps.toFixed(0)} req/s  (${usPerReq.toFixed(0)}us/req)`);
    bunServer.stop();
  });

  // Declarative static mock
  it("declarative static mock", async () => {
    const server = new FerrimockServer();
    await server.addMock({
      id: "static",
      match: { method: "GET", url: "/bench" },
      response: { status: 200, body: '{"ok":true}' },
    });
    const url = await server.listen();
    for (let i = 0; i < 100; i++) await fetch(`${url}/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    console.log(`  Declarative static:       ${rps.toFixed(0)} req/s  (${usPerReq.toFixed(0)}us/req)`);
    await server.close();
  });

  // JS handler - minimal (no context access)
  it("JS handler - minimal", async () => {
    const server = new FerrimockServer();
    server.useHandlers([
      http.get("/bench", async () => HttpResponse.json({ ok: true })),
    ]);
    const url = await server.listen();
    for (let i = 0; i < 100; i++) await fetch(`${url}/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    console.log(`  JS handler (minimal):     ${rps.toFixed(0)} req/s  (${usPerReq.toFixed(0)}us/req)`);
    await server.close();
  });

  // JS handler - accesses params
  it("JS handler - with params", async () => {
    const server = new FerrimockServer();
    server.useHandlers([
      http.get("/bench/:id", async ({ params }) =>
        HttpResponse.json({ id: params.id })
      ),
    ]);
    const url = await server.listen();
    for (let i = 0; i < 100; i++) await fetch(`${url}/bench/123`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/bench/123`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    console.log(`  JS handler (params):      ${rps.toFixed(0)} req/s  (${usPerReq.toFixed(0)}us/req)`);
    await server.close();
  });

  // JS handler - with fake data
  it("JS handler - with fake data", async () => {
    const server = new FerrimockServer();
    server.useHandlers([
      http.get("/bench", async () =>
        HttpResponse.json({ id: fake.uuid(), name: fake.name(), email: fake.email() })
      ),
    ]);
    const url = await server.listen();
    for (let i = 0; i < 100; i++) await fetch(`${url}/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    console.log(`  JS handler (fake data):   ${rps.toFixed(0)} req/s  (${usPerReq.toFixed(0)}us/req)`);
    await server.close();
  });

  // Measure just the TSFN overhead (no HTTP)
  it("pure TSFN call overhead (no HTTP)", async () => {
    // We can't isolate TSFN from this level, but we can compare
    // the handler overhead vs baseline to estimate it
    console.log(`\n  === Overhead analysis ===`);
    console.log(`  The difference between "Declarative static" and "JS handler (minimal)"`);
    console.log(`  is the pure NAPI ThreadsafeFunction bridge overhead.`);
  });
});
