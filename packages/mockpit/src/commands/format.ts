import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const format = defineCommand({
  meta: { name: "format", description: "Format mock configuration files", aliases: ["fmt"] },
  args: {
    path: { type: "positional" as const, description: "Path to file or directory", default: "mocks/collections" },
    check: { type: "boolean" as const, description: "Check formatting without modifying" },
  },
  run({ args }) {
    const result = services.format({ path: args.path, check: args.check });

    for (const f of result.files) {
      if (f.error) {
        console.error(`  error: ${f.path}: ${f.error}`);
      } else if (f.changed) {
        console.log(`  ${args.check ? "would format" : "formatted"}: ${f.path}`);
      }
    }

    console.log(`\n${result.formattedCount} formatted, ${result.unchangedCount} unchanged, ${result.errorCount} errors`);

    if (args.check && result.formattedCount > 0) process.exit(1);
  },
});
