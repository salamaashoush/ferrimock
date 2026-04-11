import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";

export const fakeData = defineCommand({
  meta: { name: "data", description: "Generate fake data values", aliases: ["d"] },
  args: {
    type: { type: "positional" as const, required: true, description: "Generator type (email, name, uuid, etc.)", valueName: "TYPE" },
    count: { type: "number" as const, short: "n", default: 1, description: "Number of values to generate" },
    min: { type: "number" as const, description: "Minimum value (numeric generators)" },
    max: { type: "number" as const, description: "Maximum value (numeric generators)" },
    words: { type: "number" as const, short: "w", description: "Word count (text generators)" },
    length: { type: "number" as const, short: "l", description: "Length (alphanumeric/token)" },
    format: { type: "enum" as const, short: "f", valueParser: ["text", "json", "csv"], default: "text", description: "Output format" },
  },
  run({ args }) {
    const values = services.fakeData({
      generator: args.type,
      count: args.count,
      min: args.min,
      max: args.max,
      words: args.words,
      length: args.length,
    });

    switch (args.format) {
      case "json":
        console.log(JSON.stringify(values.length === 1 ? values[0] : values, null, 2));
        break;
      case "csv":
        console.log(values.join(","));
        break;
      default:
        for (const v of values) console.log(v);
    }
  },
});
