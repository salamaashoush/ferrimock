import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const validate = defineCommand({
  meta: { name: "validate", description: "Validate mock configuration files", aliases: ["v"] },
  args: {
    path: { type: "positional" as const, description: "Path to file or directory", default: "mocks/collections" },
    format: { type: "enum" as const, short: "f", valueParser: ["text", "json"], default: "text", description: "Output format" },
  },
  async run({ args }) {
    const result = await services.validate({ path: args.path });

    if (args.format === "json") {
      console.log(JSON.stringify(result, null, 2));
    } else {
      for (const err of result.errors) {
        console.error(`  error: ${err.message}${err.suggestion ? ` (${err.suggestion})` : ""}`);
      }
      for (const warn of result.warnings) {
        console.warn(`  warn: ${warn.message}`);
      }
      console.log(`\nChecked files: ${result.totalErrors + result.totalWarnings === 0 ? "all valid" : `${result.totalErrors} error(s), ${result.totalWarnings} warning(s)`}`);
    }

    if (!result.isValid) process.exit(1);
  },
});
