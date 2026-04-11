/**
 * MockpitInterceptor -- core mock engine wired into fetch/XHR interception.
 *
 * Patches globalThis.fetch and XMLHttpRequest to route requests through
 * the Rust mock engine via matchRequest() NAPI call.
 *
 * Framework-specific adapters (Playwright, WDIO, etc.) use matchRequest()
 * or createHandler() to wire into their own interception systems.
 */

import type { JsHandler } from "@mockpit/node";

const originalFetch = globalThis.fetch;
const OriginalXHR = typeof XMLHttpRequest !== "undefined" ? XMLHttpRequest : null;
let activeInterceptor: MockpitInterceptor | null = null;

export class MockpitInterceptor {
  private server: any;
  private applied = false;

  constructor() {
    const { MockpitServer } = require("@mockpit/node");
    this.server = new MockpitServer();
  }

  // ===== Mock registration =====

  async loadMocks(dir: string): Promise<number> {
    return this.server.loadMocks(dir);
  }

  async loadMockFile(file: string): Promise<number> {
    return this.server.loadMockFile(file);
  }

  useHandlers(handlers: JsHandler[]): void {
    this.server.useHandlers(handlers);
  }

  resetHandlers(): void {
    this.server.resetHandlers();
  }

  async addMock(config: any): Promise<string> {
    return this.server.addMock(config);
  }

  get mockCount(): number {
    return this.server.mockCount;
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
    body: string;
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
   */
  apply(): void {
    if (this.applied) return;
    if (activeInterceptor) {
      throw new Error("Another MockpitInterceptor is already active.");
    }

    const self = this;

    // -- Patch fetch --
    globalThis.fetch = async function mockpitFetch(
      input: RequestInfo | URL,
      init?: RequestInit
    ): Promise<Response> {
      const request = new Request(input, init);
      const url = new URL(request.url);
      const method = request.method;
      const path = url.pathname;
      const query = url.search ? url.search.slice(1) : undefined;

      let body: string | undefined;
      if (!["GET", "HEAD", "OPTIONS"].includes(method)) {
        try {
          body = await request.clone().text();
        } catch {}
      }

      const headers: Record<string, string> = {};
      request.headers.forEach((v, k) => {
        headers[k] = v;
      });

      const match = await self.matchRequest(method, path, query, headers, body);

      if (match) {
        return new Response(match.body, {
          status: match.status,
          headers: new Headers(match.headers),
        });
      }

      return originalFetch(input, init);
    };

    // -- Patch XMLHttpRequest --
    if (OriginalXHR) {
      patchXHR(self);
    }

    activeInterceptor = this;
    this.applied = true;
  }

  /** Restore original fetch and XMLHttpRequest. */
  dispose(): void {
    if (!this.applied) return;
    globalThis.fetch = originalFetch;
    if (OriginalXHR) {
      (globalThis as any).XMLHttpRequest = OriginalXHR;
    }
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
          if (match) {
            Object.defineProperty(xhr, "readyState", {
              value: 4,
              writable: true,
              configurable: true,
            });
            Object.defineProperty(xhr, "status", {
              value: match.status,
              writable: true,
              configurable: true,
            });
            Object.defineProperty(xhr, "statusText", {
              value: "",
              writable: true,
              configurable: true,
            });
            Object.defineProperty(xhr, "responseText", {
              value: match.body,
              writable: true,
              configurable: true,
            });
            Object.defineProperty(xhr, "response", {
              value: match.body,
              writable: true,
              configurable: true,
            });

            const headerStr = Object.entries(match.headers)
              .map(([k, v]) => `${k}: ${v}`)
              .join("\r\n");
            xhr.getAllResponseHeaders = () => headerStr;
            xhr.getResponseHeader = (name: string) =>
              match.headers[name.toLowerCase()] ?? null;

            if (typeof xhr.onreadystatechange === "function") {
              xhr.onreadystatechange(new Event("readystatechange"));
            }
            xhr.dispatchEvent(new Event("readystatechange"));
            if (typeof xhr.onload === "function") {
              xhr.onload(new ProgressEvent("load"));
            }
            xhr.dispatchEvent(new ProgressEvent("load"));
            xhr.dispatchEvent(new ProgressEvent("loadend"));
          } else {
            origSend(body);
          }
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
