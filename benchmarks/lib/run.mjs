import { spawn, execFile } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";

/**
 * Node's fetch initialises undici lazily, and the first call costs hundreds of
 * milliseconds. Paying that once up front keeps it out of every startup
 * measurement.
 */
export async function warmClient() {
  try {
    await fetch("http://127.0.0.1:1/", { signal: AbortSignal.timeout(50) });
  } catch {
    // Connection refused is the expected outcome; undici is loaded either way.
  }
}

/** Spawn a server, resolving once it answers or rejecting on early exit. */
export async function startServer(tool, { readyPath, timeoutMs = 30_000 }) {
  const [bin, args] = tool.command(tool.port);
  const started = process.hrtime.bigint();

  // stdout is discarded at the OS level rather than piped: several of these
  // servers log a line per request, and draining that pipe from JS throttles
  // the process being measured. stderr stays piped — it is quiet, and its
  // contents are the only useful diagnostic when a server fails to start.
  const child = spawn(bin, args, { stdio: ["ignore", "ignore", "pipe"] });
  let stderr = "";
  let exited = null;
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
    if (stderr.length > 8000) stderr = stderr.slice(-8000);
  });
  child.on("exit", (code, signal) => {
    exited = { code, signal };
  });

  const url = `http://127.0.0.1:${tool.port}${readyPath}`;
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    if (exited) {
      throw new Error(
        `${tool.label} exited before becoming ready (code ${exited.code}${exited.signal ? `, ${exited.signal}` : ""})\n${stderr.trim()}`,
      );
    }
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(1000) });
      // Any answer proves the listener is up; the probe's own status is
      // checked later by the support matrix.
      if (res.status > 0) {
        await res.arrayBuffer();
        const readyMs = Number(process.hrtime.bigint() - started) / 1e6;
        return { child, readyMs, stderr: () => stderr };
      }
    } catch {
      // not listening yet
    }
    // Tight poll: a coarse interval quantises the startup figure into
    // meaningless buckets for servers that come up in tens of milliseconds.
    await sleep(2);
  }

  child.kill("SIGKILL");
  throw new Error(`${tool.label} did not answer ${url} within ${timeoutMs}ms\n${stderr.trim()}`);
}

export async function stopServer(tool, handle) {
  if (tool.stop) {
    const [bin, args] = tool.stop();
    await new Promise((resolve) => execFile(bin, args, () => resolve()));
  }
  if (handle?.child && handle.child.exitCode === null) {
    handle.child.kill("SIGTERM");
    // Give it a beat to close the listener, then insist.
    await sleep(400);
    if (handle.child.exitCode === null) handle.child.kill("SIGKILL");
  }
  await sleep(200);
}

/** Resident set size in MiB for a pid tree, or null when unavailable. */
export async function rssMib(pid) {
  if (!pid) return null;
  return new Promise((resolve) => {
    execFile("ps", ["-o", "rss=", "-p", String(pid)], (err, stdout) => {
      if (err) return resolve(null);
      const kb = Number.parseInt(stdout.trim(), 10);
      resolve(Number.isFinite(kb) ? Math.round((kb / 1024) * 10) / 10 : null);
    });
  });
}

/**
 * The URL a scenario takes against one tool. A tool may address the same
 * semantic endpoint at its own path, or declare `null` for a scenario its model
 * cannot express at all.
 */
export function urlFor(tool, scenario) {
  const override = tool.paths?.[scenario.id];
  if (override === null) return null;
  return `http://127.0.0.1:${tool.port}${override ?? scenario.path}`;
}

/** Single request used to decide whether a tool supports a scenario at all. */
export async function probe(tool, scenario) {
  const url = urlFor(tool, scenario);
  if (url === null) return { ok: false, status: null, inexpressible: true, body: "" };
  try {
    const res = await fetch(url, {
      method: scenario.method,
      headers: scenario.headers,
      body: scenario.body,
      signal: AbortSignal.timeout(5000),
    });
    const text = await res.text();
    const status2xx = res.status >= 200 && res.status < 300;
    if (!status2xx) return { ok: false, status: res.status, body: text.slice(0, 400) };

    // A 2xx alone only proves the route exists. The scenario's own expectation
    // decides whether the tool actually implemented the behaviour — otherwise a
    // canned example counts as support for templating it never did.
    if (scenario.expect) {
      let satisfied = false;
      try {
        satisfied = Boolean(scenario.expect(text));
      } catch {
        satisfied = false;
      }
      if (!satisfied) {
        return { ok: false, status: res.status, wrongBody: true, body: text.slice(0, 400) };
      }
    }

    if (scenario.varies) {
      const second = await fetch(url, {
        method: scenario.method,
        headers: scenario.headers,
        body: scenario.body,
        signal: AbortSignal.timeout(5000),
      });
      const secondText = await second.text();
      if (secondText === text) {
        return { ok: false, status: res.status, staticBody: true, body: text.slice(0, 400) };
      }
    }

    return { ok: true, status: res.status, body: text.slice(0, 400) };
  } catch (err) {
    return { ok: false, status: 0, body: String(err) };
  }
}

/** Run oha against one endpoint and return its parsed summary. */
export async function load(tool, scenario, { duration, connections }) {
  const url = urlFor(tool, scenario);
  const args = [
    "--no-tui",
    "--output-format", "json",
    "-z", duration,
    "-c", String(connections),
    "-m", scenario.method,
  ];
  for (const [key, value] of Object.entries(scenario.headers ?? {})) {
    args.push("-H", `${key}: ${value}`);
  }
  if (scenario.body) args.push("-d", scenario.body);
  args.push(url);

  const stdout = await new Promise((resolve, reject) => {
    execFile("oha", args, { maxBuffer: 32 * 1024 * 1024 }, (err, out) => {
      if (err && !out) return reject(err);
      resolve(out);
    });
  });

  const report = JSON.parse(stdout);
  const codes = report.statusCodeDistribution ?? {};
  const success = Object.entries(codes)
    .filter(([code]) => Number(code) >= 200 && Number(code) < 300)
    .reduce((sum, [, n]) => sum + n, 0);
  const total = Object.values(codes).reduce((sum, n) => sum + n, 0);

  return {
    rps: Math.round(report.summary.requestsPerSec),
    p50Ms: round(report.latencyPercentiles?.p50),
    p95Ms: round(report.latencyPercentiles?.p95),
    p99Ms: round(report.latencyPercentiles?.p99),
    successRate: total ? Math.round((success / total) * 1000) / 10 : 0,
    total,
  };
}

/**
 * Latency on a single connection. With one request in flight there is no
 * queueing, so this isolates per-request cost — the figure that still means
 * something once throughput hits the host's ceiling.
 */
export async function unloadedLatency(tool, scenario, { duration = "2s" } = {}) {
  const stats = await load(tool, scenario, { duration, connections: 1 });
  return { p50Ms: stats.p50Ms, p99Ms: stats.p99Ms, rps: stats.rps };
}

function round(seconds) {
  if (typeof seconds !== "number") return null;
  return Math.round(seconds * 1e6) / 1e3;
}
