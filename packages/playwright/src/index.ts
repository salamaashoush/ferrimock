/**
 * @ferrimock/playwright -- Playwright adapter for ferrimock.
 *
 * Provides:
 * - routePage(page, interceptor) -- wire into page.route()
 * - routeContext(context, interceptor) -- wire into context.route()
 * - ferrimockFixtures() -- Playwright test fixtures for automatic setup
 *
 * Usage:
 * ```ts
 * import { test as base } from '@playwright/test'
 * import { ferrimockFixtures } from '@ferrimock/playwright'
 *
 * export const test = base.extend(ferrimockFixtures({ mocksDir: './mocks' }))
 *
 * test('user page', async ({ page, mocks }) => {
 *   await page.goto('http://localhost:3000')
 * })
 * ```
 */

import { FerrimockInterceptor } from "ferrimock";

let playwrightWsCounter = 0;

// ===== Route helpers =====

/**
 * Wire ferrimock into a Playwright Page via page.route().
 * All matching requests are mocked at the browser level.
 */
export async function routePage(
  page: any,
  interceptor: FerrimockInterceptor
): Promise<void> {
  await page.route("**/*", async (route: any) => {
    const match = await matchPlaywrightRoute(route, interceptor);
    if (match) {
      await route.fulfill({
        status: match.status,
        headers: match.headers,
        body: Buffer.from(match.body),
      });
    } else {
      await route.continue();
    }
  });
}

/**
 * Wire ferrimock into a Playwright BrowserContext via context.route().
 * All matching requests for all pages in the context are mocked.
 */
export async function routeContext(
  context: any,
  interceptor: FerrimockInterceptor
): Promise<void> {
  await context.route("**/*", async (route: any) => {
    const match = await matchPlaywrightRoute(route, interceptor);
    if (match) {
      await route.fulfill({
        status: match.status,
        headers: match.headers,
        body: Buffer.from(match.body),
      });
    } else {
      await route.continue();
    }
  });
}

/**
 * Wire ferrimock's `ws.link` handlers into Playwright via
 * page.routeWebSocket(). Matched connections get MSW-shaped
 * `{ client, server, params }` objects built over Playwright's
 * WebSocketRoute; unmatched connections pass through to the real
 * server.
 *
 * Forwarding mirrors the Node lane: after `server.connect()`, client
 * frames flow to the real server and server frames to the page unless a
 * `message` listener calls `event.preventDefault()`.
 */
export async function routeWebSocketPage(
  page: any,
  interceptor: FerrimockInterceptor
): Promise<void> {
  await page.routeWebSocket("**/*", (wsRoute: any) => {
    const url = new URL(wsRoute.url());
    const handlers = interceptor.wsRegistry
      .all()
      .filter((handler) => handler.parse(url).matches);

    if (handlers.length === 0) {
      wsRoute.connectToServer();
      return;
    }

    type Listener = (event: any) => void;
    const clientListeners = new Map<string, Listener[]>();
    const serverListeners = new Map<string, Listener[]>();
    let serverRoute: any = null;

    const makeEvent = (type: string, data?: unknown) => {
      const event: any = {
        type,
        data,
        defaultPrevented: false,
        preventDefault() {
          event.defaultPrevented = true;
        },
        stopPropagation() {},
        stopImmediatePropagation() {},
      };
      return event;
    };

    const dispatch = (
      listeners: Map<string, Listener[]>,
      type: string,
      data?: unknown
    ) => {
      const event = makeEvent(type, data);
      for (const listener of listeners.get(type) ?? []) {
        listener(event);
      }
      return event.defaultPrevented;
    };

    wsRoute.onMessage((message: unknown) => {
      const prevented = dispatch(clientListeners, "message", message);
      if (!prevented && serverRoute) {
        serverRoute.send(message);
      }
    });
    wsRoute.onClose((code?: number, reason?: string) => {
      const event = makeEvent("close");
      event.code = code;
      event.reason = reason;
      for (const listener of clientListeners.get("close") ?? []) {
        listener(event);
      }
    });

    const client = {
      id: `pw-ws:${(playwrightWsCounter++).toString(16)}`,
      url,
      send(data: unknown) {
        wsRoute.send(data);
      },
      close(code?: number, reason?: string) {
        wsRoute.close({ code, reason });
      },
      addEventListener(type: string, listener: Listener) {
        const list = clientListeners.get(type) ?? [];
        list.push(listener);
        clientListeners.set(type, list);
      },
      removeEventListener(type: string, listener: Listener) {
        const list = clientListeners.get(type) ?? [];
        clientListeners.set(
          type,
          list.filter((entry) => entry !== listener)
        );
      },
    };

    const server = {
      connect() {
        if (serverRoute) return;
        serverRoute = wsRoute.connectToServer();
        serverRoute.onMessage((message: unknown) => {
          const prevented = dispatch(serverListeners, "message", message);
          if (!prevented) {
            wsRoute.send(message);
          }
        });
        serverRoute.onClose((code?: number, reason?: string) => {
          const prevented = dispatch(serverListeners, "close");
          if (!prevented) {
            wsRoute.close({ code, reason });
          }
        });
        dispatch(serverListeners, "open");
      },
      send(data: unknown) {
        serverRoute?.send(data);
      },
      close() {
        serverRoute?.close();
      },
      addEventListener(type: string, listener: Listener) {
        const list = serverListeners.get(type) ?? [];
        list.push(listener);
        serverListeners.set(type, list);
      },
      removeEventListener(type: string, listener: Listener) {
        const list = serverListeners.get(type) ?? [];
        serverListeners.set(
          type,
          list.filter((entry) => entry !== listener)
        );
      },
    };

    for (const handler of handlers) {
      handler.run({ client, server, info: { protocols: undefined } } as any);
    }
  });
}

async function matchPlaywrightRoute(route: any, interceptor: FerrimockInterceptor) {
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

  let body: Uint8Array | undefined;
  try {
    body = request.postDataBuffer() ?? undefined;
  } catch {}

  return interceptor.matchRequest(method, path, query, headers, body);
}

// ===== Fixtures =====

export type FerrimockFixtureOptions = {
  /** Directory containing mock files (YAML/JSON/HAR/TS) */
  mocksDir?: string;
  /** Additional mock files to load */
  mockFiles?: string[];
  /** Where to intercept: 'page' (default) or 'context' */
  scope?: "page" | "context";
};

export type FerrimockFixtures = {
  mocks: FerrimockInterceptor;
};

/**
 * Create Playwright fixtures that wire ferrimock into every test.
 *
 * ```ts
 * import { test as base } from '@playwright/test'
 * import { ferrimockFixtures } from '@ferrimock/playwright'
 *
 * export const test = base.extend(ferrimockFixtures({ mocksDir: './mocks' }))
 * ```
 */
export function ferrimockFixtures(options: FerrimockFixtureOptions = {}) {
  const { mocksDir, mockFiles = [], scope = "page" } = options;

  return {
    mocks: [
      async ({}, use: any) => {
        const interceptor = new FerrimockInterceptor();

        if (mocksDir) {
          await interceptor.loadMocks(mocksDir);
        }

        for (const file of mockFiles) {
          await interceptor.loadMockFile(file);
        }

        await use(interceptor);
      },
      { scope: "test" as const },
    ],

    page: [
      async (
        { page, mocks }: { page: any; mocks: FerrimockInterceptor },
        use: any
      ) => {
        if (scope === "page") {
          await routePage(page, mocks);
          await routeWebSocketPage(page, mocks);
        }
        await use(page);
      },
      { auto: true },
    ],

    context: [
      async (
        { context, mocks }: { context: any; mocks: FerrimockInterceptor },
        use: any
      ) => {
        if (scope === "context") {
          await routeContext(context, mocks);
        }
        await use(context);
      },
      { auto: true },
    ],
  } as any;
}
