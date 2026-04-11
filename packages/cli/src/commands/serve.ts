import { defineCommand } from "clap-ts";
import { MockpitServer, loadConfig, loadMocksDir } from "@mockpit/core";
import { resolve } from "node:path";

export const serve = defineCommand({
  meta: {
    name: "serve",
    description: "Start the mock server",
    aliases: ["sv"],
  },
  args: {
    port: {
      type: "number" as const,
      short: "p",
      description: "Port to listen on (default: 3006)",
      env: "MOCKPIT_PORT",
    },
    host: {
      type: "string" as const,
      description: "Host to bind to (default: 127.0.0.1)",
    },
    mocks: {
      type: "string" as const,
      short: "m",
      description: "Mocks directory (default: mocks/)",
      env: "MOCKS_DIR",
    },
    mockFile: {
      type: "string" as const,
      short: "f",
      action: "append" as const,
      description: "Additional mock file to load (repeatable)",
    },
    config: {
      type: "string" as const,
      short: "c",
      description:
        "Config file path (default: auto-discover mockpit.config.*)",
    },
    watch: {
      type: "boolean" as const,
      short: "w",
      description: "Watch mock files and hot-reload on change",
    },
    cors: {
      type: "boolean" as const,
      description: "Enable CORS headers for browser access",
    },
    open: {
      type: "boolean" as const,
      short: "o",
      description: "Open browser to server URL",
    },
  },
  async run({ args }) {
    // Load config file (explicit path or auto-discover)
    const config = await loadConfig(args.config);

    // Merge CLI args with config (CLI takes precedence)
    const port = args.port ?? config?.port ?? 3006;
    const host = args.host ?? config?.host ?? "127.0.0.1";
    const mocksDir = args.mocks ?? config?.mocksDir ?? "mocks";
    const cors = args.cors ?? config?.cors ?? false;
    const watch = args.watch ?? config?.watch ?? false;

    const server = new MockpitServer();

    // 1. Load all mocks from the mocks directory
    //    YAML/JSON/HAR -> loaded by Rust
    //    TS/JS -> loaded by Node/Bun via dynamic import
    const { declarativeCount, handlerCount } = await loadMocksDir(
      server,
      resolve(mocksDir)
    );
    if (declarativeCount > 0)
      console.log(`Loaded ${declarativeCount} declarative mock(s) from ${mocksDir}`);
    if (handlerCount > 0)
      console.log(`Loaded ${handlerCount} handler(s) from ${mocksDir}`);

    // 2. Load additional mock files from CLI and config
    const mockFiles = [
      ...(config?.mockFiles?.map((f) => resolve(f)) ?? []),
      ...(
        Array.isArray(args.mockFile)
          ? args.mockFile
          : args.mockFile
            ? [args.mockFile]
            : []
      ).map((f) => resolve(f)),
    ];
    for (const file of mockFiles) {
      const count = await server.loadMockFile(file);
      console.log(`Loaded ${count} mock(s) from ${file}`);
    }

    // Start server
    const url = await server.listen(port);
    console.log();
    console.log(`Mock server running at ${url}`);
    console.log(`  Mocks loaded: ${server.mockCount}`);
    if (cors) console.log("  CORS: enabled");
    if (watch) console.log(`  Watching: ${mocksDir}`);
    console.log();
    console.log("Press Ctrl+C to stop");

    if (args.open) {
      const { exec } = await import("node:child_process");
      exec(
        process.platform === "darwin"
          ? `open ${url}`
          : process.platform === "win32"
            ? `start ${url}`
            : `xdg-open ${url}`
      );
    }

    // Keep process alive until signal
    await new Promise<void>((res) => {
      const shutdown = async () => {
        console.log("\nShutting down...");
        await server.close();
        res();
      };
      process.on("SIGINT", shutdown);
      process.on("SIGTERM", shutdown);
    });
  },
});
