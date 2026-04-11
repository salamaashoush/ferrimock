import { defineCommand, runMain } from "clap-ts";
import { serve } from "./commands/serve.js";
import { create } from "./commands/create.js";
import { list } from "./commands/list.js";
import { show } from "./commands/show.js";
import { testMatch } from "./commands/test.js";
import { validate } from "./commands/validate.js";
import { format } from "./commands/format.js";
import { convert } from "./commands/convert.js";
import { consolidate } from "./commands/consolidate.js";
import { exportHar } from "./commands/export.js";
import { fakeData } from "./commands/fake/data.js";
import { fakeImage } from "./commands/fake/image.js";
import { fakePdf } from "./commands/fake/pdf.js";
import { fakePreview } from "./commands/fake/preview.js";
import { fakeList } from "./commands/fake/list.js";

const mock = defineCommand({
  meta: {
    name: "mock",
    description: "Mock management commands",
    aliases: ["m"],
  },
  subCommands: {
    serve,
    create,
    list,
    show,
    test: testMatch,
    validate,
    format,
    convert,
    consolidate,
    export: exportHar,
  },
});

const fake = defineCommand({
  meta: {
    name: "fake",
    description: "Fake data generation",
    aliases: ["f"],
  },
  subCommands: {
    data: fakeData,
    image: fakeImage,
    pdf: fakePdf,
    preview: fakePreview,
    list: fakeList,
  },
});

const root = defineCommand({
  meta: {
    name: "mockpit",
    version: "0.1.0",
    description: "High-performance HTTP mocking engine",
    about:
      "A Rust-powered mock server with MSW-style handler API, template rendering, HAR conversion, and fake data generation.",
  },
  args: {
    verbose: {
      type: "boolean" as const,
      short: "v",
      description: "Enable verbose output",
      global: true,
    },
    quiet: {
      type: "boolean" as const,
      short: "q",
      description: "Suppress all output except errors",
      global: true,
      conflictsWith: ["verbose"],
    },
  },
  subCommands: { mock, fake },
});

runMain(root);
