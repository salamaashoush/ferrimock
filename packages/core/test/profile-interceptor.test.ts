import { describe, it } from "bun:test";
import { MockpitInterceptor } from "../src/interceptor.js";
import { http, MockResponse, fake } from "@mockpit/node";
import { setupServer } from "msw/node";
import { http as mswHttp, HttpResponse } from "msw";
import { faker } from "@faker-js/faker";

const N = 3000;

function bench(label: string, elapsed: number) {
  const rps = (N / elapsed) * 1000;
  const us = (elapsed / N) * 1000;
  console.log(`  ${label.padEnd(55)} ${rps.toFixed(0).padStart(7)} req/s  ${us.toFixed(1).padStart(6)}us/req`);
}

describe("profile: where is the JS handler overhead", () => {

  // 1. Baseline: just calling a JS function (no interception)
  it("baseline: raw JS function call", async () => {
    const handler = () => ({ id: "123", name: "John" });
    const start = performance.now();
    for (let i = 0; i < N; i++) handler();
    bench("Raw JS function call (no interception)", performance.now() - start);
  });

  // 2. Baseline: creating a Response object
  it("baseline: new Response() construction", async () => {
    const start = performance.now();
    for (let i = 0; i < N; i++) {
      new Response('{"id":"123"}', { status: 200, headers: { "content-type": "application/json" } });
    }
    bench("new Response() construction", performance.now() - start);
  });

  // 3. Baseline: new Request() + URL parsing
  it("baseline: new Request() + URL parse", async () => {
    const start = performance.now();
    for (let i = 0; i < N; i++) {
      const req = new Request("http://localhost/api/bench");
      const url = new URL(req.url);
      url.pathname; url.search;
    }
    bench("new Request() + URL parse", performance.now() - start);
  });

  // 4. Just the matchRequest NAPI call (declarative mock, no handler)
  it("matchRequest() NAPI call only (declarative)", async () => {
    const interceptor = new MockpitInterceptor();
    await interceptor.addMock({
      id: "bench",
      match: { method: "GET", url: "/api/bench" },
      response: { status: 200, body: '{"ok":true}' },
    });

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await interceptor.matchRequest("GET", "/api/bench");
    }
    bench("matchRequest() NAPI (declarative mock)", performance.now() - start);
  });

  // 5. matchRequest with JS handler
  it("matchRequest() NAPI call (JS handler)", async () => {
    const interceptor = new MockpitInterceptor();
    interceptor.useHandlers([
      http.get("/api/bench", async () => MockResponse.json({ ok: true })),
    ]);

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await interceptor.matchRequest("GET", "/api/bench");
    }
    bench("matchRequest() NAPI (JS handler)", performance.now() - start);
  });

  // 6. Full interceptor flow (declarative)
  it("full interceptor: declarative mock", async () => {
    const interceptor = new MockpitInterceptor();
    await interceptor.addMock({
      id: "bench",
      match: { method: "GET", url: "/api/bench" },
      response: { status: 200, body: '{"ok":true}' },
    });
    interceptor.apply();

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await fetch("http://localhost/api/bench");
    }
    bench("Interceptor: full flow (declarative)", performance.now() - start);
    interceptor.dispose();
  });

  // 7. Full interceptor flow (JS handler)
  it("full interceptor: JS handler", async () => {
    const interceptor = new MockpitInterceptor();
    interceptor.useHandlers([
      http.get("/api/bench", async () => MockResponse.json({ ok: true })),
    ]);
    interceptor.apply();

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await fetch("http://localhost/api/bench");
    }
    bench("Interceptor: full flow (JS handler)", performance.now() - start);
    interceptor.dispose();
  });

  // 8. MSW for comparison
  it("MSW: full flow", async () => {
    const server = setupServer(
      mswHttp.get("http://localhost:9999/api/bench", () =>
        HttpResponse.json({ ok: true })
      )
    );
    server.listen({ onUnhandledRequest: "bypass" });

    const start = performance.now();
    for (let i = 0; i < N; i++) {
      await fetch("http://localhost:9999/api/bench");
    }
    bench("MSW: full flow", performance.now() - start);
    server.close();
  });

  it("summary", () => {
    console.log("\n  === Breakdown ===");
    console.log("  Interceptor overhead = Request/URL parse + matchRequest NAPI + Response build");
    console.log("  JS handler overhead = matchRequest triggers TSFN round-trip");
    console.log("  MSW overhead = Request/URL parse + path-to-regexp match + handler call + Response build");
  });
});
