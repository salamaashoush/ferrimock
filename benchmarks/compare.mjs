#!/usr/bin/env node
// Summarise interleaved A/B samples from regression.mjs.
//
// Runs alternate between the two checkouts, so slow machine drift affects both
// arms equally. Reports the median of each arm and the spread within it — a
// delta smaller than the spread is noise, not a regression.
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";

const dir = path.join(path.dirname(new URL(import.meta.url).pathname), "results");
const files = readdirSync(dir).filter((f) => f.startsWith("alt-") && f.endsWith(".json"));

const arms = { head: {}, now: {} };
for (const file of files) {
  const arm = file.startsWith("alt-head") ? "head" : "now";
  const raw = readFileSync(path.join(dir, file), "utf8");
  // A run still in flight has a partial file; skip it rather than abort.
  if (!raw.trim().endsWith("}")) continue;
  const data = JSON.parse(raw);
  for (const [id, stats] of Object.entries(data.cases)) {
    (arms[arm][id] ??= []).push(stats.bestUs);
  }
}

const median = (xs) => {
  const s = [...xs].sort((a, b) => a - b);
  return s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2;
};

const pairs = Math.min(
  ...Object.values(arms.head).map((v) => v.length),
  ...Object.values(arms.now).map((v) => v.length),
);
console.log(`${pairs} interleaved pair(s) per scenario\n`);
console.log("| Scenario | HEAD median | now median | delta | HEAD spread | now spread | verdict |");
console.log("| --- | ---: | ---: | ---: | ---: | ---: | --- |");

for (const id of Object.keys(arms.head)) {
  const h = arms.head[id];
  const n = arms.now[id];
  if (!n) continue;
  const hm = median(h);
  const nm = median(n);
  const delta = ((nm - hm) / hm) * 100;
  const hSpread = ((Math.max(...h) - Math.min(...h)) / hm) * 100;
  const nSpread = ((Math.max(...n) - Math.min(...n)) / nm) * 100;
  // A delta inside the noise band of either arm cannot be attributed.
  const noise = Math.max(hSpread, nSpread);
  const verdict = Math.abs(delta) < noise ? "within noise" : delta > 0 ? "SLOWER" : "faster";
  console.log(
    `| ${id} | ${hm.toFixed(2)} us | ${nm.toFixed(2)} us | ${delta >= 0 ? "+" : ""}${delta.toFixed(1)}% | ±${hSpread.toFixed(0)}% | ±${nSpread.toFixed(0)}% | ${verdict} |`,
  );
}
