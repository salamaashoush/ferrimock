/**
 * MockpitInterceptor -- the mock engine wired into different interception targets.
 *
 * Supports:
 * - fetch() patching (Node.js unit tests -- jest, vitest, bun test)
 * - XMLHttpRequest patching (legacy code, Axios, jQuery.ajax)
 * - Playwright page.route() (browser-level interception)
 * - Any route-based API (WDIO, Cypress, custom)
 *
 * All modes use the same Rust mock engine via matchRequest() NAPI call.
 * No HTTP server needed.
 */

import type { JsHandler } from "@mockpit/node";

const originalFetch = globalThis.fetch;
const OriginalXHR = typeof XMLHttpRequest !== "undefined" ? XMLHttpRequest : null;
let activeInterceptor: MockpitInterceptor | null = null;

export class MockpitInterceptor {
  private server: any; // MockpitServer from @mockpit/node
  private applied = false;
  private routeDisposers: Array<() => Promise<void>> = [];

  constructor() {
    const { MockpitServer } = require("@mockpit/node");
    this.server = new MockpitServer();
  }

  // ===== Mock registration (same API as MockpitServer) =====

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
   * This is the primitive that all interception modes use.
   */
  async matchRequest(
    method: string,
    path: string,
    query?: string,
    headers?: Record<string, string>,
    body?: string
  ): Promise<{ status: number; headers: Record<string, string>; body: string; mockId: string } | null> {
    return this.server.matchRequest(method, path, query ?? null, headers ?? null, body ?? null);
  }

  // ===== Mode 1: Patch globalThis.fetch =====

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
        try { body = await request.clone().text(); } catch {}
      }

      const headers: Record<string, string> = {};
      request.headers.forEach((v, k) => { headers[k] = v; });

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
    if (this.applied) {
      globalThis.fetch = originalFetch;
      if (OriginalXHR) {
        (globalThis as any).XMLHttpRequest = OriginalXHR;
      }
      activeInterceptor = null;
      this.applied = false;
    }
    for (const disposer of this.routeDisposers) {
      disposer().catch(() => {});
    }
    this.routeDisposers = [];
  }

  // ===== Mode 2: Playwright page.route() =====

  /**
   * Wire mockpit into a Playwright Page.
   * Intercepts all requests at the browser level.
   *
   * ```ts
   * const mocks = new MockpitInterceptor()
   * await mocks.loadMocks('./mocks')
   *
   * test('user page', async ({ page }) => {
   *   await mocks.routePage(page)
   *   await page.goto('http://localhost:3000')
   *   // All matching requests are mocked at browser level
   * })
   * ```
   */
  async routePage(page: any): Promise<void> {
    const self = this;

    await page.route("**/*", async (route: any) => {
      const request = route.request();
      const url = new URL(request.url());
      const method = request.method();
      const path = url.pathname;
      const query = url.search ? url.search.slice(1) : undefined;

      // Get headers from Playwright request
      const allHeaders = await request.allHeaders();
      const headers: Record<string, string> = {};
      for (const [k, v] of Object.entries(allHeaders)) {
        headers[k] = String(v);
      }

      // Get body
      let body: string | undefined;
      try { body = request.postData() ?? undefined; } catch {}

      const match = await self.matchRequest(method, path, query, headers, body);

      if (match) {
        await route.fulfill({
          status: match.status,
          headers: match.headers,
          body: match.body,
        });
      } else {
        await route.continue();
      }
    });

    // Store disposer so dispose() can unroute
    this.routeDisposers.push(async () => {
      try { await page.unrouteAll(); } catch {}
    });
  }

  // ===== Mode 3: Playwright BrowserContext.route() =====

  /**
   * Wire mockpit into a Playwright BrowserContext.
   * Intercepts all requests for all pages in the context.
   *
   * ```ts
   * const mocks = new MockpitInterceptor()
   * await mocks.loadMocks('./mocks')
   *
   * test('app', async ({ context }) => {
   *   await mocks.routeContext(context)
   *   const page = await context.newPage()
   *   await page.goto('http://localhost:3000')
   * })
   * ```
   */
  async routeContext(context: any): Promise<void> {
    const self = this;

    await context.route("**/*", async (route: any) => {
      const request = route.request();
      const url = new URL(request.url());
      const method = request.method();
      const path = url.pathname;
      const query = url.search ? url.search.slice(1) : undefined;

      const allHeaders = await request.allHeaders();
      const headers: Record<string, string> = {};
      for (const [k, v] of Object.entries(allHeaders)) {
        headers[k] = String(v);
      }

      let body: string | undefined;
      try { body = request.postData() ?? undefined; } catch {}

      const match = await self.matchRequest(method, path, query, headers, body);

      if (match) {
        await route.fulfill({
          status: match.status,
          headers: match.headers,
          body: match.body,
        });
      } else {
        await route.continue();
      }
    });

    this.routeDisposers.push(async () => {
      try { await context.unrouteAll(); } catch {}
    });
  }

  // ===== Mode 4: Generic route handler (WDIO, Cypress, custom) =====

  /**
   * Get a generic route handler function that can be wired into any
   * interception system. Returns a function that takes a request-like
   * object and returns a response or null.
   *
   * ```ts
   * const handler = mocks.createHandler()
   *
   * // Wire into any system
   * const response = await handler({
   *   method: 'GET',
   *   url: 'http://localhost/api/users/42',
   * })
   * ```
   */
  createHandler(): (req: {
    method: string;
    url: string;
    headers?: Record<string, string>;
    body?: string;
  }) => Promise<{ status: number; headers: Record<string, string>; body: string } | null> {
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
    let _async = true;
    let _mocked = false;
    let _mockStatus = 200;
    let _mockHeaders: Record<string, string> = {};
    let _mockBody = "";

    // Proxy open()
    const origOpen = xhr.open.bind(xhr);
    xhr.open = function (method: string, url: string, async_?: boolean, ...rest: any[]) {
      _method = method;
      _url = url;
      _async = async_ !== false;
      _headers = {};
      return origOpen(method, url, async_, ...rest);
    };

    // Proxy setRequestHeader()
    const origSetHeader = xhr.setRequestHeader.bind(xhr);
    xhr.setRequestHeader = function (name: string, value: string) {
      _headers[name.toLowerCase()] = value;
      return origSetHeader(name, value);
    };

    // Proxy send()
    const origSend = xhr.send.bind(xhr);
    xhr.send = function (body?: any) {
      _body = body != null ? String(body) : undefined;

      // Try to match against the mock engine
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

      // Use matchRequest -- it's async so we need to handle it
      interceptor
        .matchRequest(_method, path, query, _headers, _body)
        .then((match) => {
          if (match) {
            _mocked = true;
            _mockStatus = match.status;
            _mockHeaders = match.headers;
            _mockBody = match.body;

            // Simulate XHR lifecycle
            Object.defineProperty(xhr, "readyState", { value: 4, writable: true, configurable: true });
            Object.defineProperty(xhr, "status", { value: match.status, writable: true, configurable: true });
            Object.defineProperty(xhr, "statusText", { value: "", writable: true, configurable: true });
            Object.defineProperty(xhr, "responseText", { value: match.body, writable: true, configurable: true });
            Object.defineProperty(xhr, "response", { value: match.body, writable: true, configurable: true });

            // Build getAllResponseHeaders string
            const headerStr = Object.entries(match.headers)
              .map(([k, v]) => `${k}: ${v}`)
              .join("\r\n");
            xhr.getAllResponseHeaders = () => headerStr;
            xhr.getResponseHeader = (name: string) =>
              match.headers[name.toLowerCase()] ?? null;

            // Fire events
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
            // No match -- send to real network
            origSend(body);
          }
        })
        .catch(() => {
          origSend(body);
        });
    };

    return xhr;
  } as any;

  // Copy static properties
  MockXHR.UNSENT = 0;
  MockXHR.OPENED = 1;
  MockXHR.HEADERS_RECEIVED = 2;
  MockXHR.LOADING = 3;
  MockXHR.DONE = 4;

  (globalThis as any).XMLHttpRequest = MockXHR;
}
