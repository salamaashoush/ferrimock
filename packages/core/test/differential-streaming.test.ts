/**
 * Differential ws/sse: identical handlers against real msw and ferrimock,
 * identical clients, equal observations. Sequential (both patch
 * globals).
 *
 * msw's sse() asserts a global EventSource exists at handler
 * construction (constructor-only invariant) — a stub class satisfies it.
 */

import { describe, it, expect, beforeAll } from "bun:test";
import { ws as mswWs, sse as mswSse } from "msw";
import { setupServer as mswSetupServer } from "msw/node";
import { ws as pitWs, sse as pitSse, setupServer as pitSetupServer } from "../src/index.js";

beforeAll(() => {
  (globalThis as any).EventSource ??= class EventSourceStub {};
});

type WsNamespace = typeof mswWs;
type SseFactory = typeof mswSse;
type SetupServer = typeof mswSetupServer;

async function runWsScenario(
  wsNs: WsNamespace,
  setupServer: SetupServer,
  url: string
): Promise<unknown[]> {
  const link = wsNs.link(url);
  const server = setupServer(
    link.addEventListener("connection", ({ client, params }) => {
      client.send(`hello:${(params as any).roomId ?? "none"}`);
      client.addEventListener("message", (event) => {
        if (event.data === "bye") {
          client.close(4002, "done");
          return;
        }
        client.send(`echo:${event.data}`);
      });
    }) as any
  );
  server.listen({ onUnhandledRequest: "bypass" });

  const log: unknown[] = [];
  try {
    await new Promise<void>((resolve, reject) => {
      const socket = new WebSocket(url.replace(":roomId", "42"));
      const timer = setTimeout(() => reject(new Error("timeout")), 3000);
      socket.addEventListener("message", (event) => {
        log.push(["message", event.data]);
        if (event.data === "echo:ping") {
          socket.send("bye");
        }
      });
      socket.addEventListener("open", () => {
        socket.send("ping");
      });
      socket.addEventListener("close", (event) => {
        log.push(["close", event.code, event.reason]);
        clearTimeout(timer);
        resolve();
      });
      socket.addEventListener("error", () => {
        clearTimeout(timer);
        reject(new Error("socket error"));
      });
    });
  } finally {
    server.close();
  }
  return log;
}

async function runSseScenario(
  sseFactory: SseFactory,
  setupServer: SetupServer,
  url: string
): Promise<unknown> {
  const server = setupServer(
    sseFactory(url, ({ client }) => {
      client.send({ data: "one" } as any);
      client.send({ id: "5", event: "tick", data: { n: 2 } } as any);
      client.send({ retry: 1500 } as any);
      client.close();
    }) as any
  );
  server.listen({ onUnhandledRequest: "bypass" });
  try {
    const response = await fetch(url, {
      headers: { accept: "text/event-stream" },
    });
    return {
      status: response.status,
      contentType: response.headers.get("content-type"),
      body: await response.text(),
    };
  } finally {
    server.close();
  }
}

describe("differential: ws", () => {
  it("echo + params + close tuple match msw", async () => {
    const mswLog = await runWsScenario(
      mswWs,
      mswSetupServer,
      "wss://diff-ws.test/room/:roomId"
    );
    const pitLog = await runWsScenario(
      pitWs as unknown as WsNamespace,
      pitSetupServer as unknown as SetupServer,
      "wss://diff-ws.test/room/:roomId"
    );
    expect(pitLog).toEqual(mswLog);
    expect(mswLog).toContainEqual(["message", "hello:42"]);
  });

  it("RegExp links match the full connection URL in both", async () => {
    const run = async (wsNs: WsNamespace, setupServer: SetupServer) => {
      const link = wsNs.link(/wss:\/\/diff-regex\.test\/live/);
      const server = setupServer(
        link.addEventListener("connection", ({ client }) => {
          client.send("regex-hit");
        }) as any
      );
      server.listen({ onUnhandledRequest: "bypass" });
      const log: unknown[] = [];
      try {
        await new Promise<void>((resolve, reject) => {
          const socket = new WebSocket("wss://diff-regex.test/live/feed");
          const timer = setTimeout(() => reject(new Error("timeout")), 3000);
          socket.addEventListener("message", (event) => {
            log.push(["message", event.data]);
            socket.close();
          });
          socket.addEventListener("close", () => {
            clearTimeout(timer);
            resolve();
          });
          socket.addEventListener("error", () => {
            clearTimeout(timer);
            reject(new Error("socket error"));
          });
        });
      } finally {
        server.close();
      }
      return log;
    };

    const mswLog = await run(mswWs, mswSetupServer);
    const pitLog = await run(
      pitWs as unknown as WsNamespace,
      pitSetupServer as unknown as SetupServer
    );
    expect(pitLog).toEqual(mswLog);
    expect(mswLog).toContainEqual(["message", "regex-hit"]);
  });

  it("broadcast reaches every client in both", async () => {
    const url = "wss://diff-broadcast.test";
    const run = async (wsNs: WsNamespace, setupServer: SetupServer) => {
      const link = wsNs.link(url);
      const server = setupServer(
        link.addEventListener("connection", ({ client }) => {
          client.addEventListener("message", (event) => {
            if (event.data === "go") {
              link.broadcast("to-everyone");
            }
          });
        }) as any
      );
      server.listen({ onUnhandledRequest: "bypass" });
      try {
        const open = (s: WebSocket) =>
          new Promise<void>((resolve) =>
            s.addEventListener("open", () => resolve(), { once: true })
          );
        const nextMessage = (s: WebSocket) =>
          new Promise<unknown>((resolve) =>
            s.addEventListener("message", (e) => resolve(e.data), {
              once: true,
            })
          );
        const first = new WebSocket(url);
        const second = new WebSocket(url);
        await Promise.all([open(first), open(second)]);
        const messages = Promise.all([nextMessage(first), nextMessage(second)]);
        first.send("go");
        const received = await messages;
        first.close();
        second.close();
        return received;
      } finally {
        server.close();
      }
    };

    const mswResult = await run(mswWs, mswSetupServer);
    const pitResult = await run(
      pitWs as unknown as WsNamespace,
      pitSetupServer as unknown as SetupServer
    );
    expect(pitResult).toEqual(mswResult);
    expect(mswResult).toEqual(["to-everyone", "to-everyone"]);
  });
});

describe("differential: sse", () => {
  it("frame bytes and headers match msw", async () => {
    const mswResult = await runSseScenario(
      mswSse,
      mswSetupServer,
      "http://diff-sse.test/stream"
    );
    const pitResult = await runSseScenario(
      pitSse as unknown as SseFactory,
      pitSetupServer as unknown as SetupServer,
      "http://diff-sse.test/stream"
    );
    expect(pitResult).toEqual(mswResult);
  });

  it("named events, multi-line data, and object payloads match msw byte-for-byte", async () => {
    const run = async (
      sseFactory: SseFactory,
      setupServer: SetupServer,
      url: string
    ) => {
      const server = setupServer(
        sseFactory(url, ({ client }) => {
          client.send({ event: "tick", data: "line1\nline2" } as any);
          client.send({ id: "9", data: { nested: { deep: true } } } as any);
          client.send({ data: 42 } as any);
          client.close();
        }) as any
      );
      server.listen({ onUnhandledRequest: "bypass" });
      try {
        const response = await fetch(url, {
          headers: { accept: "text/event-stream" },
        });
        return await response.text();
      } finally {
        server.close();
      }
    };

    const mswBody = await run(mswSse, mswSetupServer, "http://diff-sse.test/rich");
    const pitBody = await run(
      pitSse as unknown as SseFactory,
      pitSetupServer as unknown as SetupServer,
      "http://diff-sse.test/rich"
    );
    expect(pitBody).toEqual(mswBody);
  });

  it("requests without the accept header do not match in either", async () => {
    const run = async (
      sseFactory: SseFactory,
      setupServer: SetupServer,
      url: string
    ) => {
      const server = setupServer(
        sseFactory(url, ({ client }) => {
          client.send({ data: "x" } as any);
          client.close();
        }) as any
      );
      server.listen({ onUnhandledRequest: "bypass" });
      try {
        const response = await fetch(url).catch(() => null);
        // Unhandled -> bypass -> network failure for a fake host.
        return response === null ? "network-error" : response.status;
      } finally {
        server.close();
      }
    };

    const mswResult = await run(mswSse, mswSetupServer, "http://diff-sse.test/gate");
    const pitResult = await run(
      pitSse as unknown as SseFactory,
      pitSetupServer as unknown as SetupServer,
      "http://diff-sse.test/gate"
    );
    expect(pitResult).toEqual(mswResult);
  });
});
