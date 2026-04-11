import { defineCommand } from "clap-ts";
import { services } from "@mockpit/node";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

export const fakeImage = defineCommand({
  meta: { name: "image", description: "Generate fake images", aliases: ["img"] },
  args: {
    type: { type: "positional" as const, description: "Image type", default: "placeholder" },
    width: { type: "number" as const, short: "W", default: 200, description: "Width in pixels" },
    height: { type: "number" as const, short: "H", default: 200, description: "Height in pixels" },
    bgColor: { type: "string" as const, short: "b", description: "Background color (hex)" },
    textColor: { type: "string" as const, short: "t", description: "Text color (hex)" },
    text: { type: "string" as const, description: "Text to display" },
    initials: { type: "string" as const, short: "i", description: "Initials for avatar" },
    output: { type: "string" as const, short: "o", description: "Output file path" },
    base64: { type: "boolean" as const, description: "Output as base64 string" },
    dataUri: { type: "boolean" as const, description: "Output as data URI" },
  },
  run({ args }) {
    const result = services.fakeImage({
      imageType: args.type,
      width: args.width,
      height: args.height,
      bgColor: args.bgColor,
      textColor: args.textColor,
      text: args.text,
      initials: args.initials,
    });

    if (args.output) {
      const bytes = Buffer.from(result.base64, "base64");
      writeFileSync(resolve(args.output), bytes);
      console.log(`Image saved to ${args.output}`);
    } else if (args.dataUri) {
      console.log(`data:${result.mimeType};base64,${result.base64}`);
    } else if (args.base64) {
      console.log(result.base64);
    } else {
      // Write to stdout as binary
      process.stdout.write(Buffer.from(result.base64, "base64"));
    }
  },
});
