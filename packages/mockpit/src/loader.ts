/**
 * Mock directory loader -- loads all mock formats from a directory.
 *
 * Supports:
 * - .yaml, .yml, .json, .har -- loaded by Rust (via MockpitServer.loadMocks)
 * - .ts, .js, .mts, .mjs -- loaded by Node/Bun (via dynamic import)
 *
 * TS/JS files should export a default array of handlers:
 *
 * ```ts
 * // mocks/api-users.ts
 * import { http, MockResponse } from 'mockpit'
 *
 * export default [
 *   http.get('/api/users/:id', ({ params }) =>
 *     MockResponse.json({ id: params.id, name: 'John' })
 *   ),
 *   http.post('/api/users', ({ bodyJson }) =>
 *     MockResponse.json({ id: '1', ...bodyJson }, { status: 201 })
 *   ),
 * ]
 * ```
 */

import type { MockpitServer, JsHandler } from "@mockpit/node";
import { readdirSync, statSync } from "node:fs";
import { resolve, extname, join } from "node:path";
import { pathToFileURL } from "node:url";

const RUST_EXTENSIONS = new Set([".yaml", ".yml", ".json", ".har"]);
const JS_EXTENSIONS = new Set([".ts", ".js", ".mts", ".mjs"]);

/**
 * Load all mocks from a directory into a MockpitServer.
 *
 * Declarative files (.yaml/.json/.har) are loaded by Rust.
 * Handler files (.ts/.js) are loaded by Node/Bun via dynamic import.
 */
export async function loadMocksDir(
  server: MockpitServer,
  dir: string
): Promise<{ declarativeCount: number; handlerCount: number }> {
  const resolvedDir = resolve(dir);

  let declarativeCount = 0;
  let handlerCount = 0;

  // Check directory exists
  try {
    const stat = statSync(resolvedDir);
    if (!stat.isDirectory()) return { declarativeCount, handlerCount };
  } catch {
    return { declarativeCount, handlerCount };
  }

  // 1. Load declarative mocks via Rust (fast, parallel)
  try {
    declarativeCount = await server.loadMocks(resolvedDir);
  } catch {
    // Directory might have no yaml/json/har files
  }

  // 2. Scan for TS/JS handler files
  const entries = readdirSync(resolvedDir);
  const handlerFiles = entries
    .filter((f) => JS_EXTENSIONS.has(extname(f).toLowerCase()))
    .filter((f) => !f.startsWith("_") && !f.startsWith(".")) // skip _helpers.ts, .hidden.ts
    .map((f) => join(resolvedDir, f))
    .sort();

  // 3. Load each handler file via dynamic import
  for (const file of handlerFiles) {
    try {
      const fileUrl = pathToFileURL(file).href;
      const mod = await import(fileUrl);
      const handlers: JsHandler[] = mod.default ?? mod.handlers ?? [];

      if (Array.isArray(handlers) && handlers.length > 0) {
        server.useHandlers(handlers);
        handlerCount += handlers.length;
      }
    } catch (err) {
      console.error(`Failed to load handler file ${file}: ${err}`);
    }
  }

  return { declarativeCount, handlerCount };
}
