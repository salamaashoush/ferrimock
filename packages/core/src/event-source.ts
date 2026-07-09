/**
 * Minimal fetch-based EventSource used by `sse()`'s `server.connect()`
 * passthrough. Node/bun ship no global EventSource, and undici's
 * experimental one bypasses the patched fetch — this reader rides
 * `bypass(fetch)` so the passthrough request skips re-matching.
 *
 * Spec subset: `message`/named events with `data`/`lastEventId`,
 * `open`/`error` events, `close()`, and auto-reconnect — a dropped
 * stream dispatches `error`, waits the server-advised `retry:` delay
 * (default 3s), and redials with `Last-Event-ID`. An HTTP error or a
 * non-`text/event-stream` response is terminal (no reconnect), per the
 * spec's fail-the-connection cases.
 */

import { bypass } from "./msw-compat.js";

export interface EventSourceLike {
  readonly url: string;
  readonly readyState: number;
  onopen: ((event: Event) => void) | null;
  onmessage: ((event: MessageEvent) => void) | null;
  onerror: ((event: Event) => void) | null;
  addEventListener(type: string, listener: (event: Event) => void): void;
  removeEventListener(type: string, listener: (event: Event) => void): void;
  close(): void;
}

const CONNECTING = 0;
const OPEN = 1;
const CLOSED = 2;

const DEFAULT_RETRY_MS = 3000;

interface ParsedFrame {
  id?: string;
  event: string;
  data: string;
  retry?: number;
}

/** Incremental SSE frame parser (handles \n, \r\n, and \r separators). */
export class SseFrameParser {
  private buffer = "";

  push(chunk: string): ParsedFrame[] {
    this.buffer += chunk;
    const frames: ParsedFrame[] = [];
    // Normalize line endings once so frame splitting is uniform.
    const normalized = this.buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
    const parts = normalized.split("\n\n");
    this.buffer = parts.pop() ?? "";
    for (const part of parts) {
      const frame = parseFrame(part);
      if (frame) {
        frames.push(frame);
      }
    }
    return frames;
  }
}

function parseFrame(block: string): ParsedFrame | null {
  let id: string | undefined;
  let event = "message";
  let retry: number | undefined;
  const data: string[] = [];
  let sawField = false;
  for (const line of block.split("\n")) {
    if (line === "" || line.startsWith(":")) {
      continue;
    }
    const colon = line.indexOf(":");
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? "" : line.slice(colon + 1);
    if (value.startsWith(" ")) {
      value = value.slice(1);
    }
    switch (field) {
      case "id":
        id = value;
        sawField = true;
        break;
      case "event":
        event = value;
        sawField = true;
        break;
      case "data":
        data.push(value);
        sawField = true;
        break;
      case "retry": {
        const parsed = Number.parseInt(value, 10);
        if (Number.isFinite(parsed) && parsed >= 0) {
          retry = parsed;
        }
        sawField = true;
        break;
      }
      default:
        break;
    }
  }
  if (!sawField) {
    return null;
  }
  if (data.length === 0) {
    if (id !== undefined || retry !== undefined) {
      return { id, event, data: "", retry };
    }
    return null;
  }
  return { id, event, data: data.join("\n"), retry };
}

export interface MockpitEventSourceOptions {
  /**
   * Called for every frame whose MessageEvent was not
   * `preventDefault()`-ed by a listener — the forwarding hook
   * `server.connect()` uses to relay real events to the mocked client.
   */
  onFrameForward?: (frame: {
    id?: string;
    event: string;
    data: string;
  }) => void;
}

export class MockpitEventSource extends EventTarget implements EventSourceLike {
  readonly url: string;
  #readyState: number = CONNECTING;
  #abort = new AbortController();
  #lastEventId = "";
  #retryMs = DEFAULT_RETRY_MS;
  #reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  #onFrameForward: MockpitEventSourceOptions["onFrameForward"];

  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;

  constructor(url: string, options?: MockpitEventSourceOptions) {
    super();
    this.url = url;
    this.#onFrameForward = options?.onFrameForward;
    void this.#connect();
  }

  get readyState(): number {
    return this.#readyState;
  }

  close(): void {
    this.#readyState = CLOSED;
    if (this.#reconnectTimer !== null) {
      clearTimeout(this.#reconnectTimer);
      this.#reconnectTimer = null;
    }
    this.#abort.abort();
  }

  #dispatch(event: Event): void {
    this.dispatchEvent(event);
    if (event.type === "open") {
      this.onopen?.(event);
    } else if (event.type === "error") {
      this.onerror?.(event);
    } else if (event.type === "message") {
      this.onmessage?.(event as MessageEvent);
    }
  }

  /** Terminal failure: no reconnect (spec: fail the connection). */
  #fail(): void {
    this.#readyState = CLOSED;
    this.#dispatch(new Event("error"));
  }

  /** Recoverable drop: error event, then redial after the retry delay. */
  #scheduleReconnect(): void {
    if (this.#readyState === CLOSED) {
      return;
    }
    this.#readyState = CONNECTING;
    this.#dispatch(new Event("error"));
    if (this.#readyState === CLOSED) {
      return; // an error listener called close()
    }
    const timer = setTimeout(() => {
      this.#reconnectTimer = null;
      if (this.#readyState !== CLOSED) {
        void this.#connect();
      }
    }, this.#retryMs);
    // Do not hold the process open for a passthrough retry loop.
    (timer as { unref?: () => void }).unref?.();
    this.#reconnectTimer = timer;
  }

  async #connect(): Promise<void> {
    this.#abort = new AbortController();
    const headers: Record<string, string> = { accept: "text/event-stream" };
    if (this.#lastEventId !== "") {
      headers["last-event-id"] = this.#lastEventId;
    }

    let response: Response;
    try {
      response = await fetch(
        bypass(this.url, {
          headers,
          signal: this.#abort.signal,
        })
      );
    } catch {
      this.#scheduleReconnect();
      return;
    }

    if (
      !response.ok ||
      !response.headers.get("content-type")?.includes("text/event-stream") ||
      !response.body
    ) {
      this.#fail();
      return;
    }

    if (this.#readyState === CLOSED) {
      return;
    }
    this.#readyState = OPEN;
    this.#dispatch(new Event("open"));

    try {
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      const parser = new SseFrameParser();
      for (;;) {
        const { done, value } = await reader.read();
        if (done) {
          break;
        }
        for (const frame of parser.push(decoder.decode(value, { stream: true }))) {
          if (frame.retry !== undefined) {
            this.#retryMs = frame.retry;
          }
          if (frame.id !== undefined) {
            this.#lastEventId = frame.id;
          }
          if (frame.data === "" && frame.id === undefined && frame.retry !== undefined) {
            continue; // bare retry: adjusts the delay, dispatches nothing
          }
          const event = new MessageEvent(frame.event, {
            data: frame.data,
            lastEventId: this.#lastEventId,
            cancelable: true,
          });
          this.#dispatch(event);
          // Forward after listeners had their chance to preventDefault
          // (MSW's listen-and-swallow semantics).
          if (this.#onFrameForward) {
            const forward = this.#onFrameForward;
            queueMicrotask(() => {
              if (!event.defaultPrevented) {
                forward(frame);
              }
            });
          }
        }
      }
    } catch {
      // fall through to the reconnect path
    }
    this.#scheduleReconnect();
  }
}
