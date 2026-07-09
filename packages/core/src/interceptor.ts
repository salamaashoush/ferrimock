/**
 * MockpitInterceptor -- core mock engine wired into fetch/XHR interception.
 *
 * Patches globalThis.fetch and XMLHttpRequest to route requests through
 * the Rust mock engine via matchRequest() NAPI call.
 *
 * Framework-specific adapters (Playwright, WDIO, etc.) use matchRequest()
 * or createHandler() to wire into their own interception systems.
 */

import { MockpitServer } from "@mockpit/node";
import type { RequestHandler } from "@mockpit/node";
import { ClientRequestInterceptor } from "@mswjs/interceptors/ClientRequest";
import {
  WebSocketInterceptor,
  type WebSocketConnectionData,
} from "@mswjs/interceptors/WebSocket";
import { WebSocketHandler, getWsHandler, isWsHandler, pruneWsHandlers } from "./ws.js";
import {
  BYPASS_HEADER,
  NETWORK_ERROR_HEADER,
  PASSTHROUGH_HEADER,
  STREAM_ID_HEADER,
} from "./msw-compat.js";
import { setInterceptorActive, takeResponse } from "./stream-stash.js";
import { mergeStoredCookies, storeResponseCookies } from "./cookie-store.js";
import { LifecycleEvents } from "./events.js";

interface MatchedResult {
  status: number;
  statusText?: string;
  headers: Record<string, string>;
  body: Uint8Array;
  mockId: string;
}

type MatchOutcome =
  | { kind: "match"; response: MatchedResult }
  | { kind: "passthrough" }
  | { kind: "unhandled" };

/**
 * Build a Response from an engine match: applies statusText and splits
 * newline-joined Set-Cookie values back into separate headers.
 */
function toResponse(match: MatchedResult): Response {
  // Interceptor lane: the handler's original Response was stashed whole
  // (live streams included) — deliver it instead of a byte-copy rebuild.
  const stashed = takeResponse(match.headers[STREAM_ID_HEADER]);
  if (stashed) {
    return stashed;
  }
  const headers = new Headers();
  for (const [key, value] of Object.entries(match.headers)) {
    if (key === STREAM_ID_HEADER) continue;
    if (key === "set-cookie" && value.includes("\n")) {
      for (const cookie of value.split("\n")) {
        if (cookie) headers.append("set-cookie", cookie);
      }
    } else {
      headers.set(key, value);
    }
  }
  const nullBodyStatus =
    match.status === 204 || match.status === 205 || match.status === 304;
  return new Response(nullBodyStatus ? null : match.body, {
    status: match.status,
    statusText: match.statusText ?? "",
    headers,
  });
}

const originalFetch = globalThis.fetch;
const OriginalXHR = typeof XMLHttpRequest !== "undefined" ? XMLHttpRequest : null;
let activeInterceptor: MockpitInterceptor | null = null;

let requestCounter = 0;
function nextRequestId(): string {
  return `req:${(requestCounter++).toString(16)}`;
}

const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);
const MAX_REDIRECTS = 20;

const STATUS_TEXT: Record<number, string> = {
  200: "OK", 201: "Created", 202: "Accepted", 204: "No Content",
  301: "Moved Permanently", 302: "Found", 303: "See Other",
  304: "Not Modified", 307: "Temporary Redirect", 308: "Permanent Redirect",
  400: "Bad Request", 401: "Unauthorized", 403: "Forbidden", 404: "Not Found",
  405: "Method Not Allowed", 409: "Conflict", 422: "Unprocessable Entity",
  429: "Too Many Requests", 500: "Internal Server Error", 502: "Bad Gateway",
  503: "Service Unavailable", 504: "Gateway Timeout",
};

/** A promise that rejects with an AbortError when the signal aborts. */
function abortRejection(signal: AbortSignal): Promise<never> {
  return new Promise((_, reject) => {
    signal.addEventListener(
      "abort",
      () =>
        reject(
          new DOMException("The operation was aborted.", "AbortError")
        ),
      { once: true }
    );
  });
}

/** Everything setupServer-style APIs accept: engine handlers plus the
 * `ws.link` wrapper (whose engine mock rides in `.native`). */
export type AnyHandler = RequestHandler | WebSocketHandler;

/** Unwrap ws handlers to their engine mocks; everything registers
 * through the engine's scopes identically. */
function toEngineHandlers(handlers: AnyHandler[]): RequestHandler[] {
  return handlers.map((handler) =>
    isWsHandler(handler) ? handler.native : (handler as RequestHandler)
  );
}

export type UnhandledRequestStrategy =
  | "bypass"
  | "warn"
  | "error"
  | ((request: Request, print: { warning(): void; error(): void }) => void);

export interface ApplyOptions {
  onUnhandledRequest?: UnhandledRequestStrategy;
}

export class MockpitInterceptor {
  private server: MockpitServer;
  private applied = false;
  private onUnhandledRequest: UnhandledRequestStrategy = "bypass";
  // Intercepts Node http/https ClientRequest (axios default adapter, got,
  // node-fetch v2, etc.). global fetch (undici) + XHR use the patches below.
  private clientRequest: ClientRequestInterceptor | null = null;
  // Patches globalThis.WebSocket (applied unconditionally, MSW parity:
  // unhandled connections must hit the onUnhandledRequest strategy even
  // with zero ws handlers registered).
  private webSocket: WebSocketInterceptor | null = null;

  // Hot-path state cached on the JS side, refreshed only when mocks change.
  // Avoids a NAPI getter crossing on every intercepted request.
  private cachedMockCount = 0;
  private cachedNeedsBody = false;
  private cachedNeedsHeaders = false;

  /** MSW-compatible lifecycle events. */
  readonly events = new LifecycleEvents();

  constructor() {
    this.server = new MockpitServer();
  }

  /** Refresh hot-path state after any mock mutation. */
  private syncState(): void {
    this.cachedMockCount = this.server.mockCount;
    this.cachedNeedsBody = this.server.needsRequestBody;
    this.cachedNeedsHeaders = this.server.needsRequestHeaders;
  }

  // ===== Mock registration =====

  async loadMocks(dir: string): Promise<number> {
    const n = await this.server.loadMocks(dir);
    this.syncState();
    return n;
  }

  async loadMockFile(file: string): Promise<number> {
    const n = await this.server.loadMockFile(file);
    this.syncState();
    return n;
  }

  useHandlers(handlers: AnyHandler[]): void {
    if (handlers.length > 0) {
      this.server.useHandlers(toEngineHandlers(handlers));
    }
    this.syncState();
  }

  /**
   * MSW's `server.resetHandlers()`: remove runtime handlers added via
   * `use()` and restore initial handlers. With arguments, replaces the
   * entire handler set with the given handlers.
   */
  resetHandlers(...nextHandlers: AnyHandler[]): void {
    if (nextHandlers.length > 0) {
      this.server.resetHandlers();
      this.server.useHandlers(toEngineHandlers(nextHandlers));
    } else {
      this.server.resetRuntimeHandlers();
    }
    this.pruneWsDispatch();
    this.syncState();
  }

  /** Drop ws dispatch entries whose engine mocks are gone. */
  private pruneWsDispatch(): void {
    pruneWsHandlers(new Set(this.server.listHandlers().map((h) => h.id)));
  }

  async addMock(config: any): Promise<string> {
    const id = await this.server.addMock(config);
    this.syncState();
    return id;
  }

  get mockCount(): number {
    return this.server.mockCount;
  }

  /** Whether any registered mock matches on the request body. */
  get needsRequestBody(): boolean {
    return this.server.needsRequestBody;
  }

  // ===== Core: match a request against the engine =====

  /**
   * Match a request against the mock engine. Returns the response or null.
   * This is the primitive that all interception modes and adapters use.
   *
   * A handler returning undefined falls through: matching retries with
   * that mock excluded until a candidate responds or none are left (MSW
   * semantics). `passthrough()` resolves to null — perform the real request.
   */
  async matchRequest(
    method: string,
    path: string,
    query?: string,
    headers?: Record<string, string>,
    body?: string | Uint8Array,
    requestId?: string
  ): Promise<MatchedResult | null> {
    const outcome = await this.resolveRequest(
      method,
      path,
      query,
      headers,
      body,
      requestId
    );
    if (outcome.kind !== "match") {
      return null;
    }
    return this.bufferMatch(outcome.response);
  }

  /**
   * Adapters (XHR, Playwright) consume plain bytes: buffer a stashed
   * Response. The fetch/ClientRequest patches keep the live stream via
   * toResponse instead.
   * @internal
   */
  async bufferMatch(match: MatchedResult): Promise<MatchedResult> {
    const stashed = takeResponse(match.headers[STREAM_ID_HEADER]);
    if (stashed) {
      delete match.headers[STREAM_ID_HEADER];
      match.body = new Uint8Array(await stashed.arrayBuffer());
      match.statusText ??= stashed.statusText || undefined;
    }
    return match;
  }

  /**
   * matchRequest plus the distinction the fetch/XHR patches need:
   * passthrough (real request, no unhandled warning) vs unhandled.
   * @internal
   */
  async resolveRequest(
    method: string,
    path: string,
    query?: string,
    headers?: Record<string, string>,
    body?: string | Uint8Array,
    requestId?: string,
    host?: string
  ): Promise<MatchOutcome> {
    // Virtual cookie jar (MSW parity): stored cookies ride along on the
    // Cookie header so resolvers see them in `cookies`.
    const cookieHost = host ?? headers?.host;
    if (cookieHost) {
      const cookie = mergeStoredCookies(cookieHost, headers?.cookie);
      if (cookie !== undefined && cookie !== headers?.cookie) {
        headers = { ...(headers ?? { host: cookieHost }), cookie };
      }
    }

    let excludeIds: string[] | null = null;
    for (;;) {
      const match = await this.server.matchRequest(
        method,
        path,
        query ?? null,
        headers ?? null,
        body ?? null,
        requestId ?? null,
        excludeIds
      );
      if (!match) {
        return { kind: "unhandled" };
      }
      if (match.fallthrough) {
        (excludeIds ??= []).push(match.mockId);
        continue;
      }
      if (match.headers[PASSTHROUGH_HEADER] === "1") {
        return { kind: "passthrough" };
      }
      if (cookieHost) {
        const setCookie = match.headers["set-cookie"];
        if (setCookie) {
          storeResponseCookies(cookieHost, setCookie.split("\n"));
        }
      }
      return { kind: "match", response: match };
    }
  }

  // ===== Patch fetch + XHR =====

  /**
   * Patch globalThis.fetch and XMLHttpRequest to intercept requests.
   * For Node.js unit tests (jest, vitest, bun test).
   *
   * @param options - Optional configuration (e.g., `onUnhandledRequest`).
   */
  apply(options?: ApplyOptions): void {
    if (this.applied) return;
    if (activeInterceptor) {
      throw new Error("Another MockpitInterceptor is already active.");
    }

    if (options?.onUnhandledRequest) {
      this.onUnhandledRequest = options.onUnhandledRequest;
    }

    const self = this;

    // -- Patch fetch --
    globalThis.fetch = async function mockpitFetch(
      input: RequestInfo | URL,
      init?: RequestInit
    ): Promise<Response> {
      const request = new Request(input, init);
      const requestId = nextRequestId();

      // Bypass: requests marked with bypass header skip interception
      if (request.headers.has(BYPASS_HEADER)) {
        request.headers.delete(BYPASS_HEADER);
        return originalFetch(request);
      }

      self.events.emit("request:start", { request, requestId });

      // Honor an already-aborted signal before doing any work.
      if (request.signal?.aborted) {
        self.events.emit("request:end", { request, requestId });
        throw new DOMException("The operation was aborted.", "AbortError");
      }

      const redirectMode = request.redirect ?? "follow";
      let currentRequest = request;

      // Redirect-following loop: a mocked 3xx with `redirect: 'follow'` (the
      // default) re-enters matching against the Location target.
      for (let hop = 0; ; hop++) {
        // Skip all match work when nothing is registered — the request is
        // guaranteed unhandled. Reads cached state (no per-request NAPI getter).
        let outcome: MatchOutcome = { kind: "unhandled" };
        if (self.cachedMockCount > 0) {
          const url = new URL(currentRequest.url);
          const method = currentRequest.method;
          const path = url.pathname;
          const query = url.search ? url.search.slice(1) : undefined;

          // Buffer the request body only when a mock could match on it.
          // Bytes, not text: binary bodies must survive the NAPI crossing.
          let body: Uint8Array | undefined;
          if (
            method !== "GET" &&
            method !== "HEAD" &&
            method !== "OPTIONS" &&
            self.cachedNeedsBody
          ) {
            try {
              body = new Uint8Array(await currentRequest.clone().arrayBuffer());
            } catch {}
          }

          // Marshal headers only when a mock could use them. The Host
          // header is not on Request.headers — inject it from the URL so
          // absolute-URL predicates (host matchers) work.
          let headers: Record<string, string> | undefined;
          if (self.cachedNeedsHeaders) {
            headers = { host: url.host };
            currentRequest.headers.forEach((v, k) => {
              headers![k] = v;
            });
          }

          const matchPromise = self.resolveRequest(
            method,
            path,
            query,
            headers,
            body,
            requestId,
            url.host
          );
          // Reject as soon as the caller aborts, even mid-flight (delayed mocks).
          try {
            outcome = currentRequest.signal
              ? await Promise.race([
                  matchPromise,
                  abortRejection(currentRequest.signal),
                ])
              : await matchPromise;
          } catch (error) {
            if (error instanceof DOMException && error.name === "AbortError") {
              self.events.emit("request:end", { request, requestId });
              throw error;
            }
            // Handler threw: emit unhandledException and respond 500
            // (MSW's default unhandled-exception strategy).
            self.events.emit("unhandledException", {
              request,
              requestId,
              error: error as Error,
            });
            self.events.emit("request:end", { request, requestId });
            const err = error as Error;
            return new Response(
              JSON.stringify({
                name: err.name ?? "Error",
                message: err.message ?? String(error),
                stack: err.stack,
              }),
              {
                status: 500,
                statusText: "Unhandled Exception",
                headers: { "content-type": "application/json" },
              }
            );
          }
        }

        if (outcome.kind === "passthrough") {
          const response = await originalFetch(currentRequest);
          self.events.emit("response:bypass", { request, requestId, response });
          self.events.emit("request:end", { request, requestId });
          return response;
        }

        if (outcome.kind === "unhandled") {
          // Unhandled — pass through the already-built request (preserves body).
          self.events.emit("request:unhandled", { request, requestId });
          self.handleUnhandled(currentRequest);
          const response = await originalFetch(currentRequest);
          self.events.emit("response:bypass", { request, requestId, response });
          self.events.emit("request:end", { request, requestId });
          return response;
        }

        const match = outcome.response;
        self.events.emit("request:match", { request, requestId });

        // Network error simulation
        if (match.headers[NETWORK_ERROR_HEADER] === "1") {
          self.events.emit("request:end", { request, requestId });
          throw new TypeError("Failed to fetch");
        }

        const response = toResponse(match);

        const location = response.headers.get("location");
        if (REDIRECT_STATUSES.has(match.status) && location) {
          if (redirectMode === "error") {
            self.events.emit("request:end", { request, requestId });
            throw new TypeError("Failed to fetch: unexpected redirect");
          }
          if (redirectMode === "follow") {
            if (hop >= MAX_REDIRECTS) {
              self.events.emit("request:end", { request, requestId });
              throw new TypeError("Failed to fetch: too many redirects");
            }
            const nextUrl = new URL(location, currentRequest.url).toString();
            // 307/308 preserve method+body; 301/302/303 become GET.
            const keepMethod = match.status === 307 || match.status === 308;
            const nextMethod = keepMethod ? currentRequest.method : "GET";
            const nextInit: RequestInit = {
              method: nextMethod,
              headers: currentRequest.headers,
              redirect: redirectMode,
              signal: currentRequest.signal,
            };
            if (keepMethod && nextMethod !== "GET" && nextMethod !== "HEAD") {
              try {
                nextInit.body = await currentRequest.clone().text();
              } catch {}
            }
            currentRequest = new Request(nextUrl, nextInit);
            continue; // re-match the redirect target
          }
          // redirectMode === "manual": return the 3xx response as-is.
        }

        self.events.emit("response:mocked", { request, requestId, response });
        self.events.emit("request:end", { request, requestId });
        return response;
      }
    };

    // -- Patch XMLHttpRequest --
    if (OriginalXHR) {
      patchXHR(self);
    }

    // -- Intercept Node http/https ClientRequest --
    // The http client follows redirects itself (re-entering this interceptor),
    // so this handler only needs match → respond / passthrough.
    const clientRequest = new ClientRequestInterceptor();
    clientRequest.on("request", async ({ request, controller, requestId }) => {
      if (request.headers.has(BYPASS_HEADER)) {
        request.headers.delete(BYPASS_HEADER);
        return; // passthrough to the real network
      }

      self.events.emit("request:start", { request, requestId });

      const url = new URL(request.url);
      const method = request.method;

      const unhandled = () => {
        self.events.emit("request:unhandled", { request, requestId });
        try {
          self.handleUnhandled(request);
        } catch (error) {
          self.events.emit("request:end", { request, requestId });
          controller.errorWith(error as Error);
        }
      };

      if (self.cachedMockCount === 0) {
        unhandled();
        return;
      }

      let body: Uint8Array | undefined;
      if (
        method !== "GET" &&
        method !== "HEAD" &&
        method !== "OPTIONS" &&
        self.cachedNeedsBody
      ) {
        try {
          body = new Uint8Array(await request.clone().arrayBuffer());
        } catch {}
      }
      let headers: Record<string, string> | undefined;
      if (self.cachedNeedsHeaders) {
        headers = { host: url.host };
        request.headers.forEach((v, k) => {
          headers![k] = v;
        });
      }

      let outcome: MatchOutcome;
      try {
        outcome = await self.resolveRequest(
          method,
          url.pathname,
          url.search ? url.search.slice(1) : undefined,
          headers,
          body,
          requestId,
          url.host
        );
      } catch (error) {
        self.events.emit("unhandledException", {
          request,
          requestId,
          error: error as Error,
        });
        self.events.emit("request:end", { request, requestId });
        controller.respondWith(
          new Response(
            JSON.stringify({
              name: (error as Error).name ?? "Error",
              message: (error as Error).message ?? String(error),
              stack: (error as Error).stack,
            }),
            {
              status: 500,
              statusText: "Unhandled Exception",
              headers: { "content-type": "application/json" },
            }
          )
        );
        return;
      }

      if (outcome.kind === "passthrough") {
        return; // real network; the "response" hook emits response:bypass
      }
      if (outcome.kind === "unhandled") {
        unhandled();
        return;
      }

      const match = outcome.response;
      self.events.emit("request:match", { request, requestId });

      if (match.headers[NETWORK_ERROR_HEADER] === "1") {
        self.events.emit("request:end", { request, requestId });
        controller.errorWith(new TypeError("Failed to fetch"));
        return;
      }
      controller.respondWith(toResponse(match));
    });
    // Response hook fires for both mocked and passthrough responses —
    // the single place response:mocked/response:bypass + request:end can
    // cover every ClientRequest outcome.
    clientRequest.on(
      "response",
      ({ response, isMockedResponse, request, requestId }) => {
        self.events.emit(
          isMockedResponse ? "response:mocked" : "response:bypass",
          { request, requestId, response }
        );
        self.events.emit("request:end", { request, requestId });
      }
    );
    clientRequest.apply();
    this.clientRequest = clientRequest;

    // -- Intercept WebSocket connections --
    const webSocket = new WebSocketInterceptor();
    webSocket.on("connection", (connection) => {
      this.handleWsConnection(connection);
    });
    webSocket.apply();
    this.webSocket = webSocket;

    activeInterceptor = this;
    this.applied = true;
    setInterceptorActive(true);
  }

  private handleWsConnection(connection: WebSocketConnectionData): void {
    const clientUrl =
      connection.client.url instanceof URL
        ? new URL(connection.client.url.href)
        : new URL(String(connection.client.url));
    // MSW's socket.io compatibility rewrite.
    clientUrl.pathname = clientUrl.pathname.replace(/^\/socket\.io\//, "/");

    // The engine resolves which ws mocks match; every match's listeners
    // run against the same connection (MSW semantics).
    let matched = false;
    for (const match of this.server.matchWsConnections(clientUrl.href)) {
      const handler = getWsHandler(match.mockId);
      if (handler) {
        handler.run(connection, match.params);
        matched = true;
      }
    }
    if (matched) {
      return;
    }

    // Unhandled connection: run the strategy against a synthetic
    // upgrade request (MSW shape), then auto-passthrough.
    const request = new Request(connection.client.url, {
      headers: { connection: "upgrade", upgrade: "websocket" },
    });
    try {
      this.handleUnhandled(request);
    } catch (error) {
      const errorEvent = new Event("error");
      Object.defineProperty(errorEvent, "cause", {
        enumerable: true,
        configurable: false,
        value: error,
      });
      connection.client.socket.dispatchEvent(errorEvent);
      return; // "error" strategy: no passthrough
    }
    connection.server.connect();
  }

  // ===== MSW-compatible server methods =====

  /**
   * Add runtime handlers (MSW's `server.use()`).
   * Runtime handlers take priority over initial handlers.
   */
  use(...handlers: AnyHandler[]): void {
    if (handlers.length > 0) {
      this.server.use(toEngineHandlers(handlers));
    }
    this.syncState();
  }

  /**
   * Re-enable consumed one-time handlers (MSW's `server.restoreHandlers()`).
   */
  restoreHandlers(): void {
    this.server.restoreHandlers();
    this.syncState();
  }

  /**
   * List all registered handlers (MSW's `server.listHandlers()`).
   */
  listHandlers(): Array<{
    id: string;
    methods: string[];
    enabled: boolean;
    kind?: "websocket";
  }> {
    return this.server.listHandlers() as Array<{
      id: string;
      methods: string[];
      enabled: boolean;
      kind?: "websocket";
    }>;
  }

  /**
   * Create an isolated handler scope (MSW's `server.boundary()`).
   *
   * Handlers added inside the boundary callback are automatically removed
   * when the callback settles (async callbacks clean up after the
   * returned promise resolves or rejects, not when it is created).
   */
  boundary<Args extends any[], R>(
    callback: (...args: Args) => R
  ): (...args: Args) => R {
    return (...args: Args): R => {
      // Snapshot current handler IDs
      const before = new Set(this.server.listHandlers().map((h) => h.id));

      const cleanup = () => {
        // Remove handlers that were added during the callback
        for (const handler of this.server.listHandlers()) {
          if (!before.has(handler.id)) {
            this.server.removeMock(handler.id);
          }
        }
        this.pruneWsDispatch();
        this.syncState();
      };

      let result: R;
      try {
        result = callback(...args);
      } catch (error) {
        cleanup();
        throw error;
      }

      if (
        result !== null &&
        typeof result === "object" &&
        typeof (result as PromiseLike<unknown>).then === "function"
      ) {
        return (result as unknown as Promise<unknown>).finally(
          cleanup
        ) as unknown as R;
      }
      cleanup();
      return result;
    };
  }

  // ===== Unhandled request handling =====

  /** @internal */
  handleUnhandled(request: Request): void {
    const strategy = this.onUnhandledRequest;
    const msg = `[mockpit] Unhandled ${request.method} ${request.url}`;

    if (strategy === "bypass") return;
    if (strategy === "warn") {
      console.warn(msg);
      return;
    }
    if (strategy === "error") {
      throw new Error(msg);
    }
    if (typeof strategy === "function") {
      strategy(request, {
        warning() { console.warn(msg); },
        error() { throw new Error(msg); },
      });
    }
  }

  /** Restore original fetch, XMLHttpRequest, and WebSocket. */
  dispose(): void {
    if (!this.applied) return;
    globalThis.fetch = originalFetch;
    if (OriginalXHR) {
      (globalThis as any).XMLHttpRequest = OriginalXHR;
    }
    this.clientRequest?.dispose();
    this.clientRequest = null;
    this.webSocket?.dispose();
    this.webSocket = null;
    activeInterceptor = null;
    this.applied = false;
    setInterceptorActive(false);
  }

  // ===== Generic handler for adapters =====

  /**
   * Get a generic handler function for use by framework adapters.
   * Takes a request-like object, returns a response or null.
   */
  createHandler(): (req: {
    method: string;
    url: string;
    headers?: Record<string, string>;
    body?: string | Uint8Array;
  }) => Promise<MatchedResult | null> {
    const self = this;
    return async (req) => {
      const url = new URL(req.url);
      const headers = req.headers ? { host: url.host, ...req.headers } : undefined;
      return self.matchRequest(
        req.method,
        url.pathname,
        url.search ? url.search.slice(1) : undefined,
        headers,
        req.body
      );
    };
  }
}

// ===== XMLHttpRequest patching =====

function patchXHR(interceptor: MockpitInterceptor): void {
  if (!OriginalXHR) return;

  const MockXHR = function (this: any) {
    const xhr = new OriginalXHR!();
    let _method = "GET";
    let _url = "";
    let _headers: Record<string, string> = {};
    let _body: string | Uint8Array | undefined;

    const origOpen = xhr.open.bind(xhr);
    xhr.open = function (
      method: string,
      url: string,
      async_?: boolean,
      ...rest: any[]
    ) {
      _method = method;
      _url = url;
      _headers = {};
      return origOpen(method, url, async_, ...rest);
    };

    const origSetHeader = xhr.setRequestHeader.bind(xhr);
    xhr.setRequestHeader = function (name: string, value: string) {
      _headers[name.toLowerCase()] = value;
      return origSetHeader(name, value);
    };

    const origSend = xhr.send.bind(xhr);
    xhr.send = function (body?: any) {
      if (body == null) {
        _body = undefined;
      } else if (body instanceof ArrayBuffer) {
        _body = new Uint8Array(body);
      } else if (ArrayBuffer.isView(body)) {
        _body = new Uint8Array(body.buffer, body.byteOffset, body.byteLength);
      } else {
        _body = String(body);
      }

      let url: URL;
      try {
        url = new URL(_url);
      } catch {
        try {
          url = new URL(_url, "http://localhost");
        } catch {
          return origSend(body);
        }
      }

      const path = url.pathname;
      const query = url.search ? url.search.slice(1) : undefined;
      _headers["host"] ??= url.host;

      const requestId = nextRequestId();
      const eventsRequest = new Request(url.toString(), {
        method: _method,
        headers: _headers,
        body:
          _body !== undefined && _method !== "GET" && _method !== "HEAD"
            ? _body
            : undefined,
      });
      interceptor.events.emit("request:start", {
        request: eventsRequest,
        requestId,
      });

      interceptor
        .resolveRequest(_method, path, query, _headers, _body, requestId, url.host)
        .then(async (outcome) => {
          if (outcome.kind === "passthrough") {
            interceptor.events.emit("request:end", {
              request: eventsRequest,
              requestId,
            });
            origSend(body);
            return;
          }
          if (outcome.kind === "unhandled") {
            interceptor.events.emit("request:unhandled", {
              request: eventsRequest,
              requestId,
            });
            try {
              interceptor.handleUnhandled(eventsRequest);
            } catch {
              interceptor.events.emit("request:end", {
                request: eventsRequest,
                requestId,
              });
              xhr.dispatchEvent(new ProgressEvent("error"));
              xhr.dispatchEvent(new ProgressEvent("loadend"));
              return;
            }
            interceptor.events.emit("request:end", {
              request: eventsRequest,
              requestId,
            });
            origSend(body);
            return;
          }

          const match = await interceptor.bufferMatch(outcome.response);
          interceptor.events.emit("request:match", {
            request: eventsRequest,
            requestId,
          });

          if (match.headers[NETWORK_ERROR_HEADER] === "1") {
            interceptor.events.emit("request:end", {
              request: eventsRequest,
              requestId,
            });
            xhr.dispatchEvent(new ProgressEvent("error"));
            xhr.dispatchEvent(new ProgressEvent("loadend"));
            return;
          }

          const setProp = (k: string, v: unknown) =>
            Object.defineProperty(xhr, k, {
              value: v,
              writable: true,
              configurable: true,
            });

          setProp("status", match.status);
          setProp(
            "statusText",
            match.statusText ?? STATUS_TEXT[match.status] ?? ""
          );

          const headerStr = Object.entries(match.headers)
            .map(([k, v]) => `${k}: ${v}`)
            .join("\r\n");
          xhr.getAllResponseHeaders = () => headerStr;
          xhr.getResponseHeader = (name: string) =>
            match.headers[name.toLowerCase()] ?? null;

          // Honor responseType for `response`; `responseText` is text-only.
          const responseType: string = xhr.responseType || "";
          const isText = responseType === "" || responseType === "text";
          const bodyText =
            isText || responseType === "json"
              ? new TextDecoder().decode(match.body)
              : "";
          setProp("responseText", isText ? bodyText : "");

          let responseValue: unknown;
          switch (responseType) {
            case "json":
              try {
                responseValue = JSON.parse(bodyText);
              } catch {
                responseValue = null;
              }
              break;
            case "arraybuffer":
              responseValue = match.body.buffer.slice(
                match.body.byteOffset,
                match.body.byteOffset + match.body.byteLength
              );
              break;
            case "blob":
              responseValue = new Blob([match.body], {
                type: match.headers["content-type"] ?? "",
              });
              break;
            default:
              responseValue = bodyText;
          }
          setProp("response", responseValue);

          // Progress through the readyState lifecycle, dispatching once each.
          // dispatchEvent also invokes the matching on* handler (event-handler
          // IDL attribute), so we do NOT call on* explicitly (avoids double-fire).
          for (const rs of [2, 3, 4]) {
            setProp("readyState", rs);
            xhr.dispatchEvent(new Event("readystatechange"));
          }
          xhr.dispatchEvent(new ProgressEvent("load"));
          xhr.dispatchEvent(new ProgressEvent("loadend"));

          interceptor.events.emit("response:mocked", {
            request: eventsRequest,
            requestId,
            response: toResponse(match),
          });
          interceptor.events.emit("request:end", {
            request: eventsRequest,
            requestId,
          });
        })
        .catch((error) => {
          interceptor.events.emit("unhandledException", {
            request: eventsRequest,
            requestId,
            error: error as Error,
          });
          interceptor.events.emit("request:end", {
            request: eventsRequest,
            requestId,
          });
          origSend(body);
        });
    };

    return xhr;
  } as any;

  MockXHR.UNSENT = 0;
  MockXHR.OPENED = 1;
  MockXHR.HEADERS_RECEIVED = 2;
  MockXHR.LOADING = 3;
  MockXHR.DONE = 4;

  (globalThis as any).XMLHttpRequest = MockXHR;
}
