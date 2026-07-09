/**
 * Coverage for the MSW-compat surface gaps found in the July 2026 audit:
 * graphql.operation, wildcard params, bypass(), async generators,
 * urlencoded request bodies, binary request bodies, and the
 * matchRequestUrl/cleanUrl utilities.
 */

import { describe, it, expect, afterEach } from "bun:test";
import {
  http,
  graphql,
  HttpResponse,
  setupServer,
  bypass,
  cleanUrl,
  matchRequestUrl,
} from "../src/index.js";

let active: { close(): void } | null = null;
let realServer: ReturnType<typeof Bun.serve> | null = null;

function server(...handlers: Parameters<typeof setupServer>) {
  const s = setupServer(...handlers);
  s.listen({ onUnhandledRequest: "bypass" });
  active = s;
  return s;
}

afterEach(() => {
  active?.close();
  active = null;
  realServer?.stop();
  realServer = null;
});

describe("graphql.operation", () => {
  it("matches any operation and exposes operationName/variables", async () => {
    server(
      graphql.operation(({ operationName, variables }) =>
        HttpResponse.json({ data: { op: operationName, vars: variables } })
      )
    );

    const res = await fetch("http://localhost/graphql", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        query: "query Anything { id }",
        variables: { x: 1 },
      }),
    });
    const json = (await res.json()) as any;
    expect(json.data.op).toBe("Anything");
    expect(json.data.vars).toEqual({ x: 1 });
  });
});

describe("wildcard paths", () => {
  it("captures a trailing * into params['0']", async () => {
    server(
      http.get("/files/*", ({ params }) =>
        HttpResponse.json({ splat: (params as any)["0"] })
      )
    );

    const res = await fetch("http://localhost/files/img/logo.png");
    expect(res.status).toBe(200);
    const json = (await res.json()) as any;
    expect(json.splat).toBe("img/logo.png");
  });

  it("captures multiple * segments positionally", async () => {
    server(
      http.get("/a/*/b/*", ({ params }) =>
        HttpResponse.json({
          first: (params as any)["0"],
          second: (params as any)["1"],
        })
      )
    );

    const res = await fetch("http://localhost/a/x/b/y/z");
    const json = (await res.json()) as any;
    expect(json.first).toBe("x");
    expect(json.second).toBe("y/z");
  });
});

describe("bypass()", () => {
  it("performs a real network request from inside a handler", async () => {
    realServer = Bun.serve({
      port: 0,
      fetch() {
        return new Response(JSON.stringify({ from: "network" }), {
          headers: { "content-type": "application/json" },
        });
      },
    });
    const base = `http://127.0.0.1:${realServer.port}`;

    server(
      http.get(`${base}/real`, () => HttpResponse.json({ from: "mock" })),
      http.get("/proxy", async () => {
        const real = await fetch(bypass(`${base}/real`));
        const data = (await real.json()) as any;
        return HttpResponse.json({ ...data, proxied: true });
      })
    );

    const res = await fetch("http://localhost/proxy");
    const json = (await res.json()) as any;
    // bypass() skipped the mock for /real and hit the network.
    expect(json.from).toBe("network");
    expect(json.proxied).toBe(true);
  });
});

describe("async generators", () => {
  it("advances an async generator per request and repeats the last value", async () => {
    server(
      http.get("/gen", async function* () {
        yield HttpResponse.json({ n: 1 });
        yield HttpResponse.json({ n: 2 });
        return HttpResponse.json({ n: 3 });
      })
    );

    const results: number[] = [];
    for (let i = 0; i < 4; i++) {
      const res = await fetch("http://localhost/gen");
      results.push(((await res.json()) as any).n);
    }
    expect(results).toEqual([1, 2, 3, 3]);
  });
});

describe("urlencoded request bodies", () => {
  it("parses application/x-www-form-urlencoded via request.formData()", async () => {
    server(
      http.post("/form", async ({ request }) => {
        const form = await request.formData();
        return HttpResponse.json({
          name: form.get("name"),
          tags: form.getAll("tag"),
        });
      })
    );

    const res = await fetch("http://localhost/form", {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: "name=box%20unit&tag=a&tag=b",
    });
    const json = (await res.json()) as any;
    expect(json.name).toBe("box unit");
    expect(json.tags).toEqual(["a", "b"]);
  });
});

describe("binary request bodies", () => {
  it("delivers non-UTF8 bytes to the resolver intact", async () => {
    const payload = new Uint8Array([0xff, 0x00, 0x88, 0xfe, 0x01]);
    server(
      http.post("/upload", async ({ request }) => {
        const bytes = new Uint8Array(await request.arrayBuffer());
        return HttpResponse.json({
          len: bytes.length,
          first: bytes[0],
          last: bytes[bytes.length - 1],
        });
      })
    );

    const res = await fetch("http://localhost/upload", {
      method: "POST",
      body: payload,
    });
    const json = (await res.json()) as any;
    expect(json.len).toBe(5);
    expect(json.first).toBe(255);
    expect(json.last).toBe(1);
  });
});

describe("virtual cookie jar", () => {
  it("stores mocked Set-Cookie and replays it on later requests", async () => {
    server(
      http.post("http://jar.test/login", () =>
        HttpResponse.json(
          { ok: true },
          { headers: { "set-cookie": "session=s3cret" } }
        )
      ),
      http.get("http://jar.test/me", ({ cookies }) =>
        HttpResponse.json({ cookies })
      )
    );

    await fetch("http://jar.test/login", { method: "POST" });
    const res = await fetch("http://jar.test/me");
    const json = (await res.json()) as any;
    expect(json.cookies.session).toBe("s3cret");

    // A cookie the request itself sends wins over the stored one.
    const res2 = await fetch("http://jar.test/me", {
      headers: { cookie: "session=mine" },
    });
    expect(((await res2.json()) as any).cookies.session).toBe("mine");
  });
});

describe("ClientRequest lifecycle events", () => {
  const isBun = typeof (globalThis as any).Bun !== "undefined";

  // Bun's node:http is not patchable by @mswjs/interceptors; verified
  // under real Node via `bun run verify:node-http`.
  it.skipIf(isBun)(
    "emits request:start/match and response:mocked on the node:http path",
    async () => {
      const nodeHttp = await import("node:http");
      const s = server(
        http.get("http://events.test/hit", () => HttpResponse.json({ ok: true }))
      );

      const seen: string[] = [];
      s.events.on("request:start", () => seen.push("start"));
      s.events.on("request:match", () => seen.push("match"));
      s.events.on("response:mocked", () => seen.push("mocked"));
      s.events.on("request:end", () => seen.push("end"));

      await new Promise<void>((resolve, reject) => {
        const req = nodeHttp.request("http://events.test/hit", (res) => {
          res.resume();
          res.on("end", resolve);
        });
        req.on("error", reject);
        req.end();
      });

      expect(seen).toContain("start");
      expect(seen).toContain("match");
      expect(seen).toContain("mocked");
      expect(seen).toContain("end");
    }
  );
});

describe("matchRequestUrl / cleanUrl", () => {
  it("cleanUrl strips query and hash", () => {
    expect(cleanUrl("/user?id=1")).toBe("/user");
    expect(cleanUrl("/user#top")).toBe("/user");
    expect(cleanUrl("/user")).toBe("/user");
  });

  it("matches path params", () => {
    const result = matchRequestUrl(
      new URL("http://localhost/users/42"),
      "/users/:id"
    );
    expect(result.matches).toBe(true);
    expect(result.params).toEqual({ id: "42" });
  });

  it("matches wildcards into numeric params", () => {
    const result = matchRequestUrl(
      new URL("http://localhost/files/a/b.png"),
      "/files/*"
    );
    expect(result.matches).toBe(true);
    expect(result.params).toEqual({ "0": "a/b.png" });
  });

  it("matches repeatable params into string arrays", () => {
    const plus = matchRequestUrl(
      new URL("http://localhost/files/a%20b/c"),
      "/files/:path+"
    );
    expect(plus.matches).toBe(true);
    expect(plus.params).toEqual({ path: ["a b", "c"] });

    const zero = matchRequestUrl(new URL("http://localhost/tree"), "/tree/:path*");
    expect(zero.matches).toBe(true);
    expect(zero.params).toEqual({});

    const miss = matchRequestUrl(new URL("http://localhost/files"), "/files/:path+");
    expect(miss.matches).toBe(false);
  });

  it("honors absolute-URL hosts", () => {
    const hit = matchRequestUrl(
      new URL("https://api.example.com/users/1"),
      "https://api.example.com/users/:id"
    );
    expect(hit.matches).toBe(true);
    expect(hit.params).toEqual({ id: "1" });

    const miss = matchRequestUrl(
      new URL("https://other.example.com/users/1"),
      "https://api.example.com/users/:id"
    );
    expect(miss.matches).toBe(false);
  });

  it("supports RegExp paths against the pathname", () => {
    const result = matchRequestUrl(
      new URL("http://localhost/v2/items"),
      /^\/v(?<version>\d+)\//
    );
    expect(result.matches).toBe(true);
    expect(result.params).toEqual({ version: "2" });
  });
});
