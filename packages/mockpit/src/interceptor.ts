/**
 * MockpitInterceptor -- the mock engine wired into different interception targets.
 *
 * Supports:
 * - fetch() patching (Node.js unit tests -- jest, vitest, bun test)
 * - Playwright page.route() (browser-level interception)
 * - Any route-based API (WDIO, Cypress, custom)
 *
 * All modes use the same Rust mock engine via matchRequest() NAPI call.
 * No HTTP server needed.
 */

import type { JsHandler } from "@mockpit/node";

const originalFetch = globalThis.fetch;
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
   * Patch globalThis.fetch to intercept requests.
   * For Node.js unit tests (jest, vitest, bun test).
   */
  apply(): void {
    if (this.applied) return;
    if (activeInterceptor) {
      throw new Error("Another MockpitInterceptor is already active.");
    }

    const self = this;

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

    activeInterceptor = this;
    this.applied = true;
  }

  /** Restore original fetch. */
  dispose(): void {
    if (this.applied) {
      globalThis.fetch = originalFetch;
      activeInterceptor = null;
      this.applied = false;
    }
    // Also clean up any page routes
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
