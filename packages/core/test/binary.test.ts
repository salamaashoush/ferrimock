import { describe, it, expect, afterEach } from "bun:test";
import { FerrimockInterceptor } from "../src/interceptor.js";
import { http, HttpResponse } from "ferrimock-node";

// Non-UTF8 bytes: 0xFF/0xFE/0x80 are invalid UTF-8 start/continuation bytes.
// The old String-based transport corrupted these (from_utf8_lossy / unwrap_or_default).
const BINARY = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0xff, 0x00, 0xfe, 0x80, 0x01, 0x7f]);

describe("binary body transport", () => {
  let interceptor: FerrimockInterceptor | null = null;
  afterEach(() => {
    interceptor?.dispose();
    interceptor = null;
  });

  it("round-trips arbitrary binary bytes through a handler arrayBuffer response", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/img.png", async () =>
        HttpResponse.arrayBuffer(BINARY, {
          headers: { "content-type": "image/png" },
        })
      ),
    ]);
    interceptor.apply();

    const res = await fetch("http://localhost/img.png");
    expect(res.headers.get("content-type")).toBe("image/png");
    const got = new Uint8Array(await res.arrayBuffer());
    expect(Array.from(got)).toEqual(Array.from(BINARY));
  });

  it("does not corrupt UTF-8 JSON bodies", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/api/user", async () =>
        HttpResponse.json({ name: "Jürgen", emoji: "🚀", n: 42 })
      ),
    ]);
    interceptor.apply();

    const res = await fetch("http://localhost/api/user");
    const body = await res.json();
    expect(body).toEqual({ name: "Jürgen", emoji: "🚀", n: 42 });
  });

  it("preserves binary for a declarative mock served via matchRequest", async () => {
    interceptor = new FerrimockInterceptor();
    // Declarative inline bodies are strings; verify the matchRequest output is
    // bytes and decodes back to the exact UTF-8 string (no lossy round-trip).
    await interceptor.addMock({
      id: "decl-text",
      match: { method: "GET", url: "/decl" },
      response: { status: 200, body: '{"msg":"héllo ☃"}' },
    });
    const match = await interceptor.matchRequest("GET", "/decl");
    expect(match).not.toBeNull();
    expect(match!.body).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(match!.body)).toBe('{"msg":"héllo ☃"}');
  });
});
