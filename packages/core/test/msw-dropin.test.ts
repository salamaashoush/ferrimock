/**
 * MSW drop-in surface: the APIs an unmodified MSW 2.x test suite uses.
 * Everything here goes through the public package entry points
 * (`http`/`graphql`/`HttpResponse`/`setupServer`), not the raw NAPI
 * namespaces.
 */

import { describe, it, expect, afterEach } from "bun:test";
import {
  http,
  graphql,
  HttpResponse,
  FerrimockInterceptor,
  passthrough,
  setupServer,
} from "../src/index.js";

let active: { close(): void } | null = null;

function server(...handlers: Parameters<typeof setupServer>) {
  const s = setupServer(...handlers);
  s.listen({ onUnhandledRequest: "bypass" });
  active = s;
  return s;
}

afterEach(() => {
  active?.close();
  active = null;
});

describe("HttpResponse", () => {
  it("handlers can return HttpResponse.json", async () => {
    server(http.get("/api/user", () => HttpResponse.json({ name: "John" })));
    const res = await fetch("http://localhost/api/user");
    expect(res.headers.get("content-type")).toContain("application/json");
    expect(await res.json()).toEqual({ name: "John" });
  });

  it("new HttpResponse carries status, statusText, and headers", async () => {
    server(
      http.get(
        "/api/raw",
        () =>
          new HttpResponse("teapot", {
            status: 418,
            statusText: "I'm a teapot",
            headers: { "x-custom": "yes" },
          })
      )
    );
    const res = await fetch("http://localhost/api/raw");
    expect(res.status).toBe(418);
    expect(res.statusText).toBe("I'm a teapot");
    expect(res.headers.get("x-custom")).toBe("yes");
    expect(await res.text()).toBe("teapot");
  });

  it("plain Response return values work", async () => {
    server(
      http.get("/api/plain", () => new Response("plain", { status: 201 }))
    );
    const res = await fetch("http://localhost/api/plain");
    expect(res.status).toBe(201);
    expect(await res.text()).toBe("plain");
  });

  it("HttpResponse.error() produces a network error", async () => {
    server(http.get("/api/fail", () => HttpResponse.error()));
    expect(fetch("http://localhost/api/fail")).rejects.toThrow(
      "Failed to fetch"
    );
  });

  it("HttpResponse.redirect is followed by fetch", async () => {
    server(
      http.get("/api/from", () =>
        HttpResponse.redirect("http://localhost/api/to", 302)
      ),
      http.get("/api/to", () => HttpResponse.json({ arrived: true }))
    );
    const res = await fetch("http://localhost/api/from");
    expect(await res.json()).toEqual({ arrived: true });
  });

  it("multiple Set-Cookie headers survive", async () => {
    server(
      http.get("/api/cookies", () => {
        const headers = new Headers();
        headers.append("set-cookie", "a=1; Path=/");
        headers.append("set-cookie", "b=2; Path=/");
        return new HttpResponse(null, { status: 200, headers });
      })
    );
    const res = await fetch("http://localhost/api/cookies");
    expect(res.headers.getSetCookie()).toEqual(["a=1; Path=/", "b=2; Path=/"]);
  });
});

describe("resolver info", () => {
  it("destructures { request, params, cookies, requestId }", async () => {
    server(
      http.post("/api/users/:id", async ({ request, params, cookies, requestId }) => {
        const body = await request.json();
        return HttpResponse.json({
          id: params.id,
          url: request.url,
          method: request.method,
          contentType: request.headers.get("Content-Type"),
          session: cookies.session,
          hasRequestId: typeof requestId === "string" && requestId.length > 0,
          echo: body,
        });
      })
    );

    const res = await fetch("http://localhost/api/users/42", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        cookie: "session=abc123",
      },
      body: JSON.stringify({ hello: "world" }),
    });
    const data = await res.json();
    expect(data.id).toBe("42");
    expect(data.url).toContain("/api/users/42");
    expect(data.method).toBe("POST");
    expect(data.contentType).toBe("application/json");
    expect(data.session).toBe("abc123");
    expect(data.hasRequestId).toBe(true);
    expect(data.echo).toEqual({ hello: "world" });
  });

  it("request.text() and request.clone() work", async () => {
    server(
      http.post("/api/echo", async ({ request }) => {
        const clone = request.clone();
        const text = await clone.text();
        return HttpResponse.text(text.toUpperCase());
      })
    );
    const res = await fetch("http://localhost/api/echo", {
      method: "POST",
      body: "hello",
    });
    expect(await res.text()).toBe("HELLO");
  });
});

describe("fall-through and passthrough", () => {
  it("returning undefined falls through to the next handler", async () => {
    server(
      http.get("/api/data", ({ request }) => {
        if (request.headers.get("x-special") === "1") {
          return HttpResponse.json({ from: "special" });
        }
        return undefined;
      }),
      http.get("/api/data", () => HttpResponse.json({ from: "default" }))
    );

    const special = await fetch("http://localhost/api/data", {
      headers: { "x-special": "1" },
    });
    expect(await special.json()).toEqual({ from: "special" });

    const plain = await fetch("http://localhost/api/data");
    expect(await plain.json()).toEqual({ from: "default" });
  });

  it("passthrough() performs the real request", async () => {
    const upstream = Bun.serve({
      port: 0,
      fetch: () => Response.json({ real: true }),
    });
    try {
      const base = `http://localhost:${upstream.port}`;
      server(
        http.get(`${base}/api/real`, () => passthrough()),
        http.get(`${base}/api/mocked`, () => HttpResponse.json({ real: false }))
      );

      const real = await fetch(`${base}/api/real`);
      expect(await real.json()).toEqual({ real: true });

      const mocked = await fetch(`${base}/api/mocked`);
      expect(await mocked.json()).toEqual({ real: false });
    } finally {
      upstream.stop(true);
    }
  });
});

describe("generator resolvers", () => {
  it("advances per request and repeats the last value", async () => {
    server(
      http.get("/api/poll", function* () {
        yield HttpResponse.json({ status: "pending" });
        yield HttpResponse.json({ status: "running" });
        return HttpResponse.json({ status: "done" });
      })
    );

    const seq: string[] = [];
    for (let i = 0; i < 4; i++) {
      const res = await fetch("http://localhost/api/poll");
      seq.push((await res.json()).status);
    }
    expect(seq).toEqual(["pending", "running", "done", "done"]);
  });
});

describe("absolute URL predicates", () => {
  it("matches host and path from a full URL", async () => {
    server(
      http.get("https://api.example.com/users/:id", ({ params }) =>
        HttpResponse.json({ id: params.id, host: "example" })
      )
    );

    const res = await fetch("https://api.example.com/users/7");
    expect(await res.json()).toEqual({ id: "7", host: "example" });
  });

  it("does not match a different host", async () => {
    let unhandled = 0;
    const s = setupServer(
      http.get("https://api.example.com/users/:id", () =>
        HttpResponse.json({})
      )
    );
    s.listen({ onUnhandledRequest: () => void unhandled++ });
    active = s;

    // Different host, same path — must not match; swallow the network error.
    await fetch("https://api.other.com/users/7").catch(() => {});
    expect(unhandled).toBe(1);
  });
});

describe("graphql", () => {
  const gql = (query: string, variables?: unknown, operationName?: string) =>
    fetch("http://localhost/graphql", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ query, variables, operationName }),
    });

  it("resolver receives { query, variables, operationName }", async () => {
    server(
      graphql.query("GetUser", ({ query, variables, operationName }) =>
        HttpResponse.json({
          data: {
            op: operationName,
            id: variables.id,
            hasQuery: typeof query === "string",
          },
        })
      )
    );

    const res = await gql("query GetUser($id: ID!) { user(id: $id) { id } }", {
      id: "u1",
    });
    expect((await res.json()).data).toEqual({
      op: "GetUser",
      id: "u1",
      hasQuery: true,
    });
  });

  it("matches operation name declared in the document (no operationName field)", async () => {
    server(
      graphql.query("GetTodos", () =>
        HttpResponse.json({ data: { todos: [] } })
      )
    );
    const res = await gql("query GetTodos { todos { id } }");
    expect((await res.json()).data).toEqual({ todos: [] });
  });

  it("accepts RegExp operation names", async () => {
    server(
      graphql.query(/^Get/, ({ operationName }) =>
        HttpResponse.json({ data: { matched: operationName } })
      )
    );
    const res = await gql("query GetThing { thing }");
    expect((await res.json()).data).toEqual({ matched: "GetThing" });
  });

  it("graphql.link scopes handlers to an endpoint", async () => {
    const github = graphql.link("https://api.github.com/graphql");
    server(
      github.query("GetRepo", () =>
        HttpResponse.json({ data: { source: "github" } })
      ),
      graphql.query("GetRepo", () =>
        HttpResponse.json({ data: { source: "generic" } })
      )
    );

    const scoped = await fetch("https://api.github.com/graphql", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ query: "query GetRepo { repo }" }),
    });
    expect((await scoped.json()).data).toEqual({ source: "github" });

    const generic = await gql("query GetRepo { repo }");
    expect((await generic.json()).data).toEqual({ source: "generic" });
  });
});

describe("setupServer lifecycle", () => {
  it("use() overrides and resetHandlers() restores", async () => {
    const s = server(
      http.get("/api/value", () => HttpResponse.json({ v: "initial" }))
    );

    s.use(http.get("/api/value", () => HttpResponse.json({ v: "override" })));
    let res = await fetch("http://localhost/api/value");
    expect(await res.json()).toEqual({ v: "override" });

    s.resetHandlers();
    res = await fetch("http://localhost/api/value");
    expect(await res.json()).toEqual({ v: "initial" });
  });

  it("events use the same requestId as the resolver", async () => {
    const eventIds: string[] = [];
    let resolverId = "";
    const s = server(
      http.get("/api/id", ({ requestId }) => {
        resolverId = requestId;
        return HttpResponse.json({});
      })
    );
    s.events.on("request:start", ({ requestId }) => {
      eventIds.push(requestId);
    });

    await fetch("http://localhost/api/id");
    expect(eventIds).toHaveLength(1);
    expect(resolverId).toBe(eventIds[0]);
  });

  it("unhandledException event fires and the response is a 500", async () => {
    const errors: Error[] = [];
    const s = server(
      http.get("/api/boom", () => {
        throw new Error("resolver exploded");
      })
    );
    s.events.on("unhandledException", ({ error }) => {
      errors.push(error);
    });

    const res = await fetch("http://localhost/api/boom");
    expect(res.status).toBe(500);
    expect(errors).toHaveLength(1);
  });
});

describe("interceptor boundary with async callbacks", () => {
  it("keeps boundary handlers alive until the async callback settles", async () => {
    const interceptor = new FerrimockInterceptor();
    interceptor.apply();
    try {
      const run = interceptor.boundary(async () => {
        interceptor.use(
          http.get("/api/scoped", () => HttpResponse.json({ scoped: true }))
        );
        const res = await fetch("http://localhost/api/scoped");
        return res.json();
      });

      const result = await run();
      expect(result).toEqual({ scoped: true });
      expect(interceptor.mockCount).toBe(0);
    } finally {
      interceptor.dispose();
    }
  });
});

describe("ReadableStream response bodies", () => {
  it("delivers the handler's stream to fetch untouched", async () => {
    const encoder = new TextEncoder();
    server(
      http.get("/api/stream", () => {
        const stream = new ReadableStream({
          start(controller) {
            controller.enqueue(encoder.encode("first "));
            controller.enqueue(encoder.encode("second"));
            controller.close();
          },
        });
        return new HttpResponse(stream, {
          headers: { "content-type": "text/plain" },
        });
      })
    );

    const res = await fetch("http://localhost/api/stream");
    expect(res.body).toBeInstanceOf(ReadableStream);
    expect(await res.text()).toBe("first second");
  });

  it("streams chunks progressively (timed producer)", async () => {
    const encoder = new TextEncoder();
    server(
      http.get("/api/slow-stream", () => {
        const stream = new ReadableStream({
          async start(controller) {
            controller.enqueue(encoder.encode("a"));
            await new Promise((r) => setTimeout(r, 20));
            controller.enqueue(encoder.encode("b"));
            controller.close();
          },
        });
        return new HttpResponse(stream);
      })
    );

    const res = await fetch("http://localhost/api/slow-stream");
    const reader = res.body!.getReader();
    const decoder = new TextDecoder();

    const first = await reader.read();
    const firstAt = performance.now();
    expect(decoder.decode(first.value)).toBe("a");

    const second = await reader.read();
    const secondAt = performance.now();
    expect(decoder.decode(second.value)).toBe("b");
    // The second chunk arrived after the producer's timer, so the body
    // was NOT buffered before delivery.
    expect(secondAt - firstAt).toBeGreaterThan(10);

    const end = await reader.read();
    expect(end.done).toBe(true);
  });
});

describe("form data", () => {
  it("request.formData() parses multipart bodies", async () => {
    server(
      http.post("/api/upload", async ({ request }) => {
        const form = await request.formData();
        const file = form.get("file") as File;
        return HttpResponse.json({
          field: form.get("field"),
          fileName: file.name,
          fileText: await file.text(),
        });
      })
    );

    const form = new FormData();
    form.append("field", "value1");
    form.append("file", new File(["file contents"], "data.txt", { type: "text/plain" }));
    const res = await fetch("http://localhost/api/upload", {
      method: "POST",
      body: form,
    });
    expect(await res.json()).toEqual({
      field: "value1",
      fileName: "data.txt",
      fileText: "file contents",
    });
  });

  it("HttpResponse.formData() returns a parseable multipart response", async () => {
    server(
      http.get("/api/form", () => {
        const form = new FormData();
        form.append("greeting", "hello");
        form.append("file", new File(["file body"], "notes.txt", { type: "text/plain" }));
        return HttpResponse.formData(form);
      })
    );

    const res = await fetch("http://localhost/api/form");
    const form = await res.formData();
    expect(form.get("greeting")).toBe("hello");
    const file = form.get("file") as File;
    expect(file.name).toBe("notes.txt");
    expect(await file.text()).toBe("file body");
  });
});
