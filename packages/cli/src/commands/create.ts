import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";

export const create = defineCommand({
  meta: { name: "create", description: "Create a new mock definition", aliases: ["new"] },
  args: {
    url: { type: "positional" as const, required: true, description: "URL pattern to match" },
    method: { type: "string" as const, short: "m", default: "GET", description: "HTTP method" },
    status: { type: "number" as const, short: "s", default: 200, description: "Response status code" },
    body: { type: "string" as const, short: "b", description: "Response body" },
    template: { type: "boolean" as const, short: "t", description: "Generate template with fake data" },
    id: { type: "string" as const, short: "i", description: "Custom mock ID" },
    priority: { type: "number" as const, short: "p", default: 100, description: "Mock priority" },
    collection: { type: "string" as const, short: "c", description: "Collection name" },
    output: { type: "string" as const, short: "o", description: "Output file path" },
    format: { type: "enum" as const, short: "f", valueParser: ["yaml", "json"], default: "yaml", description: "Output format" },
  },
  run({ args }) {
    const result = services.create({
      url: args.url,
      method: args.method,
      status: args.status,
      body: args.body,
      template: args.template,
      id: args.id,
      priority: args.priority,
      collection: args.collection,
      format: args.format,
    });

    if (args.output) {
      const outPath = resolve(args.output);
      mkdirSync(dirname(outPath), { recursive: true });
      writeFileSync(outPath, result.content);
      console.log(`Created mock "${result.mockId}" at ${outPath}`);
    } else {
      console.log(result.content);
    }
  },
});
