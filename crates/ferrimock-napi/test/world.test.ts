import { describe, it, expect, beforeAll, afterAll } from "bun:test";
import { FerrimockServer, world } from "../index.js";
import { resolve } from "node:path";

const FIXTURES = resolve(import.meta.dir, "fixtures", "world");

/**
 * The entity world from Node.
 *
 * The point of the world being an engine concept rather than something a spec
 * owns: a Node handler reads and writes the same entities a spec-derived route
 * serves. These entity names are prefixed `Napi` because the world is
 * process-global, so a bare `User` would collide with another suite's.
 */
describe("world", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    baseUrl = await server.listen();
    await server.loadMocks(FIXTURES);
  });

  afterAll(async () => {
    world.reset();
    await server.close();
  });

  async function graphql(query: string) {
    const response = await fetch(`${baseUrl}/graphql`, {
      method: "POST",
      headers: { "content-type": "application/json", host: "napi.example.com" },
      body: JSON.stringify({ query }),
    });
    return response.json();
  }

  it("exposes the entities a schema declared", () => {
    expect(world.types()).toContain("NapiUser");
    expect(world.types()).toContain("NapiFolder");
    expect(world.count("NapiUser")).toBe(3);
    expect(world.count("NapiFolder")).toBe(4);
  });

  it("hands back numbers as numbers", () => {
    // `serde_json/arbitrary_precision` is force-enabled workspace-wide; a
    // count that crossed the boundary through the serde data model would
    // arrive as a tagged object instead.
    expect(typeof world.count("NapiUser")).toBe("number");

    const page = world.list("NapiUser");
    expect(typeof page.total).toBe("number");
    expect(page.total + 1).toBe(4);
  });

  it("lists, pages and sorts", () => {
    const all = world.list("NapiUser");
    expect(all.records).toHaveLength(3);
    expect(all.hasNext).toBe(false);

    const page = world.list("NapiUser", { limit: 2 });
    expect(page.records).toHaveLength(2);
    expect(page.total).toBe(3);
    expect(page.hasNext).toBe(true);

    const ascending = world.list("NapiUser", { sort: "name" }).records.map((u) => u.name);
    const descending = world.list("NapiUser", { sort: "-name" }).records.map((u) => u.name);
    expect(descending).toEqual([...ascending].reverse());
  });

  it("filters by field", () => {
    const target = world.list("NapiUser", { limit: 1 }).records[0];
    const found = world.list("NapiUser", { filter: { name: target.name } });
    expect(found.total).toBe(1);
    expect(found.records[0].id).toBe(target.id);
  });

  it("returns undefined for a miss, matching the QuickJS lane", () => {
    expect(world.get("NapiUser", "no-such-key")).toBeUndefined();
  });

  it("follows a relation", () => {
    const folder = world.list("NapiFolder", { limit: 1 }).records[0];
    const owner = world.related("NapiFolder", folder.id, "owner");
    expect(owner.total).toBe(1);
    expect(typeof owner.records[0].name).toBe("string");
  });

  it("creates, updates and deletes", () => {
    const created = world.create("NapiUser", { name: "Ada Lovelace", email: "ada@example.com" });
    expect(created.name).toBe("Ada Lovelace");
    expect(world.count("NapiUser")).toBe(4);
    expect(world.get("NapiUser", created.id).email).toBe("ada@example.com");

    world.update("NapiUser", created.id, { name: "Grace Hopper" });
    const updated = world.get("NapiUser", created.id);
    expect(updated.name).toBe("Grace Hopper");
    expect(updated.email).toBe("ada@example.com");

    world.delete("NapiUser", created.id);
    expect(world.get("NapiUser", created.id)).toBeUndefined();
    expect(world.count("NapiUser")).toBe(3);
  });

  it("fills fields the caller left out", () => {
    const created = world.create("NapiUser", { name: "Partial" });
    // The response validates against the same schema a real one would, so a
    // non-nullable field cannot come back missing.
    expect(typeof created.email).toBe("string");
    expect(created.email.length).toBeGreaterThan(0);
    world.delete("NapiUser", created.id);
  });

  it("reports and drops pending writes", () => {
    world.reset();
    expect(world.pendingWrites()).toBe(0);

    world.create("NapiUser", { name: "Temporary" });
    expect(world.pendingWrites()).toBeGreaterThan(0);
    expect(world.count("NapiUser")).toBe(4);

    world.reset();
    expect(world.pendingWrites()).toBe(0);
    expect(world.count("NapiUser")).toBe(3);
  });

  it("errors on an unknown entity rather than reading as empty", () => {
    expect(() => world.list("NoSuchEntity")).toThrow();
  });

  // The claim the whole design exists for.
  it("shares entities with a spec-derived route", async () => {
    const created = world.create("NapiUser", { name: "Seen By GraphQL", email: "s@example.com" });

    const body = await graphql("{ napiUsers { name } }");
    const names = body.data.napiUsers.map((u: { name: string }) => u.name);
    expect(names).toContain("Seen By GraphQL");

    world.delete("NapiUser", created.id);
    const after = await graphql("{ napiUsers { name } }");
    expect(after.data.napiUsers.map((u: { name: string }) => u.name)).not.toContain(
      "Seen By GraphQL",
    );
  });

  it("sees what a spec-derived mutation wrote", async () => {
    const before = world.count("NapiFolder");
    const body = await graphql('mutation { createNapiFolder(name: "From GraphQL") { id name } }');

    const id = body.data.createNapiFolder.id;
    expect(world.count("NapiFolder")).toBe(before + 1);
    expect(world.get("NapiFolder", id).name).toBe("From GraphQL");

    world.delete("NapiFolder", id);
  });
});
