/**
 * @mockpit/playwright -- Playwright adapter for mockpit.
 *
 * Provides:
 * - routePage(page, interceptor) -- wire into page.route()
 * - routeContext(context, interceptor) -- wire into context.route()
 * - mockpitFixtures() -- Playwright test fixtures for automatic setup
 *
 * Usage:
 * ```ts
 * import { test as base } from '@playwright/test'
 * import { mockpitFixtures } from '@mockpit/playwright'
 *
 * export const test = base.extend(mockpitFixtures({ mocksDir: './mocks' }))
 *
 * test('user page', async ({ page, mocks }) => {
 *   await page.goto('http://localhost:3000')
 * })
 * ```
 */

import { MockpitInterceptor } from "@mockpit/core";

// ===== Route helpers =====

/**
 * Wire mockpit into a Playwright Page via page.route().
 * All matching requests are mocked at the browser level.
 */
export async function routePage(
  page: any,
  interceptor: MockpitInterceptor
): Promise<void> {
  await page.route("**/*", async (route: any) => {
    const match = await matchPlaywrightRoute(route, interceptor);
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
}

/**
 * Wire mockpit into a Playwright BrowserContext via context.route().
 * All matching requests for all pages in the context are mocked.
 */
export async function routeContext(
  context: any,
  interceptor: MockpitInterceptor
): Promise<void> {
  await context.route("**/*", async (route: any) => {
    const match = await matchPlaywrightRoute(route, interceptor);
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
}

async function matchPlaywrightRoute(route: any, interceptor: MockpitInterceptor) {
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
  try {
    body = request.postData() ?? undefined;
  } catch {}

  return interceptor.matchRequest(method, path, query, headers, body);
}

// ===== Fixtures =====

export type MockpitFixtureOptions = {
  /** Directory containing mock files (YAML/JSON/HAR/TS) */
  mocksDir?: string;
  /** Additional mock files to load */
  mockFiles?: string[];
  /** Where to intercept: 'page' (default) or 'context' */
  scope?: "page" | "context";
};

export type MockpitFixtures = {
  mocks: MockpitInterceptor;
};

/**
 * Create Playwright fixtures that wire mockpit into every test.
 *
 * ```ts
 * import { test as base } from '@playwright/test'
 * import { mockpitFixtures } from '@mockpit/playwright'
 *
 * export const test = base.extend(mockpitFixtures({ mocksDir: './mocks' }))
 * ```
 */
export function mockpitFixtures(options: MockpitFixtureOptions = {}) {
  const { mocksDir, mockFiles = [], scope = "page" } = options;

  return {
    mocks: [
      async ({}, use: any) => {
        const interceptor = new MockpitInterceptor();

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
        { page, mocks }: { page: any; mocks: MockpitInterceptor },
        use: any
      ) => {
        if (scope === "page") {
          await routePage(page, mocks);
        }
        await use(page);
      },
      { auto: true },
    ],

    context: [
      async (
        { context, mocks }: { context: any; mocks: MockpitInterceptor },
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
