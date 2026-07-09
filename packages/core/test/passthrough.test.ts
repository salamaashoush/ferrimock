import { describe, it, expect, afterEach } from "bun:test";
import { MockpitInterceptor } from "../src/interceptor.js";
import { http, HttpResponse } from "@mockpit/node";

describe("passthrough", () => {
  let interceptor: MockpitInterceptor | null = null;
  let server: ReturnType<typeof Bun.serve> | null = null;
  afterEach(() => {
    interceptor?.dispose();
    interceptor = null;
    server?.stop();
    server = null;
  });

  it("passes the request body through intact on an unmatched POST", async () => {
    // Echo server: returns whatever body it received.
    server = Bun.serve({
      port: 0,
      async fetch(req) {
        const body = await req.text();
        return new Response(body, { headers: { "x-echo": "1" } });
      },
    });
    const base = `http://127.0.0.1:${server.port}`;

    interceptor = new MockpitInterceptor();
    // A mock for a DIFFERENT path, so /echo passes through.
    interceptor.useHandlers([
      http.get("/mocked", async () => HttpResponse.json({ ok: true })),
    ]);
    interceptor.apply();

    const payload = JSON.stringify({ hello: "world", n: 7 });
    const res = await fetch(`${base}/echo`, {
      method: "POST",
      body: payload,
      headers: { "content-type": "application/json" },
    });
    expect(res.headers.get("x-echo")).toBe("1"); // really hit the echo server
    expect(await res.text()).toBe(payload); // body survived passthrough
  });

  it("passes through with zero registered mocks (fast path) still hitting the network", async () => {
    server = Bun.serve({
      port: 0,
      fetch() {
        return new Response("real", { headers: { "x-real": "1" } });
      },
    });
    const base = `http://127.0.0.1:${server.port}`;

    interceptor = new MockpitInterceptor();
    interceptor.apply(); // no mocks registered

    const res = await fetch(`${base}/anything`);
    expect(res.headers.get("x-real")).toBe("1");
    expect(await res.text()).toBe("real");
  });
});
