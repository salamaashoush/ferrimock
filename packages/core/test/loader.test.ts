import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { MockpitServer } from "@mockpit/node";
import { loadMocksDir } from "@mockpit/core";
import { resolve } from "node:path";

const MOCKS_DIR = resolve(import.meta.dir, "fixtures/mocks");

describe("loadMocksDir - mixed formats", () => {
  let server: MockpitServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new MockpitServer();

    // Load all mocks from directory (YAML + TS handler files)
    const { declarativeCount, handlerCount } = await loadMocksDir(
      server,
      MOCKS_DIR
    );
    expect(declarativeCount).toBe(1); // api.yaml has 1 mock
    expect(handlerCount).toBe(2); // users.ts exports 2 handlers

    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("loads declarative YAML mocks", async () => {
    const res = await fetch(`${baseUrl}/api/hello`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.source).toBe("yaml");
    expect(body.message).toBe("hello from yaml");
  });

  it("loads TS handler mocks from the same directory", async () => {
    const res = await fetch(`${baseUrl}/api/users/42`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.source).toBe("ts-handler");
    expect(body.id).toBe("42");
    expect(body.name).toBe("John");
  });

  it("TS handler POST works", async () => {
    const res = await fetch(`${baseUrl}/api/users`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name: "Alice", email: "alice@test.com" }),
    });
    expect(res.status).toBe(201);
    const body = await res.json();
    expect(body.source).toBe("ts-handler");
    expect(body.name).toBe("Alice");
    expect(body.id).toBe("new-1");
  });

  it("all mocks coexist in the same registry", () => {
    expect(server.mockCount).toBe(3); // 1 yaml + 2 ts handlers
  });
});
