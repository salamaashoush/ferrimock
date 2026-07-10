import { describe, it, expect } from "bun:test";
import { services } from "../index.js";
import { resolve } from "node:path";
import { writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";

const FIXTURES = resolve(import.meta.dir, "fixtures");

describe("services.validate", () => {
  it("validates a valid YAML mock file", async () => {
    const result = await services.validate({
      path: resolve(FIXTURES, "mocks.yaml"),
    });
    expect(result.isValid).toBe(true);
    expect(result.totalErrors).toBe(0);
  });

  it("validates a directory of mock files", async () => {
    const result = await services.validate({
      path: resolve(FIXTURES, "mocks-dir"),
    });
    expect(result.isValid).toBe(true);
  });
});

describe("services.list", () => {
  it("lists mocks from a directory", async () => {
    const result = await services.list({
      mocksDir: resolve(FIXTURES, "mocks-dir"),
    });
    expect(result.total).toBe(2);
    expect(result.mocks.length).toBe(2);
    expect(result.mocks[0].id).toBeDefined();
    expect(result.mocks[0].methods).toBeDefined();
  });

  it("filters mocks by ID", async () => {
    const result = await services.list({
      mocksDir: resolve(FIXTURES, "mocks-dir"),
      filter: "dir-users",
    });
    expect(result.total).toBe(1);
    expect(result.mocks[0].id).toContain("dir-users");
  });
});

describe("services.create", () => {
  it("creates a mock definition in YAML", () => {
    const result = services.create({
      url: "/api/users/:id",
      method: "GET",
      status: 200,
      format: "yaml",
    });
    expect(result.mockId).toBe("get-api-users-id");
    expect(result.content).toContain("url: /api/users/:id");
    expect(result.content).toContain("method: GET");
  });

  it("creates a mock with template", () => {
    const result = services.create({
      url: "/api/users",
      method: "GET",
      template: true,
      format: "json",
    });
    expect(result.content).toContain("fake_uuid");
    expect(result.content).toContain("fake_name");
  });
});

describe("services.format", () => {
  it("formats mock files in check mode", () => {
    const result = services.format({
      path: resolve(FIXTURES, "mocks-dir"),
      check: true,
    });
    expect(result.errorCount).toBe(0);
    // Files may or may not be formatted already
    expect(result.files.length).toBeGreaterThan(0);
  });
});

describe("services.fakeData", () => {
  it("generates fake emails", () => {
    const values = services.fakeData({
      generator: "email",
      count: 3,
    });
    expect(values.length).toBe(3);
    for (const v of values) {
      expect(v).toContain("@");
    }
  });

  it("generates fake UUIDs", () => {
    const values = services.fakeData({ generator: "uuid", count: 2 });
    expect(values.length).toBe(2);
    for (const v of values) {
      expect(v).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
      );
    }
  });

  it("generates fake numbers with range", () => {
    const values = services.fakeData({
      generator: "number",
      count: 5,
      min: 1,
      max: 10,
    });
    expect(values.length).toBe(5);
    for (const v of values) {
      const n = parseInt(v, 10);
      expect(n).toBeGreaterThanOrEqual(1);
      expect(n).toBeLessThanOrEqual(10);
    }
  });

  it("throws on unknown generator", () => {
    expect(() =>
      services.fakeData({ generator: "nonexistent" })
    ).toThrow("Unknown generator");
  });
});

describe("services.listGenerators", () => {
  it("lists all generators", () => {
    const generators = services.listGenerators();
    expect(generators.length).toBeGreaterThan(10);
    expect(generators[0].name).toBeDefined();
    expect(generators[0].category).toBeDefined();
  });

  it("filters by category", () => {
    const generators = services.listGenerators("Identity");
    expect(generators.length).toBeGreaterThan(0);
    for (const g of generators) {
      expect(g.category).toBe("Identity");
    }
  });
});

describe("services.fakeImage", () => {
  it("generates a placeholder image", () => {
    const result = services.fakeImage({
      imageType: "placeholder",
      width: 100,
      height: 100,
    });
    expect(result.base64.length).toBeGreaterThan(100);
    expect(result.mimeType).toBe("image/png");
  });
});

describe("services.fakePdf", () => {
  it("generates a PDF", () => {
    const result = services.fakePdf({ pages: 2, text: "Hello PDF" });
    expect(result.base64.length).toBeGreaterThan(100);
  });
});

describe("services.renderTemplate", () => {
  it("renders a template with fake data", () => {
    const results = services.renderTemplate({
      template: '{"name": "{{ fake_name() }}", "id": "{{ fake_uuid() }}"}',
      count: 2,
    });
    expect(results.length).toBe(2);
    for (const r of results) {
      const parsed = JSON.parse(r);
      expect(parsed.name).toBeDefined();
      expect(parsed.id).toMatch(/^[0-9a-f-]+$/);
    }
  });

  it("renders with context", () => {
    const results = services.renderTemplate({
      template: "Hello {{ captures.name }}!",
      context: { captures: { name: "World" } },
    });
    expect(results[0]).toBe("Hello World!");
  });
});

describe("services.testMatch", () => {
  it("tests a request against mock files", async () => {
    const result = await services.testMatch({
      method: "GET",
      path: "/api/dir/users",
      mocksDir: resolve(FIXTURES, "mocks-dir"),
    });
    expect(result.matched).toBe(true);
    expect(result.mock_id).toContain("dir-users");
  });

  it("returns no match for unknown paths", async () => {
    const result = await services.testMatch({
      method: "GET",
      path: "/nonexistent",
      mocksDir: resolve(FIXTURES, "mocks-dir"),
    });
    expect(result.matched).toBe(false);
  });
});

describe("services.show", () => {
  it("shows a mock by ID", async () => {
    const mock = await services.show("dir-users", resolve(FIXTURES, "mocks-dir"));
    expect(mock).not.toBeNull();
    expect(mock!.id).toContain("dir-users");
  });

  it("returns null for unknown ID", async () => {
    const mock = await services.show("nonexistent", resolve(FIXTURES, "mocks-dir"));
    expect(mock).toBeNull();
  });
});
