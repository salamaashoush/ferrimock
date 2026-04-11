/**
 * Fetch interceptor -- patches globalThis.fetch to route requests through
 * the mock engine without any real HTTP.
 *
 * Architecture:
 *   fetch() → interceptor → MockpitServer.matchRequest() (NAPI) → Response
 *
 * No TCP, no axum, no tokio for the request path.
 * Declarative mocks are matched and rendered entirely in Rust.
 * JS handlers run on the JS thread (TSFN still needed for those).
 * Unmatched requests pass through to the real network.
 */

import type { JsHandler } from "@mockpit/node";

const originalFetch = globalThis.fetch;
let activeInterceptor: MockpitInterceptor | null = null;

export class MockpitInterceptor {
  private server: any; // MockpitServer instance
  private applied = false;

  constructor() {
    const { MockpitServer } = require("@mockpit/node");
    this.server = new MockpitServer();
  }

  /** Load declarative mocks from a directory (YAML/JSON/HAR). */
  async loadMocks(dir: string): Promise<number> {
    return this.server.loadMocks(dir);
  }

  /** Load a single mock file. */
  async loadMockFile(file: string): Promise<number> {
    return this.server.loadMockFile(file);
  }

  /** Register JS handler mocks. */
  useHandlers(handlers: JsHandler[]): void {
    this.server.useHandlers(handlers);
  }

  /** Reset handler-based mocks. */
  resetHandlers(): void {
    this.server.resetHandlers();
  }

  /** Add a declarative mock via JSON config. */
  async addMock(config: any): Promise<string> {
    return this.server.addMock(config);
  }

  /** Number of registered mocks. */
  get mockCount(): number {
    return this.server.mockCount;
  }

  /**
   * Start intercepting fetch requests.
   * After this, fetch() routes through the Rust mock engine -- no real HTTP.
   */
  apply(): void {
    if (this.applied) return;
    if (activeInterceptor) {
      throw new Error(
        "Another MockpitInterceptor is already active. Call dispose() first."
      );
    }

    const server = this.server;

    globalThis.fetch = async function mockpitFetch(
      input: RequestInfo | URL,
      init?: RequestInit
    ): Promise<Response> {
      // Normalize to a Request
      const request = new Request(input, init);
      const url = new URL(request.url);
      const path = url.pathname;
      const query = url.search ? url.search.slice(1) : undefined;
      const method = request.method;

      // Read body for non-safe methods
      let body: string | undefined;
      if (!["GET", "HEAD", "OPTIONS"].includes(method)) {
        try {
          body = await request.clone().text();
        } catch {
          // Body might not be readable
        }
      }

      // Build headers
      const headers: Record<string, string> = {};
      request.headers.forEach((value, key) => {
        headers[key] = value;
      });

      // Match directly against Rust mock engine -- no HTTP
      const match = await server.matchRequest(
        method,
        path,
        query,
        Object.keys(headers).length > 0 ? headers : undefined,
        body
      );

      if (match) {
        // Build Response from matched result
        const responseHeaders = new Headers(match.headers);
        return new Response(match.body, {
          status: match.status,
          headers: responseHeaders,
        });
      }

      // No match -- pass through to real fetch
      return originalFetch(input, init);
    };

    activeInterceptor = this;
    this.applied = true;
  }

  /** Stop intercepting and restore original fetch. */
  dispose(): void {
    if (!this.applied) return;
    globalThis.fetch = originalFetch;
    activeInterceptor = null;
    this.applied = false;
  }
}
