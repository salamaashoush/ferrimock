#!/usr/bin/env node
// Launcher for the native mockpit binary: resolves the platform package
// installed via optionalDependencies and hands over argv untouched. The
// linux packages ship static musl binaries, so they run on glibc
// systems too and no libc probe is needed.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

const require = createRequire(import.meta.url);

const PLATFORM_PACKAGES = {
  "darwin-arm64": ["@mockpit/cli-darwin-arm64", "mockpit"],
  "darwin-x64": ["@mockpit/cli-darwin-x64", "mockpit"],
  "linux-x64": ["@mockpit/cli-linux-x64-musl", "mockpit"],
  "linux-arm64": ["@mockpit/cli-linux-arm64-musl", "mockpit"],
  "win32-x64": ["@mockpit/cli-win32-x64", "mockpit.exe"],
};

function resolveBinary() {
  const key = `${process.platform}-${process.arch}`;
  const entry = PLATFORM_PACKAGES[key];
  if (!entry) {
    console.error(
      `mockpit: unsupported platform ${key}. ` +
        `Prebuilt binaries exist for: ${Object.keys(PLATFORM_PACKAGES).join(", ")}. ` +
        "Install from source with: cargo install mockpit-cli --locked"
    );
    process.exit(1);
  }
  const [pkg, binary] = entry;
  try {
    const manifest = require.resolve(`${pkg}/package.json`);
    return join(dirname(manifest), binary);
  } catch {
    console.error(
      `mockpit: platform package ${pkg} is not installed. ` +
        "It ships as an optionalDependency of @mockpit/cli - reinstall without " +
        "--omit=optional / --no-optional, or install from source with: " +
        "cargo install mockpit-cli --locked"
    );
    process.exit(1);
  }
}

const result = spawnSync(resolveBinary(), process.argv.slice(2), {
  stdio: "inherit",
});
if (result.error) {
  console.error(`mockpit: failed to launch native binary: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
