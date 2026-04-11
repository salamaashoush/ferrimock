import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

export const fakePdf = defineCommand({
  meta: { name: "pdf", description: "Generate fake PDF documents", aliases: ["doc"] },
  args: {
    pages: { type: "number" as const, short: "p", default: 1, description: "Number of pages" },
    text: { type: "string" as const, short: "t", description: "Custom text content" },
    output: { type: "string" as const, short: "o", description: "Output file path" },
    base64: { type: "boolean" as const, description: "Output as base64 string" },
    dataUri: { type: "boolean" as const, description: "Output as data URI" },
  },
  run({ args }) {
    const result = services.fakePdf({ pages: args.pages, text: args.text });

    if (args.output) {
      const bytes = Buffer.from(result.base64, "base64");
      writeFileSync(resolve(args.output), bytes);
      console.log(`PDF saved to ${args.output}`);
    } else if (args.dataUri) {
      console.log(`data:application/pdf;base64,${result.base64}`);
    } else if (args.base64) {
      console.log(result.base64);
    } else {
      process.stdout.write(Buffer.from(result.base64, "base64"));
    }
  },
});
