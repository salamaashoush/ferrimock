import { defineConfig } from "tsdown";

// @mockpit/node (native addon) is a dependency and is auto-externalized.
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
});
