#!/usr/bin/env node
// Launcher for the native ferrimock binary: resolves the platform package
// installed via optionalDependencies and hands over argv untouched. The
// linux packages ship static musl binaries, so they run on glibc
// systems too and no libc probe is needed.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";

const require = createRequire(import.meta.url);

const PLATFORM_PACKAGES = {
  "darwin-arm64": ["@ferrimock/cli-darwin-arm64", "ferrimock"],
  "darwin-x64": ["@ferrimock/cli-darwin-x64", "ferrimock"],
  "linux-x64": ["@ferrimock/cli-linux-x64-musl", "ferrimock"],
  "linux-arm64": ["@ferrimock/cli-linux-arm64-musl", "ferrimock"],
  "win32-x64": ["@ferrimock/cli-win32-x64", "ferrimock.exe"],
};

function resolveBinary() {
  const key = `${process.platform}-${process.arch}`;
  const entry = PLATFORM_PACKAGES[key];
  if (!entry) {
    console.error(
      `ferrimock: unsupported platform ${key}. ` +
        `Prebuilt binaries exist for: ${Object.keys(PLATFORM_PACKAGES).join(", ")}. ` +
        "Install from source with: cargo install @ferrimock/cli --locked"
    );
    process.exit(1);
  }
  const [pkg, binary] = entry;
  try {
    const manifest = require.resolve(`${pkg}/package.json`);
    return join(dirname(manifest), binary);
  } catch {
    console.error(
      `ferrimock: platform package ${pkg} is not installed. ` +
        "It ships as an optionalDependency of @ferrimock/cli - reinstall without " +
        "--omit=optional / --no-optional, or install from source with: " +
        "cargo install @ferrimock/cli --locked"
    );
    process.exit(1);
  }
}

const result = spawnSync(resolveBinary(), process.argv.slice(2), {
  stdio: "inherit",
});
if (result.error) {
  console.error(`ferrimock: failed to launch native binary: ${result.error.message}`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status ?? 1);
