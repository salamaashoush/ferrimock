/**
 * Mock directory loader -- loads all mock formats from a directory.
 *
 * Supports:
 * - .yaml, .yml, .json, .har -- loaded by Rust (via MockpitServer.loadMocks)
 * - .ts, .js, .mts, .mjs -- imported natively (jiti only for TS on plain Node)
 *
 * Handler files import from 'mockpit' and either export a default array
 * or register by calling the factories at module scope -- the same two
 * shapes the embedded QuickJS runtime accepts, so files are portable:
 *
 * ```ts
 * // mocks/users.ts
 * import { http, HttpResponse } from 'mockpit'
 *
 * export default [
 *   http.get('/api/users/:id', ({ params }) =>
 *     HttpResponse.json({ id: params.id, name: 'John' })
 *   ),
 * ]
 * ```
 */

import type { MockpitServer, RequestHandler } from "@mockpit/node";
import { createJiti } from "jiti";
import { readdirSync, statSync } from "node:fs";
import { resolve, extname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { collectHandlers } from "./registration.js";
import { isWsHandler, type WebSocketHandler } from "./ws.js";
import type { MockpitInterceptor } from "./interceptor.js";

const RUST_EXTENSIONS = new Set([".yaml", ".yml", ".json", ".har"]);
const JS_EXTENSIONS = new Set([".ts", ".js", ".mts", ".mjs"]);

// Single jiti instance for loading TS mock files on plain Node
const jiti = createJiti(import.meta.url, {
  // Use native ESM when possible, fallback to transpilation for TS
  interopDefault: true,
});

/**
 * Prefer the runtime's own `import()` so mock files share this process's
 * module graph -- jiti evaluates in a separate graph, which duplicates
 * stateful singletons (fetch-patching interceptors, the registration
 * collector) and desyncs them. jiti is only needed where the runtime
 * can't execute the file natively: TypeScript on plain Node.
 */
async function importMockFile(file: string): Promise<unknown> {
  const ext = extname(file).toLowerCase();
  const nativeTs = !!process.versions.bun;
  if (ext === ".js" || ext === ".mjs" || nativeTs) {
    return import(pathToFileURL(file).href);
  }
  return jiti.import(file);
}

/**
 * Load all mocks from a directory into a MockpitServer or a
 * MockpitInterceptor.
 *
 * Declarative files (.yaml/.json/.har) are loaded by Rust.
 * Handler files (.ts/.js) run on this runtime's own module graph.
 * WebSocket handlers (`ws.link`) are real engine mocks and register on
 * both lanes; a TCP MockpitServer serves them natively.
 */
export async function loadMocksDir(
  server: MockpitServer | MockpitInterceptor,
  dir: string
): Promise<{
  declarativeCount: number;
  handlerCount: number;
  wsHandlerCount: number;
}> {
  const resolvedDir = resolve(dir);
  const interceptor =
    "resolveRequest" in server ? (server as MockpitInterceptor) : null;

  let declarativeCount = 0;
  let handlerCount = 0;
  let wsHandlerCount = 0;

  // Check directory exists
  try {
    const stat = statSync(resolvedDir);
    if (!stat.isDirectory())
      return { declarativeCount, handlerCount, wsHandlerCount };
  } catch {
    return { declarativeCount, handlerCount, wsHandlerCount };
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
    .filter((f) => !f.startsWith("_") && !f.startsWith("."))
    .map((f) => join(resolvedDir, f))
    .sort();

  // 3. Load each handler file. Two registration styles work:
  //    - `export default [http.get(...), ...]`
  //    - bare `http.get(...)` calls at module scope, caught by the
  //      collection window (the QuickJS/CLI convention)
  for (const file of handlerFiles) {
    try {
      const {
        result: mod,
        handlers: collected,
        wsHandlers: collectedWs,
      } = await collectHandlers(() => importMockFile(file));
      const exported = (mod as any).default ?? (mod as any).handlers ?? mod;
      const exportedAll: unknown[] = Array.isArray(exported) ? exported : [];
      const exportedHandlers = exportedAll.filter(
        (h): h is RequestHandler => !isWsHandler(h)
      );
      const exportedWs = exportedAll.filter(isWsHandler);

      // Union: exported first (explicit wins the ordering), then any
      // side-effect registrations not already exported.
      const seen = new Set(exportedHandlers);
      const handlers = [
        ...exportedHandlers,
        ...collected.filter((h) => !seen.has(h)),
      ];
      const seenWs = new Set(exportedWs);
      const wsHandlers = [
        ...exportedWs,
        ...(collectedWs as WebSocketHandler[]).filter((h) => !seenWs.has(h)),
      ];

      if (handlers.length > 0) {
        server.useHandlers(handlers);
        handlerCount += handlers.length;
      }
      if (wsHandlers.length > 0) {
        if (interceptor) {
          interceptor.useHandlers(wsHandlers);
        } else {
          (server as MockpitServer).useHandlers(
            wsHandlers.map((handler) => handler.native)
          );
        }
        wsHandlerCount += wsHandlers.length;
      }
    } catch (err) {
      console.error(`Failed to load handler file ${file}: ${err}`);
    }
  }

  return { declarativeCount, handlerCount, wsHandlerCount };
}
