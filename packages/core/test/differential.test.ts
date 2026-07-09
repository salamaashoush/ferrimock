/**
 * Differential suite: the same handlers registered with real msw and
 * with mockpit, driven by identical requests, must produce equivalent
 * responses. Guards drop-in compatibility with observed MSW behavior
 * rather than our reading of its docs.
 *
 * Each scenario runs against one library at a time (both patch global
 * fetch, so servers are started and closed sequentially).
 */

import { describe, it, expect } from "bun:test";
import { http as mswHttp, graphql as mswGraphql, HttpResponse as MswHttpResponse } from "msw";
import { setupServer as mswSetupServer } from "msw/node";
import {
  http as pitHttp,
  graphql as pitGraphql,
  HttpResponse as PitHttpResponse,
  setupServer as pitSetupServer,
} from "../src/index.js";

type Lib = {
  http: typeof mswHttp;
  graphql: typeof mswGraphql;
  HttpResponse: typeof MswHttpResponse;
};

type Scenario = {
  name: string;
  handlers: (lib: Lib) => any[];
  exec: () => Promise<unknown>;
};

async function normalized(res: Response): Promise<unknown> {
  const contentType = res.headers.get("content-type");
  const text = await res.text();
  // Parse JSON bodies so object key order (hash-map vs insertion) does
  // not fail the comparison.
  let body: unknown = text;
  if (contentType?.includes("json")) {
    try {
      body = JSON.parse(text);
    } catch {}
  }
  return { status: res.status, contentType, body };
}

const scenarios: Scenario[] = [
  {
    name: "json static with status and header",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/api/thing", () =>
        HttpResponse.json(
          { id: 1, tags: ["a", "b"] },
          { status: 201, headers: { "x-custom": "yes" } }
        )
      ),
    ],
    exec: async () => {
      const res = await fetch("http://mocked.test/api/thing");
      return {
        ...(await normalized(res)) as object,
        custom: res.headers.get("x-custom"),
      };
    },
  },
  {
    name: "path params and query",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/users/:userId/posts/:postId", ({ params, request }) => {
        const url = new URL(request.url);
        return HttpResponse.json({
          params,
          sort: url.searchParams.get("sort"),
        });
      }),
    ],
    exec: async () => {
      const res = await fetch("http://mocked.test/users/7/posts/42?sort=desc");
      return normalized(res);
    },
  },
  {
    name: "wildcard path captures into params['0']",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/files/*", ({ params }) => HttpResponse.json({ params })),
    ],
    exec: async () => {
      const res = await fetch("http://mocked.test/files/img/logo.png");
      return normalized(res);
    },
  },
  {
    name: "text/xml/html statics",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/t", () => HttpResponse.text("plain")),
      http.get("http://mocked.test/x", () => HttpResponse.xml("<a/>")),
      http.get("http://mocked.test/h", () => HttpResponse.html("<p>hi</p>")),
    ],
    exec: async () => [
      await normalized(await fetch("http://mocked.test/t")),
      await normalized(await fetch("http://mocked.test/x")),
      await normalized(await fetch("http://mocked.test/h")),
    ],
  },
  {
    name: "once handler falls back after consumption",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/data", () => HttpResponse.json({ from: "once" }), {
        once: true,
      }),
      http.get("http://mocked.test/data", () => HttpResponse.json({ from: "steady" })),
    ],
    exec: async () => {
      const out: string[] = [];
      for (let i = 0; i < 3; i++) {
        const res = await fetch("http://mocked.test/data");
        out.push(((await res.json()) as any).from);
      }
      return out;
    },
  },
  {
    name: "generator advances and repeats the last value",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/gen", function* () {
        yield HttpResponse.json({ n: 1 });
        yield HttpResponse.json({ n: 2 });
      }),
    ],
    exec: async () => {
      const out: number[] = [];
      for (let i = 0; i < 3; i++) {
        const res = await fetch("http://mocked.test/gen");
        out.push(((await res.json()) as any).n);
      }
      return out;
    },
  },
  {
    name: "undefined return falls through to the next handler",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/ft", ({ request }) => {
        if (new URL(request.url).searchParams.get("special") !== "1") {
          return undefined;
        }
        return HttpResponse.json({ from: "special" });
      }),
      http.get("http://mocked.test/ft", () => HttpResponse.json({ from: "default" })),
    ],
    exec: async () => [
      ((await (await fetch("http://mocked.test/ft?special=1")).json()) as any)
        .from,
      ((await (await fetch("http://mocked.test/ft")).json()) as any).from,
    ],
  },
  {
    name: "redirect with manual redirect mode",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/old", () => HttpResponse.redirect("http://mocked.test/new", 308)),
    ],
    exec: async () => {
      const res = await fetch("http://mocked.test/old", { redirect: "manual" });
      return { status: res.status, location: res.headers.get("location") };
    },
  },
  {
    name: "network error rejects fetch",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://mocked.test/down", () => HttpResponse.error()),
    ],
    exec: async () => {
      try {
        await fetch("http://mocked.test/down");
        return "resolved";
      } catch (error) {
        return (error as Error).constructor.name;
      }
    },
  },
  {
    name: "statusText and multi Set-Cookie",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://cookie-set.test/cookies", () => {
        const headers = new Headers();
        headers.append("set-cookie", "a=1");
        headers.append("set-cookie", "b=2");
        return new HttpResponse("ok", {
          status: 418,
          statusText: "Teapot",
          headers,
        });
      }),
    ],
    exec: async () => {
      const res = await fetch("http://cookie-set.test/cookies");
      return {
        status: res.status,
        statusText: res.statusText,
        cookies: res.headers.getSetCookie(),
      };
    },
  },
  {
    name: "request cookies reach the resolver",
    handlers: ({ http, HttpResponse }) => [
      http.get("http://cookie-read.test/whoami", ({ cookies }) => HttpResponse.json({ cookies })),
    ],
    exec: async () => {
      const res = await fetch("http://cookie-read.test/whoami", {
        headers: { cookie: "session=abc; pref=dark" },
      });
      return normalized(res);
    },
  },
  {
    name: "graphql query, mutation, and operation",
    handlers: ({ graphql, HttpResponse }) => [
      graphql.query("GetUser", ({ variables }) =>
        HttpResponse.json({ data: { user: { id: variables.id } } })
      ),
      graphql.mutation("CreateUser", () =>
        HttpResponse.json({ data: { created: true } })
      ),
      graphql.operation(({ operationName }) =>
        HttpResponse.json({ data: { fallback: operationName } })
      ),
    ],
    exec: async () => {
      const gql = async (query: string, variables?: object) => {
        const res = await fetch("http://mocked.test/graphql", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ query, variables }),
        });
        return res.json();
      };
      return [
        await gql("query GetUser($id: ID!) { user(id: $id) { id } }", {
          id: "u1",
        }),
        await gql("mutation CreateUser { createUser { id } }"),
        await gql("query SomethingElse { x }"),
      ];
    },
  },
];

async function run(
  lib: Lib,
  setupServer: typeof mswSetupServer,
  scenario: Scenario
): Promise<unknown> {
  const server = setupServer(...scenario.handlers(lib));
  server.listen({ onUnhandledRequest: "error" });
  try {
    return await scenario.exec();
  } finally {
    server.close();
  }
}

describe("differential: mockpit output equals msw output", () => {
  for (const scenario of scenarios) {
    it(scenario.name, async () => {
      const mswResult = await run(
        { http: mswHttp, graphql: mswGraphql, HttpResponse: MswHttpResponse },
        mswSetupServer,
        scenario
      );
      const pitResult = await run(
        {
          http: pitHttp as unknown as typeof mswHttp,
          graphql: pitGraphql as unknown as typeof mswGraphql,
          HttpResponse: PitHttpResponse as unknown as typeof MswHttpResponse,
        },
        pitSetupServer as unknown as typeof mswSetupServer,
        scenario
      );
      expect(pitResult).toEqual(mswResult as any);
    });
  }
});
