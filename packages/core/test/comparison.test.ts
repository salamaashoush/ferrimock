/**
 * Ferrimock performance smoke.
 *
 * This file measures ferrimock ONLY. It deliberately does not benchmark MSW.
 *
 * Loading two fetch interceptors into one process penalises whichever runs
 * second — measured at 8x for MSW when it followed ferrimock — so any
 * cross-library number produced in a single process is an artifact of test
 * ordering, not of either library. The published comparison therefore runs each
 * library in its own process, alternating which goes first:
 *
 *   cd benchmarks && node fair.mjs                  # bun
 *   cd benchmarks && BENCH_RUNTIME=node node fair.mjs
 *
 * The two groups below are also kept apart on purpose. Interception has no
 * sockets; server mode is a real axum server over real TCP. They are not
 * comparable with each other, and server-mode figures must never be quoted
 * against an interceptor's.
 */

import { describe, it, expect, afterAll } from "bun:test";
import { FerrimockServer, fake } from "@ferrimock/node";
import { http, HttpResponse, setupServer } from "../src/index.js";

const N = Number(process.env.BENCH_N ?? 2000);
const WARMUP = Number(process.env.BENCH_WARMUP ?? 500);
const REPEATS = Number(process.env.BENCH_REPEATS ?? 3);

type Row = { label: string; usPerReq: number; rps: number };
const interception: Row[] = [];
const serverMode: Row[] = [];

/**
 * Time `N` requests, `REPEATS` times, and keep the fastest run — the minimum is
 * the least GC- and scheduler-contaminated estimate of steady-state cost.
 */
async function measure(label: string, request: () => Promise<unknown>): Promise<Row> {
  for (let i = 0; i < WARMUP; i++) await request();

  let bestUs = Number.POSITIVE_INFINITY;
  for (let repeat = 0; repeat < REPEATS; repeat++) {
    const start = performance.now();
    for (let i = 0; i < N; i++) await request();
    const usPerReq = ((performance.now() - start) / N) * 1000;
    if (usPerReq < bestUs) bestUs = usPerReq;
  }

  return { label, usPerReq: bestUs, rps: 1e6 / bestUs };
}

const get = (url: string) => async () => {
  const res = await fetch(url);
  await res.arrayBuffer();
};

describe("Interception (in-process, no sockets)", () => {
  it("static JSON", async () => {
    const server = setupServer(
      http.get("http://bench.test/api/bench", () =>
        HttpResponse.json({ id: "123", name: "John" }),
      ),
    );
    server.listen({ onUnhandledRequest: "error" });
    interception.push(await measure("Static JSON", get("http://bench.test/api/bench")));
    server.close();

    expect(interception.at(-1)!.usPerReq).toBeGreaterThan(0);
  });

  it("handler with :params", async () => {
    const server = setupServer(
      http.get("http://bench.test/api/users/:id", ({ params }) =>
        HttpResponse.json({ id: params.id, name: "John" }),
      ),
    );
    server.listen({ onUnhandledRequest: "error" });
    interception.push(await measure("Path params", get("http://bench.test/api/users/42")));
    server.close();

    expect(interception.at(-1)!.usPerReq).toBeGreaterThan(0);
  });

  it("handler generating fake data", async () => {
    const server = setupServer(
      http.get("http://bench.test/api/bench", () =>
        HttpResponse.json({ id: fake.uuid(), name: fake.name(), email: fake.email() }),
      ),
    );
    server.listen({ onUnhandledRequest: "error" });
    interception.push(await measure("Handler + fake data", get("http://bench.test/api/bench")));
    server.close();

    expect(interception.at(-1)!.usPerReq).toBeGreaterThan(0);
  });
});

describe("Server mode (real axum server, real TCP)", () => {
  it("static declarative mock", async () => {
    const server = new FerrimockServer();
    await server.addMock({
      id: "bench-static",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"id":"123","name":"John"}',
      },
    });
    const url = await server.listen();
    serverMode.push(await measure("Declarative (pure Rust)", get(`${url}/api/bench`)));
    await server.close();

    expect(serverMode.at(-1)!.usPerReq).toBeGreaterThan(0);
  });

  it("template with Rust fake data", async () => {
    const server = new FerrimockServer();
    await server.addMock({
      id: "bench-template",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        template:
          '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}","email":"{{ fake_email() }}"}',
      },
    });
    const url = await server.listen();
    serverMode.push(await measure("Template + Rust fake data", get(`${url}/api/bench`)));
    await server.close();

    expect(serverMode.at(-1)!.usPerReq).toBeGreaterThan(0);
  });

  it("JS handler over NAPI", async () => {
    const server = new FerrimockServer();
    server.useHandlers([
      http.get("/api/users/:id", async (req) =>
        HttpResponse.json({ id: req.params.id, name: "John" }),
      ),
    ]);
    const url = await server.listen();
    serverMode.push(await measure("JS handler over NAPI", get(`${url}/api/users/42`)));
    await server.close();

    expect(serverMode.at(-1)!.usPerReq).toBeGreaterThan(0);
  });
});

afterAll(() => {
  const runtime = typeof Bun !== "undefined" ? `bun ${Bun.version}` : `node ${process.version}`;
  const print = (title: string, rows: Row[]) => {
    console.log(`\n  === ${title} (${runtime}, best of ${REPEATS} x ${N}) ===`);
    for (const row of rows) {
      console.log(
        `  ${row.label.padEnd(28)} ${`${row.usPerReq.toFixed(1)}us`.padStart(10)}  ${row.rps.toFixed(0).padStart(7)} req/s`,
      );
    }
  };

  print("Interception (in-process)", interception);
  print("Server mode (real TCP)", serverMode);
  console.log(
    "\n  These two groups are not comparable with each other, and neither is\n" +
      "  comparable with a number measured in a process that also loaded MSW.\n" +
      "  For ferrimock vs MSW see benchmarks/fair.mjs.\n",
  );
});
