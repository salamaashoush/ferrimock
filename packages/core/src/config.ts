/**
 * Ferrimock configuration.
 *
 * Pure configuration -- no mocks, no handlers. Those go in the mocks directory.
 *
 * @example
 * ```yaml
 * # ferrimock.config.yaml
 * port: 3006
 * mocksDir: ./mocks
 * cors: true
 * watch: true
 * ```
 *
 * @example
 * ```ts
 * // ferrimock.config.ts
 * import { defineConfig } from 'ferrimock'
 *
 * export default defineConfig({
 *   port: 3006,
 *   mocksDir: './mocks',
 *   cors: true,
 * })
 * ```
 */
export interface FerrimockConfig {
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
 * Define a ferrimock configuration with full type safety.
 */
export function defineConfig(config: FerrimockConfig): FerrimockConfig {
  return config;
}

/**
 * Load a ferrimock config file.
 *
 * - YAML/JSON: parsed by Rust
 * - TS/JS: loaded via dynamic import
 * - Auto-discovers `ferrimock.config.*` if no path given
 */
export async function loadConfig(
  configPath?: string
): Promise<FerrimockConfig | null> {
  const {
    parseConfigFile,
    discoverConfigFile,
  } = await import("@ferrimock/node");
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
    return parseConfigFile(resolvedPath) as FerrimockConfig;
  }

  // TS/JS config -- load via jiti (works on plain Node.js, no --import tsx)
  const { createJiti } = await import("jiti");
  const jiti = createJiti(import.meta.url, { interopDefault: true });
  const mod = await jiti.import(resolvedPath) as any;
  return (mod.default ?? mod.config ?? mod) as FerrimockConfig;
}
