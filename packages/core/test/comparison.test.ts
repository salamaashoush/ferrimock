import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { MockpitServer, http, HttpResponse, fake } from "@mockpit/node";
import { setupServer } from "msw/node";
import { http as mswHttp, HttpResponse as mswHttpResponse } from "msw";
import { faker } from "@faker-js/faker";

const N = 2000;
const WARMUP = 100;

function bench(label: string, rps: number, usPerReq: number) {
  console.log(`  ${label.padEnd(45)} ${rps.toFixed(0).padStart(7)} req/s  (${usPerReq.toFixed(0)}us/req)`);
}

// ===== MSW Benchmarks =====

describe("MSW vs Mockpit comparison", () => {
  // -- MSW: static JSON --
  it("MSW - static JSON handler", async () => {
    const server = setupServer(
      mswHttp.get("http://127.0.0.1:9999/api/bench", () => {
        return mswHttpResponse.json({ id: "123", name: "John", source: "msw" });
      })
    );
    server.listen({ onUnhandledRequest: "bypass" });

    // MSW intercepts fetch, so we need a real target to intercept
    // MSW doesn't actually start a server -- it intercepts at the network level
    // We need to measure just the interception overhead

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await fetch("http://127.0.0.1:9999/api/bench");
    }
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("MSW: static JSON (intercept)", rps, usPerReq);

    server.close();
  });

  // -- MSW: handler with faker.js --
  it("MSW - handler with faker.js", async () => {
    const server = setupServer(
      mswHttp.get("http://127.0.0.1:9999/api/bench", () => {
        return mswHttpResponse.json({
          id: faker.string.uuid(),
          name: faker.person.fullName(),
          email: faker.internet.email(),
          source: "msw+faker",
        });
      })
    );
    server.listen({ onUnhandledRequest: "bypass" });

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await fetch("http://127.0.0.1:9999/api/bench");
    }
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("MSW: handler + faker.js", rps, usPerReq);

    server.close();
  });

  // -- MSW: handler with params --
  it("MSW - handler with path params", async () => {
    const server = setupServer(
      mswHttp.get("http://127.0.0.1:9999/api/users/:id", ({ params }) => {
        return mswHttpResponse.json({ id: params.id, name: "John" });
      })
    );
    server.listen({ onUnhandledRequest: "bypass" });

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await fetch("http://127.0.0.1:9999/api/users/42");
    }
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("MSW: handler with :params", rps, usPerReq);

    server.close();
  });

  // -- Mockpit: static declarative --
  it("Mockpit - static declarative mock", async () => {
    const server = new MockpitServer();
    await server.addMock({
      id: "bench-static",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"id":"123","name":"John","source":"mockpit-static"}',
      },
    });
    const url = await server.listen();
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/api/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("Mockpit: static declarative", rps, usPerReq);

    await server.close();
  });

  // -- Mockpit: Tera template with fake data --
  it("Mockpit - declarative template with fake data", async () => {
    const server = new MockpitServer();
    await server.addMock({
      id: "bench-template",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        template:
          '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}","email":"{{ fake_email() }}","source":"mockpit-template"}',
      },
    });
    const url = await server.listen();
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/api/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("Mockpit: template + Rust fake data", rps, usPerReq);

    await server.close();
  });

  // -- Mockpit: JS handler static --
  it("Mockpit - JS handler static response", async () => {
    const server = new MockpitServer();
    server.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({ id: "123", name: "John", source: "mockpit-handler" })
      ),
    ]);
    const url = await server.listen();
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/api/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("Mockpit: JS handler (static)", rps, usPerReq);

    await server.close();
  });

  // -- Mockpit: JS handler with fake namespace --
  it("Mockpit - JS handler with fake.*", async () => {
    const server = new MockpitServer();
    server.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({
          id: fake.uuid(),
          name: fake.name(),
          email: fake.email(),
          source: "mockpit-handler+fake",
        })
      ),
    ]);
    const url = await server.listen();
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/api/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("Mockpit: JS handler + fake.* (NAPI)", rps, usPerReq);

    await server.close();
  });

  // -- Mockpit: JS handler with faker.js (pure JS) --
  it("Mockpit - JS handler with faker.js", async () => {
    const server = new MockpitServer();
    server.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({
          id: faker.string.uuid(),
          name: faker.person.fullName(),
          email: faker.internet.email(),
          source: "mockpit-handler+fakerjs",
        })
      ),
    ]);
    const url = await server.listen();
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/api/bench`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("Mockpit: JS handler + faker.js (pure JS)", rps, usPerReq);

    await server.close();
  });

  // -- Mockpit: JS handler with params --
  it("Mockpit - JS handler with :params", async () => {
    const server = new MockpitServer();
    server.useHandlers([
      http.get("/api/users/:id", async (req) =>
        HttpResponse.json({ id: req.params.id, name: "John" })
      ),
    ]);
    const url = await server.listen();
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/users/42`);

    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch(`${url}/api/users/42`);
    const elapsed = performance.now() - start;
    const rps = (N / elapsed) * 1000;
    const usPerReq = (elapsed / N) * 1000;
    bench("Mockpit: JS handler with :params", rps, usPerReq);

    await server.close();
  });

  // -- Summary --
  it("prints summary", () => {
    console.log("\n  === Summary ===");
    console.log("  MSW intercepts fetch at the network level (no real HTTP).");
    console.log("  Mockpit runs a real HTTP server (axum) with actual TCP connections.");
    console.log("  Mockpit declarative mocks are pure Rust (no JS overhead).");
    console.log("  Mockpit templates use Tera engine in Rust (no JS overhead).");
    console.log("  fake.* uses Rust generators via NAPI (~1us per call).");
    console.log("  faker.js is pure JavaScript.");
  });
});
