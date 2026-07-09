/**
 * MSW-compatible `ws` namespace: interception via the patched global
 * WebSocket, link semantics, registry lifecycle, and passthrough.
 */

import { describe, it, expect, afterEach } from "bun:test";
import { setupServer } from "../src/node.js";
import { ws } from "../src/ws.js";
import { http, HttpResponse } from "../src/index.js";

let active: { close(): void } | null = null;
let realServer: ReturnType<typeof Bun.serve> | null = null;

function server(...handlers: Parameters<typeof setupServer>) {
  const s = setupServer(...handlers);
  s.listen({ onUnhandledRequest: "error" });
  active = s;
  return s;
}

afterEach(() => {
  active?.close();
  active = null;
  realServer?.stop(true);
  realServer = null;
});

/**
 * Open a socket with a message queue attached BEFORE open fires — mocked
 * sends queued in the connection listener dispatch immediately after
 * the open event, so a listener attached after `await open` misses them.
 */
type TestSocket = {
  socket: WebSocket;
  next(): Promise<unknown>;
  pending(): number;
};

function openSocket(url: string): Promise<TestSocket> {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url);
    const queue: unknown[] = [];
    const waiters: Array<(data: unknown) => void> = [];
    socket.addEventListener("message", (event) => {
      const waiter = waiters.shift();
      if (waiter) {
        waiter(event.data);
      } else {
        queue.push(event.data);
      }
    });
    const next = () =>
      new Promise<unknown>((resolveNext) => {
        if (queue.length > 0) {
          resolveNext(queue.shift());
        } else {
          waiters.push(resolveNext);
        }
      });
    socket.addEventListener("open", () =>
      resolve({ socket, next, pending: () => queue.length })
    );
    socket.addEventListener("error", (event) =>
      reject((event as any).cause ?? new Error("socket error"))
    );
  });
}

function nextClose(socket: WebSocket): Promise<CloseEvent> {
  return new Promise((resolve) => {
    socket.addEventListener("close", (event) => resolve(event as CloseEvent), {
      once: true,
    });
  });
}

describe("ws.link", () => {
  it("intercepts connections and echoes messages", async () => {
    const chat = ws.link("wss://chat.example.com");
    server(
      chat.addEventListener("connection", ({ client }) => {
        client.send("welcome");
        client.addEventListener("message", (event) => {
          client.send(`echo:${event.data}`);
        });
      })
    );

    const { socket, next } = await openSocket("wss://chat.example.com");
    expect(await next()).toBe("welcome");

    socket.send("hi");
    expect(await next()).toBe("echo:hi");
    socket.close();
  });

  it("extracts path params into the connection", async () => {
    const rooms = ws.link("wss://chat.example.com/room/:roomId");
    let seenParams: Record<string, unknown> | null = null;
    server(
      rooms.addEventListener("connection", ({ client, params }) => {
        seenParams = params;
        client.send(`room:${params.roomId}`);
      })
    );

    const { socket, next } = await openSocket(
      "wss://chat.example.com/room/42"
    );
    expect(await next()).toBe("room:42");
    expect(seenParams).toEqual({ roomId: "42" });
    socket.close();
  });

  it("broadcast and broadcastExcept reach the right clients", async () => {
    const link = ws.link("wss://hub.example.com");
    server(
      link.addEventListener("connection", ({ client }) => {
        client.addEventListener("message", (event) => {
          if (event.data === "all") {
            link.broadcast("to-everyone");
          } else if (event.data === "others") {
            link.broadcastExcept(client, "to-others");
          }
        });
      })
    );

    const first = await openSocket("wss://hub.example.com");
    const second = await openSocket("wss://hub.example.com");
    expect(link.clients.size).toBe(2);

    first.socket.send("all");
    expect(await first.next()).toBe("to-everyone");
    expect(await second.next()).toBe("to-everyone");

    first.socket.send("others");
    expect(await second.next()).toBe("to-others");
    expect(first.pending()).toBe(0);

    first.socket.close();
    second.socket.close();
  });

  it("client.close delivers code and reason; clients set prunes", async () => {
    const link = ws.link("wss://close.example.com");
    server(
      link.addEventListener("connection", ({ client }) => {
        client.addEventListener("message", () => {
          client.close(4001, "goodbye");
        });
      })
    );

    const { socket } = await openSocket("wss://close.example.com");
    expect(link.clients.size).toBe(1);
    const closed = nextClose(socket);
    socket.send("bye");
    const event = await closed;
    expect(event.code).toBe(4001);
    expect(event.reason).toBe("goodbye");
    expect(link.clients.size).toBe(0);
  });

  it("binary frames pass through both directions", async () => {
    const link = ws.link("wss://bin.example.com");
    server(
      link.addEventListener("connection", ({ client }) => {
        client.addEventListener("message", (event) => {
          if (typeof event.data === "string") {
            client.send(new Uint8Array([7, 8, 9]));
          } else {
            const bytes = new Uint8Array(event.data as ArrayBuffer);
            client.send(`len:${bytes.byteLength}`);
          }
        });
      })
    );

    const { socket, next } = await openSocket("wss://bin.example.com");
    socket.binaryType = "arraybuffer";

    socket.send("gimme");
    const data = (await next()) as ArrayBuffer | Blob;
    const bytes = new Uint8Array(
      data instanceof Blob ? await data.arrayBuffer() : data
    );
    expect([...bytes]).toEqual([7, 8, 9]);

    socket.send(new Uint8Array([1, 2, 3, 4]));
    expect(await next()).toBe("len:4");
    socket.close();
  });

  it("RegExp links match against the full URL", async () => {
    const link = ws.link(/wss:\/\/regex\.example\.com\/live/);
    server(
      link.addEventListener("connection", ({ client }) => {
        client.send("regex-hit");
      })
    );

    const { socket, next } = await openSocket(
      "wss://regex.example.com/live/feed"
    );
    expect(await next()).toBe("regex-hit");
    socket.close();
  });
});

describe("ws lifecycle", () => {
  it("use() adds runtime handlers; resetHandlers removes them", async () => {
    const initial = ws.link("wss://life.example.com");
    const s = server(
      initial.addEventListener("connection", ({ client }) => {
        client.send("initial");
      })
    );

    const runtime = ws.link("wss://runtime.example.com");
    const runtimeHandler = runtime.addEventListener(
      "connection",
      ({ client }) => {
        client.send("runtime");
      }
    );
    s.use(runtimeHandler);

    const runtimeSocket = await openSocket("wss://runtime.example.com");
    expect(await runtimeSocket.next()).toBe("runtime");
    runtimeSocket.socket.close();

    s.resetHandlers();
    // Runtime handler gone: connection is unhandled -> error strategy.
    await expect(openSocket("wss://runtime.example.com")).rejects.toBeDefined();

    // Initial handler still present.
    const initialSocket = await openSocket("wss://life.example.com");
    expect(await initialSocket.next()).toBe("initial");
    initialSocket.socket.close();
  });

  it("listHandlers includes websocket entries", () => {
    const link = ws.link("wss://list.example.com");
    const s = server(
      link.addEventListener("connection", () => {}),
      http.get("/plain", () => HttpResponse.json({ ok: true }))
    );
    const entries = s.listHandlers();
    const wsEntries = entries.filter(
      (entry) => (entry as { kind?: string }).kind === "websocket"
    );
    expect(wsEntries.length).toBe(1);
    expect(entries.length).toBeGreaterThanOrEqual(2);
  });

  it("boundary scopes ws handlers added inside", async () => {
    const s = server();
    const link = ws.link("wss://scoped.example.com");

    await s.boundary(async () => {
      s.use(
        link.addEventListener("connection", ({ client }) => {
          client.send("scoped");
        })
      );
      const scoped = await openSocket("wss://scoped.example.com");
      expect(await scoped.next()).toBe("scoped");
      scoped.socket.close();
    })();

    await expect(openSocket("wss://scoped.example.com")).rejects.toBeDefined();
  });

  it("unhandled connections hit onUnhandledRequest 'error' with no passthrough", async () => {
    server(); // zero handlers, error strategy
    await expect(openSocket("wss://nobody.example.com")).rejects.toBeDefined();
  });
});

describe("ws passthrough", () => {
  it("server.connect() forwards both directions against a real server", async () => {
    realServer = Bun.serve({
      port: 0,
      fetch(req, srv) {
        if (srv.upgrade(req)) return undefined as unknown as Response;
        return new Response("not ws", { status: 400 });
      },
      websocket: {
        open(socket) {
          socket.send("real-hello");
        },
        message(socket, message) {
          socket.send(`real-echo:${message}`);
        },
      },
    });
    const realUrl = `ws://127.0.0.1:${realServer.port}`;

    const link = ws.link(realUrl);
    server(
      link.addEventListener("connection", ({ client, server: real }) => {
        real.connect();
        client.addEventListener("message", (event) => {
          if (event.data === "local") {
            event.preventDefault();
            client.send("answered-locally");
          }
        });
        real.addEventListener("message", (event) => {
          if (event.data === "real-hello") {
            event.preventDefault();
            client.send("rewrote-hello");
          }
        });
      })
    );

    const { socket, next } = await openSocket(realUrl);

    expect(await next()).toBe("rewrote-hello");

    socket.send("local");
    expect(await next()).toBe("answered-locally");

    socket.send("ping");
    expect(await next()).toBe("real-echo:ping");
    socket.close();
  });
});
