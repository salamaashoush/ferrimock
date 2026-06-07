import { defineConfig } from "tsdown";

// @mockpit/core, @mockpit/node, clap-ts are deps — auto-externalized.
export default defineConfig({
  entry: ["src/cli.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
});
