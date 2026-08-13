#!/usr/bin/env node
// Ergonomics half of the suite.
//
// Two things are measured rather than asserted:
//   1. Config weight — non-empty, non-comment lines each tool needs to serve
//      the identical seven-endpoint API in `fixtures/`.
//   2. Capability — whether a tool can express a scenario at all, probed
//      against the live server by servers.mjs (results/servers.json) so the
//      matrix reflects behaviour, not documentation.
//
// The capability rows a benchmark cannot probe (recording, verification,
// browser support) are declared here with a source note, and are the only
// hand-maintained part of the file.
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import path from "node:path";
import { ROOT, FIXTURES, SCENARIOS } from "./lib/tools.mjs";

/** Config a user writes to serve the benchmark API. */
const CONFIGS = {
  ferrimock: [path.join(FIXTURES, "ferrimock", "mocks.yaml")],
  mockoon: [path.join(FIXTURES, "mockoon", "build.mjs")],
  prism: [path.join(FIXTURES, "prism", "openapi.yaml")],
  "json-server": [path.join(FIXTURES, "jsonserver", "db.json")],
  wiremock: [path.join(FIXTURES, "wiremock", "mappings", "benchmark.json")],
};

function significantLines(file) {
  if (!existsSync(file)) return null;
  return readFileSync(file, "utf8")
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0 && !line.startsWith("#") && !line.startsWith("//")).length;
}

/**
 * Capabilities no HTTP probe can reach. Each entry records where the claim
 * comes from so it can be rechecked when a tool ships a new version.
 */
const DECLARED = {
  "Record real traffic to mocks": {
    ferrimock: "yes — HAR import + `mock convert`",
    mockoon: "yes — proxy mode with recording",
    prism: "partial — `prism proxy` validates, does not persist mocks",
    "json-server": "no",
    wiremock: "yes — record and playback",
    source: "tool docs, Aug 2026",
  },
  "Assert which mocks ran": {
    ferrimock: "yes — always-on counts, `verify()`, `/calls`",
    mockoon: "no",
    prism: "no",
    "json-server": "no",
    wiremock: "yes — `verify()` on the admin API",
    source: "tool docs, Aug 2026",
  },
  "Explain why a request did not match": {
    ferrimock: "yes — per-criterion near misses in the 404 and `mock test --debug`",
    mockoon: "partial — request logs show the request, not the failed criterion",
    prism: "yes — spec validation errors",
    "json-server": "no",
    wiremock: "yes — near-miss diff",
    source: "tool docs, Aug 2026",
  },
  "Deterministic generated data": {
    ferrimock: "yes — `--seed`",
    mockoon: "no — faker is unseeded per request",
    prism: "yes — `--seed` (dynamic mode only)",
    "json-server": "n/a — static file",
    wiremock: "no",
    source: "tool docs, Aug 2026",
  },
  "Runs without a runtime dependency": {
    ferrimock: "yes — single static binary",
    mockoon: "needs Node",
    prism: "needs Node",
    "json-server": "needs Node",
    wiremock: "needs a JVM or Docker",
    source: "install docs, Aug 2026",
  },
  "Browser (service worker) mocking": {
    ferrimock: "no — native addon, no `setupWorker`",
    mockoon: "n/a — standalone server",
    prism: "n/a — standalone server",
    "json-server": "n/a — standalone server",
    wiremock: "n/a — standalone server",
    source: "ferrimock README; MSW is the tool that does this",
  },
};

const serversFile = path.join(ROOT, "results", "servers.json");
if (!existsSync(serversFile)) {
  console.error("run `node servers.mjs` first — the capability matrix is built from its probes");
  process.exit(1);
}
const servers = JSON.parse(readFileSync(serversFile, "utf8"));
const measured = servers.tools.filter((t) => !t.skipped && !t.error && t.kind !== "reference");

const report = { configWeight: {}, scenarioSupport: {}, declared: DECLARED };

for (const [tool, files] of Object.entries(CONFIGS)) {
  const counts = files.map(significantLines);
  report.configWeight[tool] = counts.some((c) => c === null)
    ? null
    : counts.reduce((a, b) => a + b, 0);
}

for (const scenario of SCENARIOS) {
  report.scenarioSupport[scenario.id] = Object.fromEntries(
    measured.map((t) => [t.id, !t.scenarios[scenario.id]?.unsupported]),
  );
}

mkdirSync(path.join(ROOT, "results"), { recursive: true });
writeFileSync(path.join(ROOT, "results", "ergonomics.json"), `${JSON.stringify(report, null, 2)}\n`);

const ids = measured.map((t) => t.id);
const labels = measured.map((t) => t.label);

console.log("Lines of config to serve the same seven endpoints (non-empty, non-comment).\n");
console.log(`| ${labels.join(" | ")} |`);
console.log(`| ${labels.map(() => "---:").join(" | ")} |`);
console.log(`| ${ids.map((id) => report.configWeight[id] ?? "—").join(" | ")} |`);

console.log("\nScenario support, probed against each running server.\n");
console.log(`| Scenario | ${labels.join(" | ")} |`);
console.log(`| --- | ${labels.map(() => ":---:").join(" | ")} |`);
for (const scenario of SCENARIOS) {
  const cells = ids.map((id) => (report.scenarioSupport[scenario.id][id] ? "yes" : "no"));
  console.log(`| ${scenario.label} | ${cells.join(" | ")} |`);
}

console.log("\nCapabilities outside the benchmark's reach (declared, with sources).\n");
console.log(`| Capability | ${labels.join(" | ")} |`);
console.log(`| --- | ${labels.map(() => "---").join(" | ")} |`);
for (const [capability, row] of Object.entries(DECLARED)) {
  console.log(`| ${capability} | ${ids.map((id) => row[id] ?? "—").join(" | ")} |`);
}
