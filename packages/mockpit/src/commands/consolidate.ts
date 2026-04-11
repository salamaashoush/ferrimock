import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

export const consolidate = defineCommand({
  meta: { name: "consolidate", description: "Optimize mock collections by merging patterns", aliases: ["opt"] },
  args: {
    input: { type: "positional" as const, required: true, description: "Input mock collection", valueName: "INPUT" },
    output: { type: "positional" as const, required: true, description: "Output file", valueName: "OUTPUT" },
    format: { type: "enum" as const, short: "f", valueParser: ["json", "yaml"], default: "json", description: "Output format" },
    minPattern: { type: "number" as const, default: 3, description: "Min similar requests to form pattern" },
    noTemplates: { type: "boolean" as const, description: "Skip template extraction" },
  },
  async run({ args }) {
    const result = await services.consolidate({
      input: resolve(args.input),
      format: args.format,
      minPattern: args.minPattern,
      enableTemplates: !args.noTemplates,
    });

    writeFileSync(resolve(args.output), result.content);
    const savings = result.inputSize > 0
      ? ((1 - result.outputSize / result.inputSize) * 100).toFixed(1)
      : "0";
    console.log(`Consolidated: ${result.mocksBefore} -> ${result.mocksAfter} mocks (${savings}% reduction)`);
    console.log(`Written to ${args.output}`);
  },
});
