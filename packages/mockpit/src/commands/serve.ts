import { defineCommand } from "clap-ts";
import { MockpitServer } from "@mockpit/node";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

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
      default: 3006,
      description: "Port to listen on",
      env: "MOCKPIT_PORT",
    },
    host: {
      type: "string" as const,
      default: "127.0.0.1",
      description: "Host to bind to",
    },
    mocks: {
      type: "string" as const,
      short: "m",
      description: "Mock collections directory",
      env: "MOCKS_DIR",
    },
    mockFile: {
      type: "string" as const,
      short: "f",
      description: "Load specific mock file",
    },
    config: {
      type: "string" as const,
      short: "c",
      description: "TS/JS config file with handler functions",
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
    const server = new MockpitServer();

    // Load declarative mocks from directory
    if (args.mocks) {
      const count = await server.loadMocks(args.mocks);
      console.log(`Loaded ${count} mock(s) from ${args.mocks}`);
    } else if (!args.mockFile && !args.config) {
      // Default directory
      const defaultDir = process.env.MOCKS_DIR || "mocks/collections";
      try {
        const count = await server.loadMocks(defaultDir);
        if (count > 0) console.log(`Loaded ${count} mock(s) from ${defaultDir}`);
      } catch {
        // Directory might not exist, that's fine
      }
    }

    // Load specific mock file
    if (args.mockFile) {
      const count = await server.loadMockFile(resolve(args.mockFile));
      console.log(`Loaded ${count} mock(s) from ${args.mockFile}`);
    }

    // Load TS/JS config file with handlers
    if (args.config) {
      const configPath = resolve(args.config);
      const configUrl = pathToFileURL(configPath).href;
      const config = await import(configUrl);
      const handlers = config.default ?? config.handlers;
      if (Array.isArray(handlers)) {
        server.useHandlers(handlers);
        console.log(`Loaded ${handlers.length} handler(s) from ${args.config}`);
      } else {
        console.error(
          `Config file must export default or handlers as an array`
        );
        process.exit(1);
      }
    }

    const url = await server.listen(args.port);
    console.log();
    console.log(`Mock server running at ${url}`);
    console.log(`  Mocks loaded: ${server.mockCount}`);
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

    // Keep process alive until Ctrl+C
    await new Promise<void>((resolve) => {
      process.on("SIGINT", async () => {
        console.log("\nShutting down...");
        await server.close();
        resolve();
      });
      process.on("SIGTERM", async () => {
        await server.close();
        resolve();
      });
    });
  },
});
