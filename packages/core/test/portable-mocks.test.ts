/**
 * The portability contract: the same .mjs mock files the QuickJS CLI
 * serves also load under Node via loadMocksDir -- both the bare-call
 * (side-effect) style and the export-default style, importing from the
 * bare 'ferrimock' specifier.
 */
import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { FerrimockServer } from "@ferrimock/node";
import { loadMocksDir } from "../src/index.js";
import { resolve } from "node:path";

const PORTABLE_DIR = resolve(import.meta.dir, "fixtures/portable");

describe("portable mock files (QuickJS <-> Node)", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    const { handlerCount } = await loadMocksDir(server, PORTABLE_DIR);
    expect(handlerCount).toBe(3); // 2 bare-call + 1 export-default
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("bare-call registrations are collected and stateful", async () => {
    const first = await fetch(`${baseUrl}/api/count`, { method: "POST" });
    expect(first.status).toBe(200);
    expect((await first.json()).count).toBe(1);

    const second = await fetch(`${baseUrl}/api/count`, { method: "POST" });
    expect((await second.json()).count).toBe(2);
  });

  it("fake data works through the bare specifier", async () => {
    const res = await fetch(`${baseUrl}/api/id`);
    const body = await res.json();
    expect(body.id).toHaveLength(36);
  });

  it("export-default style still works", async () => {
    const res = await fetch(`${baseUrl}/api/exported`);
    expect((await res.json()).style).toBe("export-default");
  });
});
