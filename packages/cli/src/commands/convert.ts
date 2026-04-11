import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

export const convert = defineCommand({
  meta: { name: "convert", description: "Convert HAR file to mock definitions", aliases: ["conv"] },
  args: {
    input: { type: "positional" as const, required: true, description: "Input HAR file", valueName: "INPUT" },
    output: { type: "positional" as const, required: true, description: "Output mock file", valueName: "OUTPUT" },
    format: { type: "enum" as const, short: "f", valueParser: ["yaml", "json"], default: "yaml", description: "Output format" },
    domains: { type: "string" as const, short: "d", description: "Include only these domains (comma-separated)", valueDelimiter: "," },
    noPreflight: { type: "boolean" as const, description: "Exclude OPTIONS preflight requests" },
    noRedirects: { type: "boolean" as const, description: "Exclude redirect responses" },
    noStaticAssets: { type: "boolean" as const, description: "Exclude static assets" },
    keepSensitiveHeaders: { type: "boolean" as const, description: "Keep auth/cookie headers" },
    absoluteUrls: { type: "boolean" as const, description: "Keep absolute URLs" },
  },
  async run({ args }) {
    const result = await services.convert({
      input: resolve(args.input),
      format: args.format,
      allowedDomains: args.domains ? (Array.isArray(args.domains) ? args.domains : [args.domains]) : undefined,
      excludePreflight: args.noPreflight !== false,
      excludeRedirects: args.noRedirects !== false,
      excludeStaticAssets: args.noStaticAssets !== false,
      stripSensitiveHeaders: !args.keepSensitiveHeaders,
      normalizeUrls: !args.absoluteUrls,
    });

    writeFileSync(resolve(args.output), result.content);
    console.log(`Converted ${result.entriesProcessed} entries -> ${result.mocksCount} mock(s)`);
    console.log(`Written to ${args.output}`);
  },
});
