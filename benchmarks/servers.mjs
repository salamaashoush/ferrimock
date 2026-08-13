#!/usr/bin/env node
// Standalone mock-server benchmark: same seven endpoints, one tool at a time,
// driven by `oha`. Writes results/servers.json and prints a markdown summary.
import { writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { SCENARIOS, TOOLS, ROOT } from "./lib/tools.mjs";
import { load, probe, rssMib, startServer, stopServer, unloadedLatency, warmClient } from "./lib/run.mjs";

const DURATION = process.env.BENCH_DURATION ?? "5s";
const CONNECTIONS = Number(process.env.BENCH_CONNECTIONS ?? 50);
const WARMUP = process.env.BENCH_WARMUP ?? "1s";
const only = process.env.BENCH_ONLY?.split(",").map((s) => s.trim());

await warmClient();

const results = {
  meta: {
    duration: DURATION,
    connections: CONNECTIONS,
    warmup: WARMUP,
    platform: `${process.platform} ${process.arch}`,
    node: process.version,
  },
  tools: [],
};

for (const tool of TOOLS) {
  if (only && !only.includes(tool.id)) continue;

  if (!tool.available()) {
    console.log(`- ${tool.label}: skipped (${tool.hint})`);
    results.tools.push({ id: tool.id, label: tool.label, kind: tool.kind, skipped: tool.hint });
    continue;
  }

  console.log(`\n=== ${tool.label} (port ${tool.port}) ===`);
  let handle;
  const entry = { id: tool.id, label: tool.label, kind: tool.kind, scenarios: {} };

  try {
    handle = await startServer(tool, { readyPath: "/api/static" });
    entry.startupMs = Math.round(handle.readyMs);
    console.log(`  ready in ${entry.startupMs}ms`);

    entry.idleRssMib = await rssMib(handle.child.pid);

    for (const scenario of SCENARIOS) {
      const check = await probe(tool, scenario);
      if (!check.ok) {
        const why = check.inexpressible
          ? "not expressible in this tool"
          : check.staticBody
            ? "serves a fixed example, does not generate"
            : check.wrongBody
              ? "answers, but not with the behaviour the scenario asks for"
              : `HTTP ${check.status}`;
        console.log(`  ${scenario.id}: unsupported (${why})`);
        entry.scenarios[scenario.id] = {
          unsupported: true,
          status: check.status,
          reason: why,
          inexpressible: Boolean(check.inexpressible),
        };
        continue;
      }

      // Warm the JIT / caches so the measured window is steady-state.
      await load(tool, scenario, { duration: WARMUP, connections: CONNECTIONS });
      const stats = await load(tool, scenario, { duration: DURATION, connections: CONNECTIONS });
      const solo = await unloadedLatency(tool, scenario);
      entry.scenarios[scenario.id] = { ...stats, unloaded: solo };
      console.log(
        `  ${scenario.id.padEnd(12)} ${String(stats.rps).padStart(7)} rps  p50 ${stats.p50Ms}ms  p99 ${stats.p99Ms}ms  solo p50 ${solo.p50Ms}ms  ${stats.successRate}% 2xx`,
      );
    }

    entry.loadedRssMib = await rssMib(handle.child.pid);
  } catch (err) {
    console.log(`  FAILED: ${err.message}`);
    entry.error = err.message;
  } finally {
    await stopServer(tool, handle);
  }

  results.tools.push(entry);
}

mkdirSync(path.join(ROOT, "results"), { recursive: true });
const out = path.join(ROOT, "results", "servers.json");
writeFileSync(out, `${JSON.stringify(results, null, 2)}\n`);
console.log(`\nwrote ${out}`);

console.log(`\n${markdown(results)}`);

function markdown(res) {
  const active = res.tools.filter((t) => !t.skipped && !t.error);
  if (!active.length) return "_No tool produced results._";

  const lines = [];
  lines.push(`Requests/sec — ${res.meta.duration} at ${res.meta.connections} connections, higher is better.`);
  lines.push("");
  lines.push(`| Scenario | ${active.map((t) => t.label).join(" | ")} |`);
  lines.push(`| --- | ${active.map(() => "---:").join(" | ")} |`);

  for (const scenario of SCENARIOS) {
    const cells = active.map((t) => {
      const s = t.scenarios[scenario.id];
      if (!s) return "—";
      if (s.unsupported) return s.inexpressible ? "not expressible" : "n/a";
      return s.rps.toLocaleString();
    });
    lines.push(`| ${scenario.label} | ${cells.join(" | ")} |`);
  }

  lines.push("");
  lines.push("Single-connection p50 latency (ms) — no queueing, so this still separates tools once throughput hits the host ceiling.");
  lines.push("");
  lines.push(`| Scenario | ${active.map((t) => t.label).join(" | ")} |`);
  lines.push(`| --- | ${active.map(() => "---:").join(" | ")} |`);
  for (const scenario of SCENARIOS) {
    const cells = active.map((t) => {
      const s = t.scenarios[scenario.id];
      if (!s) return "—";
      if (s.unsupported) return "n/a";
      return `${s.unloaded.p50Ms}`;
    });
    lines.push(`| ${scenario.label} | ${cells.join(" | ")} |`);
  }

  lines.push("");
  lines.push(`| Metric | ${active.map((t) => t.label).join(" | ")} |`);
  lines.push(`| --- | ${active.map(() => "---:").join(" | ")} |`);
  lines.push(`| Startup to first response | ${active.map((t) => `${t.startupMs} ms`).join(" | ")} |`);
  lines.push(`| RSS idle | ${active.map((t) => (t.idleRssMib == null ? "—" : `${t.idleRssMib} MiB`)).join(" | ")} |`);
  lines.push(`| RSS after load | ${active.map((t) => (t.loadedRssMib == null ? "—" : `${t.loadedRssMib} MiB`)).join(" | ")} |`);

  return lines.join("\n");
}
