import { defineConfig } from "tsdown";

// @mockpit/core and @playwright/test are peer deps — auto-externalized.
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
});
