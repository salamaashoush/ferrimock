/**
 * Mockpit configuration.
 *
 * Pure configuration -- no mocks, no handlers. Those go in the mocks directory.
 *
 * @example
 * ```yaml
 * # mockpit.config.yaml
 * port: 3006
 * mocksDir: ./mocks
 * cors: true
 * watch: true
 * ```
 *
 * @example
 * ```ts
 * // mockpit.config.ts
 * import { defineConfig } from 'mockpit'
 *
 * export default defineConfig({
 *   port: 3006,
 *   mocksDir: './mocks',
 *   cors: true,
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
 * - YAML/JSON: parsed by Rust
 * - TS/JS: loaded via dynamic import
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

  if (ext === ".yaml" || ext === ".yml" || ext === ".json") {
    return parseConfigFile(resolvedPath) as MockpitConfig;
  }

  const fileUrl = pathToFileURL(resolvedPath).href;
  const mod = await import(fileUrl);
  return (mod.default ?? mod.config ?? mod) as MockpitConfig;
}
