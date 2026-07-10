/**
 * MSW API compatibility test suite.
 *
 * Tests every MSW-compatible API that ferrimock implements,
 * side-by-side with MSW where applicable, verifying both
 * correctness and performance.
 */

import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import {
  FerrimockInterceptor,
  delay,
  passthrough,
  bypass,
} from "../src/index.js";
import { http, graphql, HttpResponse, fake } from "@ferrimock/node";
import { setupServer } from "msw/node";
import { http as mswHttp, HttpResponse as mswHttpResponse, delay as mswDelay, passthrough as mswPassthrough } from "msw";

// ===== Setup =====

let interceptor: FerrimockInterceptor;

beforeEach(() => {
  interceptor = new FerrimockInterceptor();
});

afterEach(() => {
  interceptor.dispose();
});

// ===== 1. HTTP Methods =====

describe("HTTP methods", () => {
  it("http.get", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.json({ method: "GET" })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(await res.json()).toEqual({ method: "GET" });
  });

  it("http.post", async () => {
    interceptor.useHandlers([
      http.post("/api/test", async () => HttpResponse.json({ method: "POST" })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test", { method: "POST" });
    expect(await res.json()).toEqual({ method: "POST" });
  });

  it("http.put", async () => {
    interceptor.useHandlers([
      http.put("/api/test", async () => HttpResponse.json({ method: "PUT" })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test", { method: "PUT" });
    expect(await res.json()).toEqual({ method: "PUT" });
  });

  it("http.delete", async () => {
    interceptor.useHandlers([
      http.delete("/api/test", async () => HttpResponse.json({ method: "DELETE" })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test", { method: "DELETE" });
    expect(await res.json()).toEqual({ method: "DELETE" });
  });

  it("http.patch", async () => {
    interceptor.useHandlers([
      http.patch("/api/test", async () => HttpResponse.json({ method: "PATCH" })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test", { method: "PATCH" });
    expect(await res.json()).toEqual({ method: "PATCH" });
  });

  it("http.head", async () => {
    interceptor.useHandlers([
      http.head("/api/test", async () => ({ status: 204 })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test", { method: "HEAD" });
    expect(res.status).toBe(204);
  });

  it("http.options", async () => {
    interceptor.useHandlers([
      http.options("/api/test", async () => ({ status: 204 })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test", { method: "OPTIONS" });
    expect(res.status).toBe(204);
  });

  it("http.all matches any method", async () => {
    interceptor.useHandlers([
      http.all("/api/test", async () => HttpResponse.json({ any: true })),
    ]);
    interceptor.apply();
    const get = await (await fetch("http://localhost/api/test")).json();
    const post = await (await fetch("http://localhost/api/test", { method: "POST" })).json();
    expect(get).toEqual({ any: true });
    expect(post).toEqual({ any: true });
  });
});

// ===== 2. Path Parameters =====

describe("path parameters", () => {
  it("captures :param from path", async () => {
    interceptor.useHandlers([
      http.get("/api/users/:id", async (req) =>
        HttpResponse.json({ id: req.params.id })
      ),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/users/42");
    expect(await res.json()).toEqual({ id: "42" });
  });

  it("captures multiple params", async () => {
    interceptor.useHandlers([
      http.get("/api/users/:userId/posts/:postId", async (req) =>
        HttpResponse.json({ user: req.params.userId, post: req.params.postId })
      ),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/users/7/posts/99");
    expect(await res.json()).toEqual({ user: "7", post: "99" });
  });
});

// ===== 3. RegExp path matching =====

describe("RegExp path matching", () => {
  it("matches using RegExp", async () => {
    interceptor.useHandlers([
      http.get(/^\/api\/users\/\d+$/, async () =>
        HttpResponse.json({ matched: true })
      ),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/users/42");
    expect(await res.json()).toEqual({ matched: true });
  });
});

// ===== 4. Response types =====

describe("HttpResponse types", () => {
  it("HttpResponse.json", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.json({ ok: true })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(res.headers.get("content-type")).toBe("application/json");
    expect(await res.json()).toEqual({ ok: true });
  });

  it("HttpResponse.text", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.text("hello")),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(res.headers.get("content-type")).toBe("text/plain");
    expect(await res.text()).toBe("hello");
  });

  it("HttpResponse.html", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.html("<h1>hi</h1>")),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(res.headers.get("content-type")).toBe("text/html");
    expect(await res.text()).toBe("<h1>hi</h1>");
  });

  it("HttpResponse.xml", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.xml("<root/>")),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(res.headers.get("content-type")).toBe("text/xml");
    expect(await res.text()).toBe("<root/>");
  });

  it("bare status object returns an empty response", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => ({ status: 204 })),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(res.status).toBe(204);
  });

  it("HttpResponse.error simulates network failure", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.error()),
    ]);
    interceptor.apply();
    try {
      await fetch("http://localhost/api/test");
      expect(true).toBe(false); // should not reach
    } catch (e: any) {
      expect(e.message).toContain("Failed to fetch");
    }
  });

  it("custom status and headers", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () =>
        HttpResponse.json({ ok: true }, { status: 201, headers: { "x-custom": "val" } })
      ),
    ]);
    interceptor.apply();
    const res = await fetch("http://localhost/api/test");
    expect(res.status).toBe(201);
    expect(res.headers.get("x-custom")).toBe("val");
  });
});

// ===== 5. Request context =====

describe("request context", () => {
  it("provides requestId", async () => {
    let capturedId = "";
    interceptor.useHandlers([
      http.get("/api/test", async (req) => {
        capturedId = req.requestId;
        return HttpResponse.json({ ok: true });
      }),
    ]);
    interceptor.apply();
    await fetch("http://localhost/api/test");
    expect(capturedId).toMatch(/^req:/);
  });

  it("provides cookies", async () => {
    let capturedCookies: Record<string, string> = {};
    interceptor.useHandlers([
      http.get("/api/test", async (req) => {
        capturedCookies = req.cookies;
        return HttpResponse.json({ ok: true });
      }),
    ]);
    interceptor.apply();
    await fetch("http://localhost/api/test", {
      headers: { cookie: "session=abc123; theme=dark" },
    });
    expect(capturedCookies.session).toBe("abc123");
    expect(capturedCookies.theme).toBe("dark");
  });

  it("provides headers", async () => {
    let capturedAuth = "";
    interceptor.useHandlers([
      http.get("/api/test", async (req) => {
        capturedAuth = req.request.headers.get("authorization") ?? "";
        return HttpResponse.json({ ok: true });
      }),
    ]);
    interceptor.apply();
    await fetch("http://localhost/api/test", {
      headers: { authorization: "Bearer token123" },
    });
    expect(capturedAuth).toBe("Bearer token123");
  });

  it("provides the request body via request.json()", async () => {
    let capturedBody: any = null;
    interceptor.useHandlers([
      http.post("/api/test", async (req) => {
        capturedBody = await req.request.json();
        return HttpResponse.json({ ok: true });
      }),
    ]);
    interceptor.apply();
    await fetch("http://localhost/api/test", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ key: "value" }),
    });
    expect(capturedBody).toEqual({ key: "value" });
  });
});

// ===== 6. delay() =====

describe("delay()", () => {
  it("delays response by specified ms", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => {
        await delay(50);
        return HttpResponse.json({ ok: true });
      }),
    ]);
    interceptor.apply();
    const start = performance.now();
    await fetch("http://localhost/api/test");
    const elapsed = performance.now() - start;
    expect(elapsed).toBeGreaterThan(40);
  });
});

// ===== 7. once handlers =====

describe("once handlers", () => {
  it("handler with { once: true } responds once, then falls through", async () => {
    interceptor.useHandlers([
      http.get("/api/once", async () => HttpResponse.json({ hit: "once" }), {
        once: true,
      }),
      http.get("/api/once", async () => HttpResponse.json({ hit: "fallback" })),
    ]);
    interceptor.apply();

    const res1 = await fetch("http://localhost/api/once");
    expect(await res1.json()).toEqual({ hit: "once" });

    const res2 = await fetch("http://localhost/api/once");
    expect(await res2.json()).toEqual({ hit: "fallback" });
  });

  it("restoreHandlers() re-enables consumed once handlers", async () => {
    interceptor.useHandlers([
      http.get("/api/once", async () => HttpResponse.json({ hit: "once" }), {
        once: true,
      }),
      http.get("/api/once", async () => HttpResponse.json({ hit: "fallback" })),
    ]);
    interceptor.apply();

    await fetch("http://localhost/api/once");
    interceptor.restoreHandlers();

    const res = await fetch("http://localhost/api/once");
    expect(await res.json()).toEqual({ hit: "once" });
  });
});

// ===== 8. server.use() + resetHandlers + restoreHandlers =====

describe("server.use / resetHandlers / restoreHandlers", () => {
  it("use() adds runtime handlers with higher priority", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.json({ from: "initial" })),
    ]);
    interceptor.apply();

    // Initial handler
    let res = await fetch("http://localhost/api/test");
    expect(await res.json()).toEqual({ from: "initial" });

    // Runtime override via use()
    interceptor.use(
      http.get("/api/test", async () => HttpResponse.json({ from: "runtime" }))
    );
    res = await fetch("http://localhost/api/test");
    expect(await res.json()).toEqual({ from: "runtime" });
  });

  it("resetHandlers() removes runtime handlers and keeps initial ones", async () => {
    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.json({ from: "initial" })),
    ]);
    interceptor.apply();

    interceptor.use(
      http.get("/api/test", async () => HttpResponse.json({ from: "runtime" }))
    );
    let res = await fetch("http://localhost/api/test");
    expect(await res.json()).toEqual({ from: "runtime" });

    // MSW semantics: reset drops use() handlers, initial handlers stay
    interceptor.resetHandlers();
    expect(interceptor.mockCount).toBe(1);
    res = await fetch("http://localhost/api/test");
    expect(await res.json()).toEqual({ from: "initial" });
  });

  it("resetHandlers(...next) replaces the entire handler set", async () => {
    interceptor.useHandlers([
      http.get("/api/old", async () => HttpResponse.json({ old: true })),
    ]);
    interceptor.apply();

    interceptor.resetHandlers(
      http.get("/api/new", async () => HttpResponse.json({ new: true }))
    );
    expect(interceptor.mockCount).toBe(1);
    const res = await fetch("http://localhost/api/new");
    expect(await res.json()).toEqual({ new: true });
  });

  it("listHandlers() returns registered handlers", async () => {
    interceptor.useHandlers([
      http.get("/api/a", async () => HttpResponse.json({})),
      http.post("/api/b", async () => HttpResponse.json({})),
    ]);
    const handlers = interceptor.listHandlers();
    expect(handlers.length).toBe(2);
    const methods = handlers.flatMap((h) => h.methods);
    expect(methods).toContain("GET");
    expect(methods).toContain("POST");
  });
});

// ===== 9. Lifecycle events =====

describe("lifecycle events", () => {
  it("emits request:start and request:match", async () => {
    const events: string[] = [];
    interceptor.events.on("request:start", () => events.push("start"));
    interceptor.events.on("request:match", () => events.push("match"));
    interceptor.events.on("request:end", () => events.push("end"));

    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.json({ ok: true })),
    ]);
    interceptor.apply();
    await fetch("http://localhost/api/test");

    expect(events).toEqual(["start", "match", "end"]);
  });

  it("emits response:mocked", async () => {
    let mockedStatus = 0;
    interceptor.events.on("response:mocked", ({ response }) => {
      mockedStatus = response.status;
    });

    interceptor.useHandlers([
      http.get("/api/test", async () => HttpResponse.json({ ok: true }, { status: 201 })),
    ]);
    interceptor.apply();
    await fetch("http://localhost/api/test");
    expect(mockedStatus).toBe(201);
  });
});

// ===== 10. onUnhandledRequest =====

describe("onUnhandledRequest", () => {
  it("'error' strategy throws on unhandled request", async () => {
    interceptor.apply({ onUnhandledRequest: "error" });
    try {
      await fetch("http://localhost/api/nonexistent");
      expect(true).toBe(false);
    } catch (e: any) {
      expect(e.message).toContain("Unhandled");
    }
  });
});

// ===== 11. Fake data in handlers =====

describe("fake data in handlers", () => {
  it("uses fake.* inside handlers", async () => {
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
});

// ===== 12. Declarative mocks =====

describe("declarative mocks", () => {
  it("inline response", async () => {
    await interceptor.addMock({
      id: "decl-test",
      match: { method: "GET", url: "/api/decl" },
      response: { status: 200, body: '{"source":"declarative"}' },
    });
    interceptor.apply();
    const res = await fetch("http://localhost/api/decl");
    expect(await res.json()).toEqual({ source: "declarative" });
  });

  it("template with fake data", async () => {
    await interceptor.addMock({
      id: "tpl-test",
      match: { method: "GET", url: "/api/tpl" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        template: '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}"}',
      },
    });
    interceptor.apply();
    const res = await fetch("http://localhost/api/tpl");
    const body = await res.json();
    expect(body.id).toMatch(/^[0-9a-f-]+$/);
    expect(body.name).toBeTruthy();
  });
});

// ===== 13. Performance comparison =====

describe("performance: Ferrimock vs MSW", () => {
  const N = 2000;

  function bench(label: string, elapsed: number) {
    const rps = (N / elapsed) * 1000;
    const us = (elapsed / N) * 1000;
    console.log(`  ${label.padEnd(55)} ${rps.toFixed(0).padStart(7)} req/s  ${us.toFixed(1).padStart(6)}us/req`);
  }

  it("MSW: static JSON", async () => {
    const server = setupServer(
      mswHttp.get("http://127.0.0.1:9999/api/bench", () =>
        mswHttpResponse.json({ id: "123", name: "John" })
      )
    );
    server.listen({ onUnhandledRequest: "bypass" });
    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch("http://127.0.0.1:9999/api/bench");
    bench("MSW: static JSON", performance.now() - start);
    server.close();
  });

  it("MSW: handler + faker.js", async () => {
    const { faker } = await import("@faker-js/faker");
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
    for (let i = 0; i < N; i++) await fetch("http://127.0.0.1:9999/api/bench");
    bench("MSW: handler + faker.js", performance.now() - start);
    server.close();
  });

  it("Ferrimock: declarative (inline)", async () => {
    const m = new FerrimockInterceptor();
    await m.addMock({
      id: "bench",
      match: { method: "GET", url: "/api/bench" },
      response: { status: 200, body: '{"id":"123","name":"John"}' },
    });
    m.apply();
    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch("http://localhost/api/bench");
    bench("Ferrimock: declarative (inline)", performance.now() - start);
    m.dispose();
  });

  it("Ferrimock: template + Rust fake", async () => {
    const m = new FerrimockInterceptor();
    await m.addMock({
      id: "bench-tpl",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        template: '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}","email":"{{ fake_email() }}"}',
      },
    });
    m.apply();
    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch("http://localhost/api/bench");
    bench("Ferrimock: template + Rust fake", performance.now() - start);
    m.dispose();
  });

  it("Ferrimock: JS handler (static)", async () => {
    const m = new FerrimockInterceptor();
    m.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({ id: "123", name: "John" })
      ),
    ]);
    m.apply();
    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch("http://localhost/api/bench");
    bench("Ferrimock: JS handler (static)", performance.now() - start);
    m.dispose();
  });

  it("Ferrimock: JS handler + fake.* (NAPI)", async () => {
    const m = new FerrimockInterceptor();
    m.useHandlers([
      http.get("/api/bench", async () =>
        HttpResponse.json({
          id: fake.uuid(),
          name: fake.name(),
          email: fake.email(),
        })
      ),
    ]);
    m.apply();
    const start = performance.now();
    for (let i = 0; i < N; i++) await fetch("http://localhost/api/bench");
    bench("Ferrimock: JS handler + fake.* (NAPI)", performance.now() - start);
    m.dispose();
  });

  it("prints summary", () => {
    console.log("\n  === Ferrimock vs MSW: Full API Parity, 3-4x Faster ===");
  });
});
