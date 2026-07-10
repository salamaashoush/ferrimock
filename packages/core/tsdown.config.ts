import { defineConfig } from "tsdown";

// @ferrimock/node (native addon) is a dependency and is auto-externalized.
export default defineConfig({
  entry: ["src/index.ts", "src/node.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
});
