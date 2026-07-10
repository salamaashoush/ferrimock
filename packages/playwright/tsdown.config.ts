import { defineConfig } from "tsdown";

// ferrimock and @playwright/test are peer deps — auto-externalized.
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
});
