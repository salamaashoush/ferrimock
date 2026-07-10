/**
 * TCP-lane streaming: `FerrimockServer.listen()` serves JS-defined
 * `ws.link` and `sse` handlers natively through the Rust engine — no
 * interceptor, real sockets.
 */

import { describe, it, expect, afterEach } from "bun:test";
import { FerrimockServer } from "ferrimock-node";
import { ws, sse } from "../src/index.js";

let server: FerrimockServer | null = null;

async function listen(): Promise<string> {
  const url = await server!.listen();
  return url;
}

afterEach(async () => {
  await server?.close();
  server = null;
});

function openSocket(url: string): Promise<{
  socket: WebSocket;
  next(): Promise<unknown>;
}> {
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
    socket.addEventListener("open", () => resolve({ socket, next }));
    socket.addEventListener("error", () => reject(new Error("socket error")));
  });
}

describe("TCP lane: ws.link", () => {
  it("serves connections natively with params and echo", async () => {
    server = new FerrimockServer();
    const rooms = ws.link("/ws/room/:roomId");
    const handler = rooms.addEventListener(
      "connection",
      ({ client, params }) => {
        client.send(`room:${params.roomId}`);
        client.addEventListener("message", (event) => {
          client.send(`echo:${(event as MessageEvent).data}`);
        });
      }
    );
    server.useHandlers([handler.native]);
    const url = (await listen()).replace("http://", "ws://");

    const { socket, next } = await openSocket(`${url}/ws/room/42`);
    expect(await next()).toBe("room:42");
    socket.send("hi");
    expect(await next()).toBe("echo:hi");
    socket.close();
  });

  it("delivers binary frames and close code/reason", async () => {
    server = new FerrimockServer();
    const link = ws.link("/ws/bin");
    const handler = link.addEventListener("connection", ({ client }) => {
      client.addEventListener("message", (event) => {
        const data = (event as MessageEvent).data;
        if (typeof data === "string") {
          client.send(new Uint8Array([7, 8, 9]));
        } else {
          client.close(4009, "enough");
        }
      });
    });
    server.useHandlers([handler.native]);
    const url = (await listen()).replace("http://", "ws://");

    const { socket, next } = await openSocket(`${url}/ws/bin`);
    socket.binaryType = "arraybuffer";
    const closed = new Promise<CloseEvent>((resolve) =>
      socket.addEventListener("close", (e) => resolve(e as CloseEvent), {
        once: true,
      })
    );

    socket.send("gimme");
    const data = (await next()) as ArrayBuffer;
    expect([...new Uint8Array(data)]).toEqual([7, 8, 9]);

    socket.send(new Uint8Array([1, 2]));
    const event = await closed;
    expect(event.code).toBe(4009);
    expect(event.reason).toBe("enough");
  });

  it("removeMock closes live connections with 1001", async () => {
    server = new FerrimockServer();
    const link = ws.link("/ws/doomed");
    const handler = link.addEventListener("connection", ({ client }) => {
      client.send("hi");
    });
    server.useHandlers([handler.native]);
    const url = (await listen()).replace("http://", "ws://");

    const { socket, next } = await openSocket(`${url}/ws/doomed`);
    expect(await next()).toBe("hi");
    const closed = new Promise<CloseEvent>((resolve) =>
      socket.addEventListener("close", (e) => resolve(e as CloseEvent), {
        once: true,
      })
    );

    server.removeMock(handler.id);
    const event = await closed;
    // The wire frame is 1001 Going Away (asserted by the Rust
    // tungstenite test); bun's client remaps it to 1000.
    expect([1000, 1001]).toContain(event.code);
  });
});

describe("TCP lane: sse", () => {
  it("streams frames in MSW wire shape without requiring an accept header", async () => {
    server = new FerrimockServer();
    const handler = sse("/stream", ({ client }) => {
      client.send({ data: "one" });
      client.send({ id: "5", event: "tick", data: { n: 2 } });
      client.send({ retry: 1500 });
      client.close();
    });
    server.useHandlers([handler]);
    const url = await listen();

    // No accept header: the TCP lane matches anyway (curl ergonomics).
    const response = await fetch(`${url}/stream`);
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("text/event-stream");
    const body = await response.text();
    expect(body).toContain("data:one\n\n");
    expect(body).toContain('id:5\nevent:tick\ndata:{"n":2}\n\n');
    expect(body).toContain("retry:1500\n\n");
  });

  it("streams incrementally with live sends", async () => {
    server = new FerrimockServer();
    const handler = sse("/live", async ({ client }) => {
      client.send({ data: "first" });
      await new Promise((r) => setTimeout(r, 30));
      client.send({ data: "second" });
      client.close();
    });
    server.useHandlers([handler]);
    const url = await listen();

    const response = await fetch(`${url}/live`);
    const reader = response.body!.getReader();
    const decoder = new TextDecoder();
    let text = "";
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      text += decoder.decode(value, { stream: true });
    }
    expect(text).toBe("data:first\n\ndata:second\n\n");
  });
});
