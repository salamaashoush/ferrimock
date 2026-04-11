import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

export const exportHar = defineCommand({
  meta: { name: "export", description: "Export mocks to HAR format", aliases: ["exp"] },
  args: {
    output: { type: "positional" as const, required: true, description: "Output HAR file path", valueName: "OUTPUT" },
    mocksDir: { type: "string" as const, short: "d", description: "Mock collections directory", env: "MOCKS_DIR" },
    filter: { type: "string" as const, short: "c", description: "Filter mocks by ID" },
  },
  async run({ args }) {
    const result = await services.export({ mocksDir: args.mocksDir, filter: args.filter });
    writeFileSync(resolve(args.output), result.content);
    console.log(`Exported ${result.mocksExported} mock(s) to ${args.output}`);
  },
});
