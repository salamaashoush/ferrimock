#!/usr/bin/env node
// Authoritative ferrimock-vs-MSW comparison.
//
// Each measurement runs in its OWN process (isolated.mjs), because loading two
// fetch interceptors into one process penalises whichever runs second — by 8x
// in the case that motivated this harness. Order is alternated across passes so
// residual machine drift cannot favour either side.
import { execFileSync } from "node:child_process";

const SCENARIOS = ["static", "params", "fake"];
const PASSES = Number(process.env.BENCH_PASSES ?? 3);
const LABELS = { static: "Static JSON", params: "Path params", fake: "Handler + fake data" };

const samples = { ferrimock: {}, msw: {} };

const RUNTIME = process.env.BENCH_RUNTIME ?? "bun";

function run(lib, scenario) {
  const out = execFileSync(RUNTIME, ["isolated.mjs"], {
    env: { ...process.env, BENCH_LIB: lib, BENCH_SCENARIO: scenario },
    encoding: "utf8",
  });
  return JSON.parse(out.trim().split("\n").at(-1)).usPerReq;
}

for (let pass = 0; pass < PASSES; pass += 1) {
  // Flip which library goes first each pass.
  const order = pass % 2 === 0 ? ["ferrimock", "msw"] : ["msw", "ferrimock"];
  for (const scenario of SCENARIOS) {
    for (const lib of order) {
      (samples[lib][scenario] ??= []).push(run(lib, scenario));
    }
  }
  console.error(`pass ${pass + 1}/${PASSES} done`);
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2;
};

const runtime = RUNTIME;
console.log(`\n| Scenario | Ferrimock | MSW | Ratio |`);
console.log(`| --- | ---: | ---: | ---: |`);
const report = {};
for (const scenario of SCENARIOS) {
  const f = median(samples.ferrimock[scenario]);
  const m = median(samples.msw[scenario]);
  const ratio = m / f;
  report[scenario] = { ferrimockUs: +f.toFixed(2), mswUs: +m.toFixed(2), ratio: +ratio.toFixed(2) };
  const verdict = ratio >= 1 ? `**${ratio.toFixed(2)}x faster**` : `${(1 / ratio).toFixed(2)}x slower`;
  console.log(`| ${LABELS[scenario]} | ${f.toFixed(1)} us | ${m.toFixed(1)} us | ${verdict} |`);
}
console.error(`\n${JSON.stringify({ runtime, passes: PASSES, report }, null, 2)}`);
