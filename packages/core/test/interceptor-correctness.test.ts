import { describe, it, expect, afterEach } from "bun:test";
import { FerrimockInterceptor } from "../src/interceptor.js";
import { http, HttpResponse } from "ferrimock-node";

describe("interceptor correctness: abort, redirects, XHR", () => {
  let interceptor: FerrimockInterceptor | null = null;
  afterEach(() => {
    interceptor?.dispose();
    interceptor = null;
  });

  // ---- AbortSignal ----
  it("rejects with AbortError when the signal is already aborted", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/x", async () => HttpResponse.json({ ok: true })),
    ]);
    interceptor.apply();

    const ctrl = new AbortController();
    ctrl.abort();
    let err: any;
    try {
      await fetch("http://localhost/x", { signal: ctrl.signal });
    } catch (e) {
      err = e;
    }
    expect(err).toBeInstanceOf(DOMException);
    expect(err.name).toBe("AbortError");
  });

  it("aborts a delayed mock mid-flight", async () => {
    interceptor = new FerrimockInterceptor();
    await interceptor.addMock({
      id: "slow",
      match: { method: "GET", url: "/slow" },
      response: { status: 200, body: "ok" },
      delay: "500ms",
    });
    interceptor.apply();

    const ctrl = new AbortController();
    setTimeout(() => ctrl.abort(), 20);
    let err: any;
    try {
      await fetch("http://localhost/slow", { signal: ctrl.signal });
    } catch (e) {
      err = e;
    }
    expect(err?.name).toBe("AbortError");
  });

  // ---- Redirect following ----
  it("follows a mocked 302 redirect to its Location target", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/old", async () =>
        HttpResponse.text("", { status: 302, headers: { location: "/new" } })
      ),
      http.get("/new", async () => HttpResponse.json({ at: "new" })),
    ]);
    interceptor.apply();

    const res = await fetch("http://localhost/old");
    expect(res.status).toBe(200);
    expect(await res.json()).toEqual({ at: "new" });
  });

  it("returns the 3xx as-is with redirect: 'manual'", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/old", async () =>
        HttpResponse.text("", { status: 302, headers: { location: "/new" } })
      ),
    ]);
    interceptor.apply();

    const res = await fetch("http://localhost/old", { redirect: "manual" });
    expect(res.status).toBe(302);
    expect(res.headers.get("location")).toBe("/new");
  });

  // ---- XHR (only in environments that provide XMLHttpRequest, e.g. jsdom) ----
  const hasXHR = typeof XMLHttpRequest !== "undefined";
  it.skipIf(!hasXHR)("XHR: fires load once, sets status/statusText/responseText", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/xhr", async () => HttpResponse.text("hello", { status: 201 })),
    ]);
    interceptor.apply();

    let loadCount = 0;
    const result = await new Promise<any>((resolve) => {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", "http://localhost/xhr");
      xhr.onload = () => {
        loadCount++;
        resolve({
          status: xhr.status,
          statusText: xhr.statusText,
          responseText: xhr.responseText,
          readyState: xhr.readyState,
        });
      };
      xhr.send();
    });

    expect(result.status).toBe(201);
    expect(result.statusText).toBe("Created");
    expect(result.responseText).toBe("hello");
    expect(result.readyState).toBe(4);
    // give a tick to ensure no second load fires
    await new Promise((r) => setTimeout(r, 5));
    expect(loadCount).toBe(1);
  });

  it.skipIf(!hasXHR)("XHR: responseType 'json' yields a parsed object", async () => {
    interceptor = new FerrimockInterceptor();
    interceptor.useHandlers([
      http.get("/xhr-json", async () => HttpResponse.json({ a: 1, b: "two" })),
    ]);
    interceptor.apply();

    const response = await new Promise<any>((resolve) => {
      const xhr = new XMLHttpRequest();
      xhr.open("GET", "http://localhost/xhr-json");
      xhr.responseType = "json";
      xhr.onload = () => resolve(xhr.response);
      xhr.send();
    });

    expect(response).toEqual({ a: 1, b: "two" });
  });
});
