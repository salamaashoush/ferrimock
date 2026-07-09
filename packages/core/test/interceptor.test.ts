import { describe, it, expect, beforeAll, afterAll, beforeEach } from "bun:test";
import { MockpitInterceptor } from "../src/index.js";
import { http, HttpResponse, fake } from "@mockpit/node";
import { setupServer } from "msw/node";
import { http as mswHttp, HttpResponse as mswHttpResponse } from "msw";
import { faker } from "@faker-js/faker";
import { resolve } from "node:path";

const N = 2000;

function bench(label: string, rps: number, usPerReq: number) {
  console.log(
    `  ${label.padEnd(50)} ${rps.toFixed(0).padStart(7)} req/s  (${usPerReq.toFixed(0)}us/req)`
  );
}

describe("MockpitInterceptor", () => {
  let interceptor: MockpitInterceptor;

  beforeAll(async () => {
    interceptor = new MockpitInterceptor();
  });

  afterAll(() => {
    interceptor.dispose();
  });

  beforeEach(() => {
    interceptor.dispose();
    interceptor.resetHandlers();
  });

  it("intercepts fetch with declarative mock", async () => {
    await interceptor.addMock({
      id: "test-hello",
      match: { method: "GET", url: "/api/hello" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"message":"hello from interceptor"}',
      },
    });

    interceptor.apply();

    const res = await fetch("http://localhost/api/hello");
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.message).toBe("hello from interceptor");
  });

  it("intercepts fetch with JS handler", async () => {
    interceptor.useHandlers([
      http.get("/api/users/:id", async (req) =>
        HttpResponse.json({ id: req.params.id, name: "John" })
      ),
    ]);

    interceptor.apply();

    const res = await fetch("http://localhost/api/users/42");
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.id).toBe("42");
    expect(body.name).toBe("John");
  });

  it("intercepts fetch with fake data", async () => {
    interceptor.useHandlers([
      http.get("/api/user", async () =>
        HttpResponse.json({
          id: fake.uuid(),
          name: fake.name(),
          email: fake.email(),
        })
      ),
    ]);

    interceptor.apply();

    const res = await fetch("http://localhost/api/user");
    const body = await res.json();
    expect(body.id).toMatch(/^[0-9a-f-]+$/);
    expect(body.name).toBeTruthy();
    expect(body.email).toContain("@");
  });

  it("passes through unmatched requests", async () => {
    interceptor.apply();

    // This should pass through to real fetch (which will fail since no server)
    try {
      await fetch("http://127.0.0.1:1/nonexistent");
    } catch (e: any) {
      // Connection refused is expected -- it means passthrough worked
      expect(e.message).toContain("Unable to connect");
    }
  });

  it("loads mocks from directory", async () => {
    const count = await interceptor.loadMocks(
      resolve(import.meta.dir, "fixtures/mocks")
    );
    expect(count).toBeGreaterThan(0);
    interceptor.apply();

    const res = await fetch("http://localhost/api/hello");
    expect(res.status).toBe(200);
  });
});

describe("Interceptor vs MSW vs Server benchmark", () => {
  // -- MSW baseline --
  it("MSW: static JSON (fetch intercept)", async () => {
    const server = setupServer(
      mswHttp.get("http://127.0.0.1:9999/api/bench", () =>
        mswHttpResponse.json({ id: "123", name: "John", source: "msw" })
      )
    );
    server.listen({ onUnhandledRequest: "bypass" });

    const start = performance.now();
    for (let i = 0; i < N; i++)
      await fetch("http://127.0.0.1:9999/api/bench");
    const elapsed = performance.now() - start;
    bench("MSW: static JSON", (N / elapsed) * 1000, (elapsed / N) * 1000);
    server.close();
  });

  it("MSW: handler + faker.js", async () => {
    const server = setupServer(
      mswHttp.get("http://127.0.0.1:9999/api/bench", () =>
        mswHttpResponse.json({
          id: faker.string.uuid(),
          name: faker.person.fullName(),
          email: faker.internet.email(),
        })
      )
    );
    server.listen({ onUnhandledRequest: "bypass" });

    const start = performance.now();
    for (let i = 0; i < N; i++)
      await fetch("http://127.0.0.1:9999/api/bench");
    const elapsed = performance.now() - start;
    bench("MSW: handler + faker.js", (N / elapsed) * 1000, (elapsed / N) * 1000);
    server.close();
  });

  // -- Mockpit Interceptor (no HTTP) --
  it("Mockpit interceptor: static declarative", async () => {
    const interceptor = new MockpitInterceptor();
    await interceptor.addMock({
      id: "bench",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"id":"123","name":"John","source":"mockpit-intercept"}',
      },
    });
    interceptor.apply();

    const start = performance.now();
    for (let i = 0; i < N; i++)
      await fetch("http://localhost/api/bench");
    const elapsed = performance.now() - start;
    bench(
      "Mockpit interceptor: static declarative",
      (N / elapsed) * 1000,
      (elapsed / N) * 1000
    );
    interceptor.dispose();
  });

  it("Mockpit interceptor: template + Rust fake data", async () => {
    const interceptor = new MockpitInterceptor();
    await interceptor.addMock({
      id: "bench-tpl",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        template:
          '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}","email":"{{ fake_email() }}"}',
      },
    });
    interceptor.apply();

    const start = performance.now();
    for (let i = 0; i < N; i++)
      await fetch("http://localhost/api/bench");
    const elapsed = performance.now() - start;
    bench(
      "Mockpit interceptor: template + Rust fake",
      (N / elapsed) * 1000,
      (elapsed / N) * 1000
    );
    interceptor.dispose();
  });

  it("Mockpit interceptor: JS handler static", async () => {
    const interceptor = new MockpitInterceptor();
    interceptor.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({ id: "123", name: "John", source: "handler" })
      ),
    ]);
    interceptor.apply();

    const start = performance.now();
    for (let i = 0; i < N; i++)
      await fetch("http://localhost/api/bench");
    const elapsed = performance.now() - start;
    bench(
      "Mockpit interceptor: JS handler (static)",
      (N / elapsed) * 1000,
      (elapsed / N) * 1000
    );
    interceptor.dispose();
  });

  it("Mockpit interceptor: JS handler + fake.*", async () => {
    const interceptor = new MockpitInterceptor();
    interceptor.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({
          id: fake.uuid(),
          name: fake.name(),
          email: fake.email(),
        })
      ),
    ]);
    interceptor.apply();

    const start = performance.now();
    for (let i = 0; i < N; i++)
      await fetch("http://localhost/api/bench");
    const elapsed = performance.now() - start;
    bench(
      "Mockpit interceptor: JS handler + fake.* (NAPI)",
      (N / elapsed) * 1000,
      (elapsed / N) * 1000
    );
    interceptor.dispose();
  });

  it("prints summary", () => {
    console.log("\n  === Interceptor mode: no HTTP, same as MSW ===");
    console.log("  Both MSW and Mockpit interceptor patch fetch().");
    console.log("  Mockpit declarative mocks use Rust matching + response gen.");
    console.log("  Mockpit templates render in Rust (Tera engine).");
    console.log("  JS handlers still need TSFN for the callback.");
  });
});
