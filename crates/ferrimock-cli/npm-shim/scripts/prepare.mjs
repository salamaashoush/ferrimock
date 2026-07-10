#!/usr/bin/env node
// Assembles the publishable npm layout for the CLI from CI build
// artifacts: one platform package per Rust target under npm/, and the
// shim's own version + optionalDependencies pinned to the release
// version. Run from anywhere:
//
//   node scripts/prepare.mjs <version> <artifacts-dir>
//
// <artifacts-dir> holds one subdirectory per target named
// cli-npm-<rust-triple> containing the raw binary (ferrimock or
// ferrimock.exe), as uploaded by the release workflow's build-cli job.

import { chmodSync, cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const TARGETS = {
  "aarch64-apple-darwin": {
    dir: "darwin-arm64",
    name: "ferrimock-cli-darwin-arm64",
    os: "darwin",
    cpu: "arm64",
    binary: "ferrimock",
  },
  "x86_64-apple-darwin": {
    dir: "darwin-x64",
    name: "ferrimock-cli-darwin-x64",
    os: "darwin",
    cpu: "x64",
    binary: "ferrimock",
  },
  // Static musl builds: no "libc" field so glibc systems install them too.
  "x86_64-unknown-linux-musl": {
    dir: "linux-x64-musl",
    name: "ferrimock-cli-linux-x64-musl",
    os: "linux",
    cpu: "x64",
    binary: "ferrimock",
  },
  "aarch64-unknown-linux-musl": {
    dir: "linux-arm64-musl",
    name: "ferrimock-cli-linux-arm64-musl",
    os: "linux",
    cpu: "arm64",
    binary: "ferrimock",
  },
  "x86_64-pc-windows-msvc": {
    dir: "win32-x64",
    name: "ferrimock-cli-win32-x64",
    os: "win32",
    cpu: "x64",
    binary: "ferrimock.exe",
  },
};

const [version, artifactsDir] = process.argv.slice(2);
if (!version || !artifactsDir) {
  console.error("usage: prepare.mjs <version> <artifacts-dir>");
  process.exit(1);
}

const shimRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const npmRoot = join(shimRoot, "npm");
const generated = [];

for (const [triple, target] of Object.entries(TARGETS)) {
  const source = join(resolve(artifactsDir), `cli-npm-${triple}`, target.binary);
  if (!existsSync(source)) {
    console.error(`skipping ${target.name}: no artifact at ${source}`);
    continue;
  }
  const pkgDir = join(npmRoot, target.dir);
  mkdirSync(pkgDir, { recursive: true });
  const binaryPath = join(pkgDir, target.binary);
  cpSync(source, binaryPath);
  // upload-artifact does not preserve the execute bit.
  chmodSync(binaryPath, 0o755);
  writeFileSync(
    join(pkgDir, "package.json"),
    JSON.stringify(
      {
        name: target.name,
        version,
        description: `Ferrimock CLI native binary for ${target.os}-${target.cpu}`,
        license: "MIT OR Apache-2.0",
        repository: {
          type: "git",
          url: "git+https://github.com/salamaashoush/ferrimock.git",
        },
        os: [target.os],
        cpu: [target.cpu],
        files: [target.binary],
      },
      null,
      2
    ) + "\n"
  );
  generated.push(pkgDir);
}

if (generated.length === 0) {
  console.error("no platform packages generated - nothing to publish");
  process.exit(1);
}

const shimManifestPath = join(shimRoot, "package.json");
const shim = JSON.parse(readFileSync(shimManifestPath, "utf8"));
shim.version = version;
for (const target of Object.values(TARGETS)) {
  if (shim.optionalDependencies[target.name] !== undefined) {
    shim.optionalDependencies[target.name] = version;
  }
}
writeFileSync(shimManifestPath, JSON.stringify(shim, null, 2) + "\n");

console.log(`prepared ${generated.length} platform package(s) at ${npmRoot}`);
for (const dir of generated) {
  console.log(`  ${dir}`);
}
