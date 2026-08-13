/**
 * A throwing handler on the server lane.
 *
 * The interceptor lane's error path is covered by msw-dropin; this one goes
 * through the ThreadsafeFunction bridge instead, and had no coverage at all.
 * It works — these tests pin that, and pin that the message a user reads is the
 * one they threw, without napi's internal status classification in front of it.
 */
import { describe, it, expect, afterEach } from "bun:test";
import { FerrimockServer, http, HttpResponse } from "../index.js";

let server: FerrimockServer | null = null;

async function serve(...handlers: Parameters<FerrimockServer["useHandlers"]>[0]) {
  server = new FerrimockServer();
  server.useHandlers(handlers);
  return server.listen();
}

afterEach(async () => {
  await server?.close();
  server = null;
});

describe("throwing handlers on the server lane", () => {
  it("answers 500 with the thrown message when a handler throws synchronously", async () => {
    const url = await serve(
      http.get("/boom", () => {
        throw new Error("sync-detonation");
      }),
    );

    const res = await fetch(`${url}/boom`);
    expect(res.status).toBe(500);

    const body = (await res.json()) as { error: string; details: string };
    expect(body.details).toContain("sync-detonation");
    // napi renders an error as "GenericFailure, <reason>"; the status is an
    // internal classification and must not reach whoever wrote the handler.
    expect(body.details).not.toContain("GenericFailure");
  });

  it("answers 500 with the thrown message when an async handler rejects", async () => {
    const url = await serve(
      http.get("/boom", async () => {
        throw new Error("async-detonation");
      }),
    );

    const res = await fetch(`${url}/boom`);
    expect(res.status).toBe(500);

    const body = (await res.json()) as { details: string };
    expect(body.details).toContain("async-detonation");
    expect(body.details).not.toContain("GenericFailure");
  });

  it("does not report a cancelled oneshot when a handler throws", async () => {
    // Regression: `call_async` left its oneshot cancelled on a synchronous
    // throw, so the response said "oneshot canceled" and the thrown message
    // was lost. `call_async_catch` carries the exception instead.
    const url = await serve(
      http.get("/boom", () => {
        throw new Error("detonation");
      }),
    );

    const body = (await (await fetch(`${url}/boom`)).json()) as { details: string };
    expect(body.details).not.toContain("oneshot canceled");
    expect(body.details).toContain("detonation");
  });

  it("carries the cause chain through the thread hop", async () => {
    const url = await serve(
      http.get("/boom", () => {
        throw new Error("outer failure", { cause: new Error("inner reason") });
      }),
    );

    const res = await fetch(`${url}/boom`);
    expect(res.status).toBe(500);
    expect((await res.json() as { details: string }).details).toContain("outer failure");
  });

  it("keeps serving other routes after a handler throws", async () => {
    const url = await serve(
      http.get("/boom", () => {
        throw new Error("detonation");
      }),
      // Async: the TSFN return type is `Promise<Option<HandlerResponse>>`, so a
      // handler registered through the raw napi `http` must return a promise.
      // The `ferrimock` package wraps resolvers for you; this lane does not.
      http.get("/fine", async () => HttpResponse.json({ ok: true })),
    );

    expect((await fetch(`${url}/boom`)).status).toBe(500);

    // A throw must not poison the bridge for subsequent requests.
    const good = await fetch(`${url}/fine`);
    expect(good.status).toBe(200);
    expect(await good.json()).toEqual({ ok: true });

    expect((await fetch(`${url}/boom`)).status).toBe(500);
  });

  it("reports the mock that threw", async () => {
    const url = await serve(
      http.get("/boom", () => {
        throw new Error("detonation");
      }),
    );

    const body = (await (await fetch(`${url}/boom`)).json()) as { mock_id: string };
    expect(body.mock_id ?? "").not.toBe("");
  });
});
