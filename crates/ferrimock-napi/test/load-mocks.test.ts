import { describe, it, expect, beforeAll, afterAll, beforeEach } from "bun:test";
import { FerrimockServer, http, HttpResponse } from "../index.js";
import { resolve } from "node:path";

const FIXTURES = resolve(import.meta.dir, "fixtures");

describe("loadMockFile - YAML", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("loads mocks from a .yaml file", async () => {
    const count = await server.loadMockFile(resolve(FIXTURES, "mocks.yaml"));
    expect(count).toBe(2);
    expect(server.mockCount).toBe(2);

    // Test the first mock
    const res = await fetch(`${baseUrl}/api/yaml/users`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.source).toBe("yaml");
    expect(body.users).toHaveLength(1);
    expect(body.users[0].name).toBe("Alice");

    // Test the second mock
    const health = await fetch(`${baseUrl}/api/yaml/health`);
    expect(health.status).toBe(200);
    expect(await health.text()).toBe("ok");
  });
});

describe("loadMockFile - JSON", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("loads mocks from a .json file", async () => {
    const count = await server.loadMockFile(resolve(FIXTURES, "mocks.json"));
    expect(count).toBe(2);

    const res = await fetch(`${baseUrl}/api/json/products`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.source).toBe("json");
    expect(body.products[0].name).toBe("Widget");

    const status = await fetch(`${baseUrl}/api/json/status`);
    expect(await status.text()).toBe("running");
  });
});

describe("loadMockFile - HAR", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("loads mocks from a .har file", async () => {
    const count = await server.loadMockFile(resolve(FIXTURES, "recording.har"));
    expect(count).toBeGreaterThanOrEqual(1);

    // HAR records the full URL but ferrimock extracts the path
    const res = await fetch(`${baseUrl}/api/har/data`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.source).toBe("har");
    expect(body.data).toBe(true);
  });
});

describe("loadMocks - directory with mixed formats", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("loads all files from a directory (YAML + JSON)", async () => {
    const count = await server.loadMocks(resolve(FIXTURES, "mocks-dir"));
    expect(count).toBe(2); // 1 from api.yaml + 1 from extra.json

    // YAML mock
    const yamlRes = await fetch(`${baseUrl}/api/dir/users`);
    expect(yamlRes.status).toBe(200);
    const yamlBody = await yamlRes.json();
    expect(yamlBody.source).toBe("dir-yaml");

    // JSON mock
    const jsonRes = await fetch(`${baseUrl}/api/dir/products`);
    expect(jsonRes.status).toBe(200);
    const jsonBody = await jsonRes.json();
    expect(jsonBody.source).toBe("dir-json");
  });
});

describe("Mixed: file mocks + handler mocks coexist", () => {
  let server: FerrimockServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new FerrimockServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("file mocks and handler mocks live in the same registry", async () => {
    // Load file-based mocks
    await server.loadMockFile(resolve(FIXTURES, "mocks.yaml"));

    // Add handler-based mocks
    server.useHandlers([
      http.get("/api/handler/data", async () => {
        return HttpResponse.json({ source: "handler" });
      }),
    ]);

    expect(server.mockCount).toBe(3); // 2 from yaml + 1 handler

    // File-based mock works
    const yamlRes = await fetch(`${baseUrl}/api/yaml/users`);
    expect(yamlRes.status).toBe(200);
    expect((await yamlRes.json()).source).toBe("yaml");

    // Handler-based mock works
    const handlerRes = await fetch(`${baseUrl}/api/handler/data`);
    expect(handlerRes.status).toBe(200);
    expect((await handlerRes.json()).source).toBe("handler");

    // resetHandlers only removes handler mocks
    server.resetHandlers();
    expect(server.mockCount).toBe(2); // yaml mocks remain

    // YAML still works
    const stillYaml = await fetch(`${baseUrl}/api/yaml/users`);
    expect(stillYaml.status).toBe(200);

    // Handler is gone
    const noHandler = await fetch(`${baseUrl}/api/handler/data`);
    expect(noHandler.status).toBe(404);
  });
});
