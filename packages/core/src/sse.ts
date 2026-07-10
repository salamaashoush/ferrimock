/**
 * MSW-compatible `sse(path, resolver)` over the native engine.
 *
 * The native `sse()` binding builds one engine mock with two facets and
 * calls the SAME resolver wrapper on both lanes:
 *
 * - Interceptor lane: called with `(info)` — strict MSW predicate (only
 *   `accept: text/event-stream` requests match, others fall through),
 *   frames stream through a stash-kept ReadableStream Response.
 * - TCP lane (`FerrimockServer.listen()`): called with `(info, client)`
 *   where `client` is the native connection sink — frames stream from
 *   Rust without the stash, and no accept header is required (curl
 *   ergonomics, matching the QuickJS/declarative lanes).
 *
 * Frame encoding matches MSW byte-for-byte (no space after the colon,
 * `data:` per line, objects JSON-stringified, bare `retry:N`).
 */

import type {
  RequestHandler,
  RequestInfo,
  SseClientHandle,
} from "ferrimock-node";
import { sse as nativeSse } from "ferrimock-node";
import { normalizeResult, registerCollected } from "./registration.js";
import { FerrimockEventSource, type EventSourceLike } from "./event-source.js";

export type ServerSentEventMessage =
  | { id?: string; event?: string; data?: unknown; retry?: never }
  | { id?: never; event?: never; data?: never; retry: number };

const encoder = new TextEncoder();

function encodeFrame(payload: ServerSentEventMessage): Uint8Array {
  const frames: string[] = [];
  if (payload.retry !== undefined) {
    frames.push(`retry:${payload.retry}`);
    if (payload.data === undefined) {
      frames.push("", "");
      return encoder.encode(frames.join("\n"));
    }
  }
  if (payload.id !== undefined) {
    frames.push(`id:${payload.id}`);
  }
  if (payload.event !== undefined) {
    frames.push(`event:${payload.event}`);
  }
  const data =
    typeof payload.data === "object" && payload.data !== null
      ? JSON.stringify(payload.data)
      : String(payload.data ?? "");
  for (const line of data.split(/\r\n|\r|\n/)) {
    frames.push(`data:${line}`);
  }
  frames.push("", "");
  return encoder.encode(frames.join("\n"));
}

interface FrameSink {
  send(payload: ServerSentEventMessage): void;
  close(): void;
  error(): void;
}

/** The frame sink handed to resolvers; backs both lanes. */
export class ServerSentEventClient {
  #sink: FrameSink;
  #closed = false;

  private constructor(sink: FrameSink) {
    this.#sink = sink;
  }

  /** @internal Interceptor lane: frames feed a ReadableStream. */
  static forStream(
    controller: ReadableStreamDefaultController<Uint8Array>
  ): ServerSentEventClient {
    return new ServerSentEventClient({
      send(payload) {
        controller.enqueue(encodeFrame(payload));
      },
      close() {
        controller.close();
      },
      error() {
        controller.error(new TypeError("Failed to fetch"));
      },
    });
  }

  /** @internal TCP lane: frames feed the native connection sink.
   * Absent payload fields must cross as undefined — napi object fields
   * reject null for Option. */
  static forNative(handle: SseClientHandle): ServerSentEventClient {
    return new ServerSentEventClient({
      send(payload) {
        handle.send({
          id: payload.id,
          event: payload.event,
          data: payload.data as any,
          retry: payload.retry,
        });
      },
      close() {
        handle.close();
      },
      error() {
        handle.error();
      },
    });
  }

  /** Send a message to the intercepted consumer. */
  send(payload: ServerSentEventMessage): void {
    if (this.#closed) {
      return;
    }
    this.#sink.send(payload);
  }

  /** Map a DOM event onto the stream (MessageEvent → send, error/close). */
  dispatchEvent(event: Event): void {
    if (event instanceof MessageEvent) {
      this.send({
        id: event.lastEventId || undefined,
        event: event.type === "message" ? undefined : event.type,
        data: event.data,
      });
      return;
    }
    if (event.type === "error") {
      this.error();
    } else if (event.type === "close") {
      this.close();
    }
  }

  /** Error the connection (consumer sees a network failure). */
  error(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#sink.error();
  }

  /** Close the connection cleanly. */
  close(): void {
    if (this.#closed) {
      return;
    }
    this.#closed = true;
    this.#sink.close();
  }
}

export class ServerSentEventServer {
  #url: string | null;
  #client: ServerSentEventClient;

  constructor(args: { url: string | null; client: ServerSentEventClient }) {
    this.#url = args.url;
    this.#client = args.client;
  }

  /**
   * Open the real connection. Events forward to the mocked client
   * unless a listener on the returned source calls `preventDefault()`.
   */
  connect(): EventSourceLike {
    if (!this.#url) {
      throw new Error(
        "sse server.connect() needs an absolute handler URL (http://host/path) to know the real endpoint"
      );
    }
    const client = this.#client;
    return new FerrimockEventSource(this.#url, {
      onFrameForward(frame) {
        client.send({
          id: frame.id,
          event: frame.event === "message" ? undefined : frame.event,
          data: frame.data,
        });
      },
    });
  }
}

export type ServerSentEventResolverInfo = {
  request: Request;
  params: Record<string, string>;
  cookies: Record<string, string>;
  requestId: string;
  client: ServerSentEventClient;
  server: ServerSentEventServer;
};

const SSE_RESPONSE_INIT: ResponseInit = {
  status: 200,
  headers: {
    "content-type": "text/event-stream",
    "cache-control": "no-cache",
    connection: "keep-alive",
  },
};

/**
 * Intercept Server-Sent Events: `sse(path, ({ client }) => {
 * client.send({ data: 'hello' }) })`.
 */
export function sse(
  path: string | RegExp,
  resolver: (
    info: ServerSentEventResolverInfo
  ) => unknown | Promise<unknown>
): RequestHandler {
  // The real endpoint server.connect() dials: only an absolute handler
  // path names one (the TCP lane's request URL points at the mock
  // server itself; the interceptor lane's real URL equals the path).
  const upstreamUrl =
    typeof path === "string" &&
    (path.startsWith("http://") || path.startsWith("https://"))
      ? path
      : null;

  const handler = nativeSse(
    path as any,
    async (info: RequestInfo, clientHandle?: SseClientHandle) => {
      if (clientHandle) {
        // TCP lane: drive the native sink; the stream lives until the
        // client closes it or the consumer disconnects. A throwing
        // resolver aborts the connection (never leaves it hanging).
        const client = ServerSentEventClient.forNative(clientHandle);
        const server = new ServerSentEventServer({ url: upstreamUrl, client });
        try {
          await resolver({
            request: info.request,
            params: info.params,
            cookies: info.cookies,
            requestId: info.requestId,
            client,
            server,
          });
        } catch (error) {
          client.error();
          throw error;
        }
        return undefined;
      }

      if (info.request.headers.get("accept") !== "text/event-stream") {
        return null; // fall through (MSW predicate parity)
      }
      const requestUrl = info.request.url;
      const stream = new ReadableStream<Uint8Array>({
        async start(controller) {
          const client = ServerSentEventClient.forStream(controller);
          const server = new ServerSentEventServer({
            url: upstreamUrl ?? requestUrl,
            client,
          });
          await resolver({
            request: info.request,
            params: info.params,
            cookies: info.cookies,
            requestId: info.requestId,
            client,
            server,
          });
        },
      });
      // Convert to the engine shape (the stash keeps the live stream).
      return normalizeResult(new Response(stream, SSE_RESPONSE_INIT));
    }
  );
  registerCollected(handler);
  return handler;
}
