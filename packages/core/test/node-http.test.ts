import { describe, it, expect, afterEach } from "bun:test";
import http from "node:http";
import { MockpitInterceptor } from "../src/interceptor.js";
import { http as mock, HttpResponse } from "@mockpit/node";

function nodeRequest(
  url: string,
  options: http.RequestOptions = {},
  body?: string
): Promise<{ status: number; body: string }> {
  return new Promise((resolve, reject) => {
    const req = http.request(url, options, (res) => {
      let data = "";
      res.setEncoding("utf8");
      res.on("data", (c) => (data += c));
      res.on("end", () => resolve({ status: res.statusCode ?? 0, body: data }));
    });
    req.on("error", reject);
    if (body) req.write(body);
    req.end();
  });
}

// Bun's node:http is not patchable by @mswjs/interceptors (it patches Node's
// internals). Verified under real Node via `bun run verify:node-http`.
const isBun = typeof (globalThis as any).Bun !== "undefined";

describe.skipIf(isBun)("node http interception", () => {
  let interceptor: MockpitInterceptor | null = null;
  afterEach(() => {
    interceptor?.dispose();
    interceptor = null;
  });

  it("intercepts a GET via node:http and returns the mocked body", async () => {
    interceptor = new MockpitInterceptor();
    interceptor.useHandlers([
      mock.get("/api/http", async () => HttpResponse.json({ ok: true, via: "node-http" })),
    ]);
    interceptor.apply();

    const res = await nodeRequest("http://example.test/api/http");
    expect(res.status).toBe(200);
    expect(JSON.parse(res.body)).toEqual({ ok: true, via: "node-http" });
  });

  it("forwards the request body to body-matching mocks over node:http", async () => {
    interceptor = new MockpitInterceptor();
    interceptor.useHandlers([
      mock.post("/echo", async ({ request }) => {
        const data = (await request.json()) as any;
        return HttpResponse.json({ received: data?.name ?? null });
      }),
    ]);
    interceptor.apply();

    const res = await nodeRequest(
      "http://example.test/echo",
      { method: "POST", headers: { "content-type": "application/json" } },
      JSON.stringify({ name: "ada" })
    );
    expect(JSON.parse(res.body)).toEqual({ received: "ada" });
  });
});
