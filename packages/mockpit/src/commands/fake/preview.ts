import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const fakePreview = defineCommand({
  meta: { name: "preview", description: "Preview template rendering with fake data", aliases: ["tpl"] },
  args: {
    template: { type: "positional" as const, description: "Template string (Tera syntax)" },
    file: { type: "string" as const, short: "f", description: "Template file to render" },
    context: { type: "string" as const, short: "c", description: "Context data as JSON" },
    count: { type: "number" as const, short: "n", default: 1, description: "Number of renders" },
    format: { type: "enum" as const, short: "F", valueParser: ["text", "json"], default: "text", description: "Output format" },
  },
  run({ args }) {
    let templateContent = args.template;
    if (args.file) {
      templateContent = readFileSync(resolve(args.file), "utf-8");
    }
    if (!templateContent) {
      console.error("Provide a template string or --file");
      process.exit(1);
    }

    const context = args.context ? JSON.parse(args.context) : undefined;

    const results = services.renderTemplate({
      template: templateContent,
      context,
      count: args.count,
    });

    if (args.format === "json") {
      console.log(JSON.stringify(results.length === 1 ? results[0] : results, null, 2));
    } else {
      for (const r of results) console.log(r);
    }
  },
});
