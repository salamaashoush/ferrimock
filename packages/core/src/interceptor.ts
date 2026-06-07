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
import type { JsHandler } from "@mockpit/node";
import { ClientRequestInterceptor } from "@mswjs/interceptors/ClientRequest";
import { BYPASS_HEADER, NETWORK_ERROR_HEADER } from "./msw-compat.js";
import { LifecycleEvents } from "./events.js";

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

  // Hot-path state cached on the JS side, refreshed only when mocks change.
  // Avoids a NAPI getter crossing on every intercepted request.
  private cachedMockCount = 0;
  private cachedNeedsBody = false;

  /** MSW-compatible lifecycle events. */
  readonly events = new LifecycleEvents();

  constructor() {
    this.server = new MockpitServer();
  }

  /** Refresh hot-path state after any mock mutation. */
  private syncState(): void {
    this.cachedMockCount = this.server.mockCount;
    this.cachedNeedsBody = this.server.needsRequestBody;
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

  useHandlers(handlers: JsHandler[]): void {
    this.server.useHandlers(handlers);
    this.syncState();
  }

  resetHandlers(): void {
    this.server.resetHandlers();
    this.syncState();
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
   */
  async matchRequest(
    method: string,
    path: string,
    query?: string,
    headers?: Record<string, string>,
    body?: string
  ): Promise<{
    status: number;
    headers: Record<string, string>;
    body: Uint8Array;
    mockId: string;
  } | null> {
    return this.server.matchRequest(
      method,
      path,
      query ?? null,
      headers ?? null,
      body ?? null
    );
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
        let match = null;
        if (self.cachedMockCount > 0) {
          const url = new URL(currentRequest.url);
          const method = currentRequest.method;
          const path = url.pathname;
          const query = url.search ? url.search.slice(1) : undefined;

          // Buffer the request body only when a mock could match on it.
          let body: string | undefined;
          if (
            method !== "GET" &&
            method !== "HEAD" &&
            method !== "OPTIONS" &&
            self.cachedNeedsBody
          ) {
            try {
              body = await currentRequest.clone().text();
            } catch {}
          }

          const headers: Record<string, string> = {};
          currentRequest.headers.forEach((v, k) => {
            headers[k] = v;
          });

          const matchPromise = self.matchRequest(method, path, query, headers, body);
          // Reject as soon as the caller aborts, even mid-flight (delayed mocks).
          match = currentRequest.signal
            ? await Promise.race([matchPromise, abortRejection(currentRequest.signal)])
            : await matchPromise;
        }

        if (!match) {
          // Unhandled — pass through the already-built request (preserves body).
          self.events.emit("request:unhandled", { request, requestId });
          self.handleUnhandled(currentRequest);
          const response = await originalFetch(currentRequest);
          self.events.emit("response:bypass", { request, requestId, response });
          self.events.emit("request:end", { request, requestId });
          return response;
        }

        self.events.emit("request:match", { request, requestId });

        // Network error simulation
        if (match.headers[NETWORK_ERROR_HEADER] === "1") {
          self.events.emit("request:end", { request, requestId });
          throw new TypeError("Failed to fetch");
        }

        const response = new Response(match.body, {
          status: match.status,
          headers: new Headers(match.headers),
        });

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
    clientRequest.on("request", async ({ request, controller }) => {
      if (request.headers.has(BYPASS_HEADER)) {
        request.headers.delete(BYPASS_HEADER);
        return; // passthrough to the real network
      }
      if (self.cachedMockCount === 0) return;

      const url = new URL(request.url);
      const method = request.method;
      let body: string | undefined;
      if (
        method !== "GET" &&
        method !== "HEAD" &&
        method !== "OPTIONS" &&
        self.cachedNeedsBody
      ) {
        try {
          body = await request.clone().text();
        } catch {}
      }
      const headers: Record<string, string> = {};
      request.headers.forEach((v, k) => {
        headers[k] = v;
      });

      const match = await self.matchRequest(
        method,
        url.pathname,
        url.search ? url.search.slice(1) : undefined,
        headers,
        body
      );
      if (!match) return; // unhandled → real network

      if (match.headers[NETWORK_ERROR_HEADER] === "1") {
        controller.errorWith(new TypeError("Failed to fetch"));
        return;
      }
      controller.respondWith(
        new Response(match.body, {
          status: match.status,
          headers: new Headers(match.headers),
        })
      );
    });
    clientRequest.apply();
    this.clientRequest = clientRequest;

    activeInterceptor = this;
    this.applied = true;
  }

  // ===== MSW-compatible server methods =====

  /**
   * Add runtime handlers (MSW's `server.use()`).
   * Runtime handlers take priority over initial handlers.
   */
  use(...handlers: JsHandler[]): void {
    this.server.use(handlers);
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
  listHandlers(): Array<{ id: string; methods: string[]; enabled: boolean }> {
    return this.server.listHandlers();
  }

  /**
   * Create an isolated handler scope (MSW's `server.boundary()`).
   *
   * Handlers added inside the boundary callback are automatically removed
   * when the callback returns.
   */
  boundary<Args extends any[], R>(
    callback: (...args: Args) => R
  ): (...args: Args) => R {
    return (...args: Args): R => {
      // Snapshot current handler IDs
      const before = new Set(this.listHandlers().map((h) => h.id));

      try {
        return callback(...args);
      } finally {
        // Remove handlers that were added during the callback
        const after = this.listHandlers();
        for (const handler of after) {
          if (!before.has(handler.id)) {
            this.server.removeMock(handler.id);
          }
        }
        this.syncState();
      }
    };
  }

  // ===== Unhandled request handling =====

  private handleUnhandled(request: Request): void {
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

  /** Restore original fetch and XMLHttpRequest. */
  dispose(): void {
    if (!this.applied) return;
    globalThis.fetch = originalFetch;
    if (OriginalXHR) {
      (globalThis as any).XMLHttpRequest = OriginalXHR;
    }
    this.clientRequest?.dispose();
    this.clientRequest = null;
    activeInterceptor = null;
    this.applied = false;
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
    body?: string;
  }) => Promise<{
    status: number;
    headers: Record<string, string>;
    body: string;
  } | null> {
    const self = this;
    return async (req) => {
      const url = new URL(req.url);
      return self.matchRequest(
        req.method,
        url.pathname,
        url.search ? url.search.slice(1) : undefined,
        req.headers,
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
    let _body: string | undefined;

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
      _body = body != null ? String(body) : undefined;

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

      interceptor
        .matchRequest(_method, path, query, _headers, _body)
        .then((match) => {
          if (!match) {
            origSend(body);
            return;
          }

          const setProp = (k: string, v: unknown) =>
            Object.defineProperty(xhr, k, {
              value: v,
              writable: true,
              configurable: true,
            });

          setProp("status", match.status);
          setProp("statusText", STATUS_TEXT[match.status] ?? "");

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
        })
        .catch(() => {
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
