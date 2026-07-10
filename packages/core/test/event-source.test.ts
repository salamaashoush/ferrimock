/**
 * FerrimockEventSource: spec-subset behavior incl. auto-reconnect — the
 * `retry:` field sets the redial delay, `Last-Event-ID` rides the
 * reconnect request, and readyState transitions CONNECTING -> OPEN ->
 * (drop) -> CONNECTING -> OPEN -> CLOSED.
 */

import { describe, it, expect, afterEach } from "bun:test";
import { FerrimockEventSource } from "../src/event-source.js";

let realServer: ReturnType<typeof Bun.serve> | null = null;

afterEach(() => {
  realServer?.stop(true);
  realServer = null;
});

function sseResponse(body: string): Response {
  return new Response(body, {
    status: 200,
    headers: { "content-type": "text/event-stream" },
  });
}

describe("FerrimockEventSource", () => {
  it("reconnects with Last-Event-ID after the retry delay", async () => {
    const lastEventIds: Array<string | null> = [];
    let requestCount = 0;

    realServer = Bun.serve({
      port: 0,
      fetch(req) {
        requestCount += 1;
        lastEventIds.push(req.headers.get("last-event-id"));
        if (requestCount === 1) {
          // retry:25 shortens the redial delay; the stream then ends.
          return sseResponse("retry:25\n\nid:7\ndata:first\n\n");
        }
        return sseResponse("id:8\ndata:second\n\n");
      },
    });

    const source = new FerrimockEventSource(
      `http://127.0.0.1:${realServer.port}/stream`
    );

    const events: string[] = [];
    const states: number[] = [];
    const gotSecond = new Promise<void>((resolve) => {
      source.addEventListener("message", (event) => {
        events.push((event as MessageEvent).data);
        if ((event as MessageEvent).data === "second") {
          resolve();
        }
      });
    });
    source.addEventListener("open", () => states.push(source.readyState));
    source.addEventListener("error", () => states.push(source.readyState));

    await gotSecond;
    source.close();

    expect(events).toEqual(["first", "second"]);
    expect(requestCount).toBe(2);
    // First request carries no Last-Event-ID; the reconnect carries the
    // last seen id.
    expect(lastEventIds).toEqual([null, "7"]);
    // open (OPEN=1), error on drop (CONNECTING=0), open again (OPEN=1).
    expect(states).toEqual([1, 0, 1]);
    expect(source.readyState).toBe(2);
  });

  it("bare retry frames adjust the delay without dispatching a message", async () => {
    realServer = Bun.serve({
      port: 0,
      fetch() {
        return sseResponse("retry:10\n\ndata:only\n\n");
      },
    });

    const source = new FerrimockEventSource(
      `http://127.0.0.1:${realServer.port}/stream`
    );
    const events: string[] = [];
    await new Promise<void>((resolve) => {
      source.addEventListener("message", (event) => {
        events.push((event as MessageEvent).data);
        resolve();
      });
    });
    source.close();
    expect(events).toEqual(["only"]);
  });

  it("HTTP errors are terminal: no reconnect", async () => {
    let requestCount = 0;
    realServer = Bun.serve({
      port: 0,
      fetch() {
        requestCount += 1;
        return new Response("nope", { status: 500 });
      },
    });

    const source = new FerrimockEventSource(
      `http://127.0.0.1:${realServer.port}/stream`
    );
    await new Promise<void>((resolve) => {
      source.addEventListener("error", () => resolve());
    });
    expect(source.readyState).toBe(2);
    await new Promise((r) => setTimeout(r, 50));
    expect(requestCount).toBe(1);
  });

  it("close() stops a pending reconnect", async () => {
    let requestCount = 0;
    realServer = Bun.serve({
      port: 0,
      fetch() {
        requestCount += 1;
        return sseResponse("retry:20\n\ndata:x\n\n");
      },
    });

    const source = new FerrimockEventSource(
      `http://127.0.0.1:${realServer.port}/stream`
    );
    // Wait for the drop -> error (reconnect scheduled), then close.
    await new Promise<void>((resolve) => {
      source.addEventListener("error", () => resolve());
    });
    source.close();
    await new Promise((r) => setTimeout(r, 80));
    expect(requestCount).toBe(1);
    expect(source.readyState).toBe(2);
  });
});
