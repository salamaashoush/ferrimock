/**
 * Playwright integration -- provides fixtures that automatically wire
 * mockpit into every test.
 *
 * Usage in playwright config or test file:
 *
 * ```ts
 * // fixtures.ts
 * import { test as base } from '@playwright/test'
 * import { mockpitFixtures } from 'mockpit/playwright'
 *
 * export const test = base.extend(mockpitFixtures({
 *   mocksDir: './mocks',
 * }))
 *
 * // Then in tests:
 * test('loads users', async ({ page, mocks }) => {
 *   // page.route() is already wired -- all matching requests are mocked
 *   await page.goto('http://localhost:3000')
 *
 *   // Add test-specific mocks on the fly
 *   mocks.useHandlers([
 *     http.get('/api/override', () => MockResponse.json({ override: true })),
 *   ])
 * })
 * ```
 *
 * Or at the context level (all pages share mocks):
 *
 * ```ts
 * export const test = base.extend(mockpitFixtures({
 *   mocksDir: './mocks',
 *   scope: 'context',  // wire into context instead of page
 * }))
 * ```
 */

import { MockpitInterceptor } from "./interceptor.js";

export type MockpitFixtureOptions = {
  /** Directory containing mock files (YAML/JSON/HAR/TS) */
  mocksDir?: string;
  /** Additional mock files to load */
  mockFiles?: string[];
  /** Where to intercept: 'page' (default) or 'context' */
  scope?: "page" | "context";
};

export type MockpitFixtures = {
  /** The mockpit interceptor instance for this test */
  mocks: MockpitInterceptor;
};

/**
 * Create Playwright fixtures that wire mockpit into every test.
 *
 * ```ts
 * import { test as base } from '@playwright/test'
 * import { mockpitFixtures } from 'mockpit/playwright'
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

        // Load mocks from directory
        if (mocksDir) {
          // Use the loader to handle YAML/JSON/HAR + TS/JS files
          const { loadMocksDir } = await import("./loader.js");
          const { MockpitServer } = require("@mockpit/node");
          // loadMocksDir needs a MockpitServer, but interceptor wraps one internally
          // Access the internal server via loadMocks for declarative, and jiti for TS
          await interceptor.loadMocks(mocksDir);
        }

        // Load additional mock files
        for (const file of mockFiles) {
          await interceptor.loadMockFile(file);
        }

        await use(interceptor);

        // Cleanup
        interceptor.dispose();
      },
      { scope: "test" as const },
    ],

    // Auto-wire into page or context
    page: [
      async (
        { page, mocks }: { page: any; mocks: MockpitInterceptor },
        use: any
      ) => {
        if (scope === "page") {
          await mocks.routePage(page);
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
          await mocks.routeContext(context);
        }
        await use(context);
      },
      { auto: true },
    ],
  } as any;
}
