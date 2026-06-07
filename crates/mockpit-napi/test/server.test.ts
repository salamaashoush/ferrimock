import { describe, it, expect, beforeAll, afterAll, beforeEach } from "bun:test";
import {
  MockpitServer,
  http,
  graphql,
  MockResponse,
  type MockpitRequest,
} from "../index.js";

describe("MockpitServer", () => {
  let server: MockpitServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new MockpitServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  beforeEach(() => {
    server.resetHandlers();
  });

  // ---- Basic lifecycle ----

  it("starts and reports running state", () => {
    expect(server.isRunning).toBe(true);
    expect(server.port).toBeGreaterThan(0);
    expect(baseUrl).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
  });

  it("reports mock count", () => {
    expect(server.mockCount).toBe(0);
  });

  // ---- http.get handler ----

  it("handles GET with JSON response", async () => {
    server.useHandlers([
      http.get("/api/hello", async () => {
        return MockResponse.json({ message: "Hello, World!" });
      }),
    ]);

    expect(server.mockCount).toBe(1);

    const res = await fetch(`${baseUrl}/api/hello`);
    expect(res.status).toBe(200);
    expect(res.headers.get("content-type")).toBe("application/json");

    const body = await res.json();
    expect(body.message).toBe("Hello, World!");
  });

  // ---- Path params ----

  it("extracts :param captures", async () => {
    server.useHandlers([
      http.get("/users/:id", async (ctx: MockpitRequest) => {
        return MockResponse.json({ userId: ctx.params.id });
      }),
    ]);

    const res = await fetch(`${baseUrl}/users/42`);
    expect(res.status).toBe(200);

    const body = await res.json();
    expect(body.userId).toBe("42");
  });

  it("extracts multiple :param captures", async () => {
    server.useHandlers([
      http.get(
        "/users/:userId/posts/:postId",
        async (ctx: MockpitRequest) => {
          return MockResponse.json({
            userId: ctx.params.userId,
            postId: ctx.params.postId,
          });
        }
      ),
    ]);

    const res = await fetch(`${baseUrl}/users/7/posts/99`);
    const body = await res.json();
    expect(body.userId).toBe("7");
    expect(body.postId).toBe("99");
  });

  // ---- http.post with request body ----

  it("handles POST with JSON request body", async () => {
    server.useHandlers([
      http.post("/api/login", async (ctx: MockpitRequest) => {
        if (ctx.bodyJson?.username === "admin") {
          return MockResponse.json({ token: "secret-token" });
        }
        return MockResponse.json({ error: "Forbidden" }, { status: 403 });
      }),
    ]);

    // Success case
    const res1 = await fetch(`${baseUrl}/api/login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: "admin", password: "pass" }),
    });
    expect(res1.status).toBe(200);
    const body1 = await res1.json();
    expect(body1.token).toBe("secret-token");

    // Failure case
    const res2 = await fetch(`${baseUrl}/api/login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: "wrong" }),
    });
    expect(res2.status).toBe(403);
    const body2 = await res2.json();
    expect(body2.error).toBe("Forbidden");
  });

  // ---- Other HTTP methods ----

  it("handles PUT", async () => {
    server.useHandlers([
      http.put("/api/items/:id", async (ctx: MockpitRequest) => {
        return MockResponse.json({ updated: ctx.params.id });
      }),
    ]);

    const res = await fetch(`${baseUrl}/api/items/5`, { method: "PUT" });
    const body = await res.json();
    expect(body.updated).toBe("5");
  });

  it("handles DELETE", async () => {
    server.useHandlers([
      http.delete("/api/items/:id", async (ctx: MockpitRequest) => {
        return MockResponse.empty(204);
      }),
    ]);

    const res = await fetch(`${baseUrl}/api/items/5`, { method: "DELETE" });
    expect(res.status).toBe(204);
  });

  it("handles PATCH", async () => {
    server.useHandlers([
      http.patch("/api/items/:id", async (ctx: MockpitRequest) => {
        return MockResponse.json({ patched: true });
      }),
    ]);

    const res = await fetch(`${baseUrl}/api/items/5`, { method: "PATCH" });
    const body = await res.json();
    expect(body.patched).toBe(true);
  });

  // ---- http.all (any method) ----

  it("handles any method with http.all", async () => {
    server.useHandlers([
      http.all("/api/any", async (ctx: MockpitRequest) => {
        return MockResponse.json({ method: ctx.method });
      }),
    ]);

    const get = await (await fetch(`${baseUrl}/api/any`)).json();
    expect(get.method).toBe("GET");

    const post = await (
      await fetch(`${baseUrl}/api/any`, { method: "POST" })
    ).json();
    expect(post.method).toBe("POST");
  });

  // ---- MockResponse variants ----

  it("returns text response", async () => {
    server.useHandlers([
      http.get("/text", async () => MockResponse.text("Hello plain text")),
    ]);

    const res = await fetch(`${baseUrl}/text`);
    expect(res.headers.get("content-type")).toBe("text/plain");
    expect(await res.text()).toBe("Hello plain text");
  });

  it("returns HTML response", async () => {
    server.useHandlers([
      http.get("/html", async () =>
        MockResponse.html("<h1>Hello HTML</h1>")
      ),
    ]);

    const res = await fetch(`${baseUrl}/html`);
    expect(res.headers.get("content-type")).toBe("text/html");
    expect(await res.text()).toBe("<h1>Hello HTML</h1>");
  });

  it("returns custom status and headers", async () => {
    server.useHandlers([
      http.get("/custom", async () =>
        MockResponse.json({ created: true }, { status: 201, headers: { "x-request-id": "abc" } })
      ),
    ]);

    const res = await fetch(`${baseUrl}/custom`);
    expect(res.status).toBe(201);
    expect(res.headers.get("x-request-id")).toBe("abc");
  });

  // ---- 404 for unmatched routes ----

  it("returns 404 for unmatched routes", async () => {
    const res = await fetch(`${baseUrl}/nonexistent`);
    expect(res.status).toBe(404);
    const body = await res.json();
    expect(body.error).toBe("No matching mock found");
  });

  // ---- resetHandlers ----

  it("resetHandlers removes handler-based mocks", async () => {
    server.useHandlers([
      http.get("/temp", async () => MockResponse.json({ temp: true })),
    ]);

    // Should work
    let res = await fetch(`${baseUrl}/temp`);
    expect(res.status).toBe(200);

    // Reset
    server.resetHandlers();
    expect(server.mockCount).toBe(0);

    // Should 404 now
    res = await fetch(`${baseUrl}/temp`);
    expect(res.status).toBe(404);
  });

  // ---- Multiple handlers with priority ----

  it("both overlapping handlers are registered and one wins", async () => {
    server.useHandlers([
      http.get("/overlap", async () => MockResponse.json({ handler: "a" })),
      http.get("/overlap", async () => MockResponse.json({ handler: "b" })),
    ]);

    const body = await (await fetch(`${baseUrl}/overlap`)).json();
    // Both have same priority, one of them wins deterministically
    expect(body.handler).toBeDefined();
    expect(["a", "b"]).toContain(body.handler);
  });

  // ---- Request context fields ----

  it("provides full request context to handler", async () => {
    server.useHandlers([
      http.post("/ctx-test/:id", async (ctx: MockpitRequest) => {
        return MockResponse.json({
          method: ctx.method,
          path: ctx.path,
          uri: ctx.uri,
          params: ctx.params,
          query: ctx.query,
          headersSent: !!ctx.headers["content-type"],
          hasBody: !!ctx.body,
          hasBodyJson: !!ctx.bodyJson,
        });
      }),
    ]);

    const res = await fetch(`${baseUrl}/ctx-test/abc?foo=bar&baz=qux`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ key: "value" }),
    });

    const body = await res.json();
    expect(body.method).toBe("POST");
    expect(body.path).toBe("/ctx-test/abc");
    expect(body.params.id).toBe("abc");
    expect(body.query.foo).toBe("bar");
    expect(body.query.baz).toBe("qux");
    expect(body.headersSent).toBe(true);
    expect(body.hasBody).toBe(true);
    expect(body.hasBodyJson).toBe(true);
  });
});

describe("GraphQL handlers", () => {
  let server: MockpitServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new MockpitServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  beforeEach(() => {
    server.resetHandlers();
  });

  it("matches GraphQL query by operation name", async () => {
    server.useHandlers([
      graphql.query("GetUser", async (ctx: MockpitRequest) => {
        const variables = ctx.bodyJson?.variables;
        return MockResponse.json({
          data: {
            user: { id: variables?.id ?? "unknown", name: "Test User" },
          },
        });
      }),
    ]);

    const res = await fetch(baseUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        query: "query GetUser($id: ID!) { user(id: $id) { id name } }",
        operationName: "GetUser",
        variables: { id: "123" },
      }),
    });

    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.data.user.id).toBe("123");
    expect(body.data.user.name).toBe("Test User");
  });

  it("matches GraphQL mutation by operation name", async () => {
    server.useHandlers([
      graphql.mutation("CreateUser", async () => {
        return MockResponse.json({
          data: { createUser: { id: "new-1", success: true } },
        });
      }),
    ]);

    const res = await fetch(baseUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        query: "mutation CreateUser($input: CreateUserInput!) { createUser(input: $input) { id success } }",
        operationName: "CreateUser",
        variables: { input: { name: "New User" } },
      }),
    });

    const body = await res.json();
    expect(body.data.createUser.success).toBe(true);
  });
});

describe("Declarative mock via addMock", () => {
  let server: MockpitServer;
  let baseUrl: string;

  beforeAll(async () => {
    server = new MockpitServer();
    baseUrl = await server.listen();
  });

  afterAll(async () => {
    await server.close();
  });

  it("adds a declarative mock via JSON config", async () => {
    const mockId = await server.addMock({
      id: "declarative-test",
      match: {
        method: "GET",
        url: "/api/declarative",
      },
      response: {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"source":"declarative"}',
      },
    });

    expect(mockId).toBe("declarative-test");

    const res = await fetch(`${baseUrl}/api/declarative`);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.source).toBe("declarative");
  });

  it("removes a mock by ID", async () => {
    // Re-add the mock first (previous test's mock is still there)
    await server.addMock({
      id: "to-remove",
      match: { method: "GET", url: "/api/to-remove" },
      response: { status: 200, body: "ok" },
    });

    const removed = server.removeMock("to-remove");
    expect(removed).toBe(true);

    const res = await fetch(`${baseUrl}/api/to-remove`);
    expect(res.status).toBe(404);
  });
});
