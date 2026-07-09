/**
 * MSW-compatible `sse()`: frame encoding, predicate fall-through,
 * close/error semantics, timing, and server.connect() passthrough.
 */

import { describe, it, expect, afterEach } from "bun:test";
import { setupServer } from "../src/node.js";
import { sse } from "../src/sse.js";
import { http, HttpResponse, delay } from "../src/index.js";

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
  realServer?.stop(true);
  realServer = null;
});

const SSE_HEADERS = { accept: "text/event-stream" };

async function readAll(response: Response): Promise<string> {
  return response.text();
}

async function readFrames(
  response: Response,
  count: number
): Promise<string[]> {
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  const frames: string[] = [];
  while (frames.length < count) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    const parts = buffer.split("\n\n");
    buffer = parts.pop() ?? "";
    frames.push(...parts);
  }
  await reader.cancel();
  return frames;
}

describe("sse", () => {
  it("encodes frames byte-exactly (MSW wire shape)", async () => {
    server(
      sse("http://sse.test/stream", ({ client }) => {
        client.send({ data: "one" });
        client.send({ id: "7", event: "price", data: { px: 123 } });
        client.send({ retry: 3000 });
        client.send({ data: "multi\nline" });
        client.close();
      })
    );

    const response = await fetch("http://sse.test/stream", {
      headers: SSE_HEADERS,
    });
    expect(response.status).toBe(200);
    expect(response.headers.get("content-type")).toBe("text/event-stream");
    expect(response.headers.get("cache-control")).toBe("no-cache");

    const body = await readAll(response);
    expect(body).toBe(
      "data:one\n\n" +
        'id:7\nevent:price\ndata:{"px":123}\n\n' +
        "retry:3000\n\n" +
        "data:multi\ndata:line\n\n"
    );
  });

  it("strict accept predicate falls through to the next handler", async () => {
    server(
      sse("http://sse.test/dual", ({ client }) => {
        client.send({ data: "stream" });
        client.close();
      }),
      http.get("http://sse.test/dual", () =>
        HttpResponse.json({ plain: true })
      )
    );

    // No accept header: sse handler falls through, JSON handler responds.
    const plain = await fetch("http://sse.test/dual");
    expect(((await plain.json()) as any).plain).toBe(true);

    // With the SSE accept header the stream handler wins.
    const stream = await fetch("http://sse.test/dual", {
      headers: SSE_HEADERS,
    });
    expect(await readAll(stream)).toBe("data:stream\n\n");
  });

  it("delivers events incrementally with resolver timing", async () => {
    server(
      sse("http://sse.test/ticks", async ({ client }) => {
        client.send({ data: "first" });
        await delay(30);
        client.send({ data: "second" });
        client.close();
      })
    );

    const started = Date.now();
    const response = await fetch("http://sse.test/ticks", {
      headers: SSE_HEADERS,
    });
    const frames = await readFrames(response, 2);
    expect(frames).toEqual(["data:first", "data:second"]);
    expect(Date.now() - started).toBeGreaterThanOrEqual(25);
  });

  it("client.error() rejects the read mid-stream", async () => {
    server(
      sse("http://sse.test/broken", async ({ client }) => {
        client.send({ data: "before" });
        await delay(10);
        client.error();
      })
    );

    const response = await fetch("http://sse.test/broken", {
      headers: SSE_HEADERS,
    });
    const reader = response.body!.getReader();
    const first = await reader.read();
    expect(new TextDecoder().decode(first.value)).toContain("before");
    await expect(
      (async () => {
        for (;;) {
          const { done } = await reader.read();
          if (done) return "clean-end";
        }
      })()
    ).rejects.toBeDefined();
  });

  it("dispatchEvent maps MessageEvents onto the stream", async () => {
    server(
      sse("http://sse.test/dispatch", ({ client }) => {
        client.dispatchEvent(
          new MessageEvent("update", { data: "via-dispatch", lastEventId: "9" })
        );
        client.close();
      })
    );

    const response = await fetch("http://sse.test/dispatch", {
      headers: SSE_HEADERS,
    });
    expect(await readAll(response)).toBe(
      "id:9\nevent:update\ndata:via-dispatch\n\n"
    );
  });

  it("server.connect() forwards real events unless prevented", async () => {
    realServer = Bun.serve({
      port: 0,
      fetch() {
        const stream = new ReadableStream({
          async start(controller) {
            const enc = new TextEncoder();
            controller.enqueue(enc.encode("data:real-one\n\n"));
            controller.enqueue(enc.encode("event:secret\ndata:hidden\n\n"));
            controller.enqueue(enc.encode("data:real-two\n\n"));
            controller.close();
          },
        });
        return new Response(stream, {
          headers: { "content-type": "text/event-stream" },
        });
      },
    });
    const realUrl = `http://127.0.0.1:${realServer.port}/feed`;

    server(
      sse(realUrl, ({ client, server: real }) => {
        const source = real.connect();
        source.addEventListener("secret", (event) => {
          event.preventDefault();
          client.send({ data: "redacted" });
        });
        source.addEventListener("error", () => {
          client.close();
        });
      })
    );

    const response = await fetch(realUrl, { headers: SSE_HEADERS });
    const body = await readAll(response);
    expect(body).toContain("data:real-one\n\n");
    expect(body).toContain("data:redacted\n\n");
    expect(body).toContain("data:real-two\n\n");
    expect(body).not.toContain("hidden");
  });
});
