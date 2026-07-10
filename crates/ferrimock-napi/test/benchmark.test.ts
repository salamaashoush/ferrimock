import { describe, it, expect } from "bun:test";
import { FerrimockServer, http, HttpResponse, fake } from "../index.js";

// ===== fake namespace test =====

describe("fake namespace", () => {
  it("generates all data types", () => {
    expect(fake.name()).toBeTruthy();
    expect(fake.email()).toContain("@");
    expect(fake.uuid()).toMatch(/^[0-9a-f-]+$/);
    expect(fake.phone()).toBeTruthy();
    expect(fake.city()).toBeTruthy();
    expect(fake.country()).toBeTruthy();
    expect(fake.url()).toContain("://");
    expect(fake.ipv4()).toMatch(/\d+\.\d+\.\d+\.\d+/);
    expect(fake.creditCard()).toBeTruthy();
    expect(fake.date()).toBeTruthy();
    expect(typeof fake.boolean()).toBe("boolean");
    expect(typeof fake.number()).toBe("number");
    expect(typeof fake.float()).toBe("number");
    expect(fake.jwt()).toContain(".");
    expect(fake.slug()).toContain("-");
    expect(fake.sentence()).toBeTruthy();
    expect(fake.word()).toBeTruthy();
  });

  it("can be used inside handlers", async () => {
    const server = new FerrimockServer();
    server.useHandlers([
      http.get("/api/user", async () => {
        return HttpResponse.json({
          id: fake.uuid(),
          name: fake.name(),
          email: fake.email(),
          city: fake.city(),
          createdAt: fake.date(),
        });
      }),
    ]);

    const url = await server.listen();
    const res = await fetch(`${url}/api/user`);
    const body = await res.json();
    expect(body.id).toMatch(/^[0-9a-f-]+$/);
    expect(body.name).toBeTruthy();
    expect(body.email).toContain("@");
    expect(body.city).toBeTruthy();
    await server.close();
  });
});

// ===== Benchmark: Handler vs Declarative =====

describe("benchmark: handler vs declarative mocks", () => {
  const WARMUP = 50;
  const ITERATIONS = 500;

  it("benchmarks YAML declarative mock throughput", async () => {
    const server = new FerrimockServer();

    // Add declarative mock
    await server.addMock({
      id: "bench-declarative",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"id":"123","name":"John","source":"declarative"}',
      },
    });

    const url = await server.listen();

    // Warmup
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    // Benchmark
    const start = performance.now();
    for (let i = 0; i < ITERATIONS; i++) {
      await fetch(`${url}/api/bench`);
    }
    const elapsed = performance.now() - start;
    const rps = (ITERATIONS / elapsed) * 1000;

    console.log(`\n  Declarative (YAML/JSON): ${ITERATIONS} requests in ${elapsed.toFixed(0)}ms = ${rps.toFixed(0)} req/s`);
    expect(rps).toBeGreaterThan(100); // sanity check

    await server.close();
  });

  it("benchmarks JS handler mock throughput", async () => {
    const server = new FerrimockServer();

    server.useHandlers([
      http.get("/api/bench", async () => {
        return HttpResponse.json({
          id: "123",
          name: "John",
          source: "handler",
        });
      }),
    ]);

    const url = await server.listen();

    // Warmup
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    // Benchmark
    const start = performance.now();
    for (let i = 0; i < ITERATIONS; i++) {
      await fetch(`${url}/api/bench`);
    }
    const elapsed = performance.now() - start;
    const rps = (ITERATIONS / elapsed) * 1000;

    console.log(`  JS Handler (static):    ${ITERATIONS} requests in ${elapsed.toFixed(0)}ms = ${rps.toFixed(0)} req/s`);
    expect(rps).toBeGreaterThan(50);

    await server.close();
  });

  it("benchmarks JS handler with fake data throughput", async () => {
    const server = new FerrimockServer();

    server.useHandlers([
      http.get("/api/bench", async () => {
        return HttpResponse.json({
          id: fake.uuid(),
          name: fake.name(),
          email: fake.email(),
          source: "handler+fake",
        });
      }),
    ]);

    const url = await server.listen();

    // Warmup
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    // Benchmark
    const start = performance.now();
    for (let i = 0; i < ITERATIONS; i++) {
      await fetch(`${url}/api/bench`);
    }
    const elapsed = performance.now() - start;
    const rps = (ITERATIONS / elapsed) * 1000;

    console.log(`  JS Handler (fake data): ${ITERATIONS} requests in ${elapsed.toFixed(0)}ms = ${rps.toFixed(0)} req/s`);
    expect(rps).toBeGreaterThan(50);

    await server.close();
  });

  it("benchmarks declarative template mock throughput", async () => {
    const server = new FerrimockServer();

    await server.addMock({
      id: "bench-template",
      match: { method: "GET", url: "/api/bench" },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        template: '{"id":"{{ fake_uuid() }}","name":"{{ fake_name() }}","email":"{{ fake_email() }}","source":"template"}',
      },
    });

    const url = await server.listen();

    // Warmup
    for (let i = 0; i < WARMUP; i++) await fetch(`${url}/api/bench`);

    // Benchmark
    const start = performance.now();
    for (let i = 0; i < ITERATIONS; i++) {
      await fetch(`${url}/api/bench`);
    }
    const elapsed = performance.now() - start;
    const rps = (ITERATIONS / elapsed) * 1000;

    console.log(`  Declarative (template): ${ITERATIONS} requests in ${elapsed.toFixed(0)}ms = ${rps.toFixed(0)} req/s`);
    expect(rps).toBeGreaterThan(50);

    await server.close();
  });
});
