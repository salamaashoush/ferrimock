import type { JsHandler } from "@mockpit/node";

/**
 * Mockpit configuration.
 *
 * Universal config format used across all ecosystems:
 * - YAML/JSON: parsed by Rust (fast, no JS runtime needed)
 * - TS/JS: loaded via dynamic import (supports handler functions)
 *
 * @example
 * ```yaml
 * # mockpit.config.yaml
 * port: 3006
 * mocksDir: ./mocks/collections
 * cors: true
 * watch: true
 * ```
 *
 * @example
 * ```ts
 * // mockpit.config.ts
 * import { defineConfig, http, MockResponse } from 'mockpit'
 *
 * export default defineConfig({
 *   port: 3006,
 *   mocksDir: './mocks/collections',
 *   cors: true,
 *   handlers: [
 *     http.get('/api/users/:id', ({ params }) =>
 *       MockResponse.json({ id: params.id, name: 'John' })
 *     ),
 *   ],
 * })
 * ```
 */
export interface MockpitConfig {
  port?: number;
  host?: string;
  mocksDir?: string;
  mockFiles?: string[];
  cors?: boolean;
  watch?: boolean;
  verbose?: boolean;
  logMatches?: boolean;
  /** Handler functions (only available in TS/JS configs) */
  handlers?: JsHandler[];
}

/**
 * Define a mockpit configuration with full type safety.
 */
export function defineConfig(config: MockpitConfig): MockpitConfig {
  return config;
}

/**
 * Load a mockpit config file.
 *
 * - YAML/JSON: parsed by Rust (via `parseConfigFile`)
 * - TS/JS: loaded via dynamic import (supports handlers, npm imports)
 * - Auto-discovers `mockpit.config.*` if no path given
 */
export async function loadConfig(
  configPath?: string
): Promise<MockpitConfig | null> {
  const {
    parseConfigFile,
    discoverConfigFile,
  } = await import("@mockpit/node");
  const { resolve, extname } = await import("node:path");
  const { existsSync } = await import("node:fs");
  const { pathToFileURL } = await import("node:url");

  // Resolve path: explicit or auto-discover
  let resolvedPath: string | null = null;

  if (configPath) {
    resolvedPath = resolve(configPath);
    if (!existsSync(resolvedPath)) {
      throw new Error(`Config file not found: ${configPath}`);
    }
  } else {
    resolvedPath = discoverConfigFile() ?? null;
  }

  if (!resolvedPath) return null;

  const ext = extname(resolvedPath).toLowerCase();

  // YAML/JSON — parse in Rust (fast)
  if (ext === ".yaml" || ext === ".yml" || ext === ".json") {
    const parsed = parseConfigFile(resolvedPath);
    return parsed as MockpitConfig;
  }

  // TS/JS — dynamic import (supports handlers, npm imports)
  const fileUrl = pathToFileURL(resolvedPath).href;
  const mod = await import(fileUrl);
  return (mod.default ?? mod.config ?? mod) as MockpitConfig;
}
