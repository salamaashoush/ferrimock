import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

export const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
export const REPO = path.dirname(ROOT);
export const FIXTURES = path.join(ROOT, "fixtures");
const BIN = path.join(ROOT, "node_modules", ".bin");

const ferrimockBin = path.join(REPO, "target", "release", "ferrimock");

/**
 * Every tool serves the same seven endpoints from its own idiomatic config, so
 * a row compares like with like. `available` gates on the binary actually being
 * installed — a missing tool is reported as skipped, never silently dropped.
 */
export const TOOLS = [
  {
    // Not a mock server: a bare axum handler returning a constant string. It
    // marks how fast anything can answer on this host, so a tool sitting at
    // this line is host-bound, not tool-bound.
    id: "baseline",
    label: "Bare axum (reference floor)",
    kind: "reference",
    port: 4100,
    available: () => existsSync(path.join(REPO, "target", "release", "bench-baseline")),
    hint: "cd benchmarks/baseline && CARGO_TARGET_DIR=../../target cargo build --release",
    command: (port) => [path.join(REPO, "target", "release", "bench-baseline"), [String(port)]],
  },
  {
    id: "ferrimock",
    label: "Ferrimock",
    kind: "native",
    port: 4101,
    available: () => existsSync(ferrimockBin),
    hint: "cargo build --release -p ferrimock-cli",
    command: (port) => [
      ferrimockBin,
      ["mock", "serve", "--port", String(port), "-f", path.join(FIXTURES, "ferrimock", "mocks.yaml"), "--no-explain"],
    ],
  },
  {
    id: "mockoon",
    label: "Mockoon CLI",
    kind: "native",
    port: 4102,
    available: () => existsSync(path.join(BIN, "mockoon-cli")),
    hint: "npm install",
    command: (port) => [
      path.join(BIN, "mockoon-cli"),
      [
        "start",
        "--data", path.join(FIXTURES, "mockoon", "environment.json"),
        "--port", String(port),
        "--disable-log-to-file",
        "--disable-admin-api",
      ],
    ],
  },
  {
    id: "prism",
    label: "Prism (OpenAPI)",
    kind: "native",
    port: 4103,
    available: () => existsSync(path.join(BIN, "prism")),
    hint: "npm install",
    command: (port) => [
      path.join(BIN, "prism"),
      // The document is positional — `-d` is `--dynamic`. Logging is silenced
      // so the row measures serving, not console output.
      ["mock", path.join(FIXTURES, "prism", "openapi.yaml"), "-p", String(port), "-h", "127.0.0.1", "-v", "silent"],
    ],
  },
  {
    id: "json-server",
    label: "json-server",
    kind: "native",
    port: 4104,
    available: () => existsSync(path.join(BIN, "json-server")),
    hint: "npm install",
    command: (port) => [
      path.join(BIN, "json-server"),
      [path.join(FIXTURES, "jsonserver", "db.json"), "--port", String(port), "--host", "127.0.0.1"],
    ],
    // json-server serves resources from a JSON document; it has no templating,
    // no header matching and no request echo. `null` marks a scenario its model
    // cannot express, which is a fairer result than a 404 against a URL it was
    // never going to serve.
    paths: {
      static: "/static",
      "path-param": "/users/42",
      dynamic: null,
      "list-20": null,
      conditional: null,
      "post-echo": null,
    },
  },
  {
    id: "wiremock",
    label: "WireMock (Docker)",
    kind: "docker",
    port: 4105,
    // Docker Desktop on macOS proxies through a VM, so this row carries
    // container networking overhead the native rows do not. Kept separate.
    available: () => process.env.BENCH_WIREMOCK === "1",
    hint: "BENCH_WIREMOCK=1 (requires Docker; adds VM networking overhead)",
    command: (port) => [
      "docker",
      [
        "run", "--rm", "--name", "ferrimock-bench-wiremock",
        "-p", `${port}:8080`,
        "-v", `${path.join(FIXTURES, "wiremock")}:/home/wiremock`,
        "wiremock/wiremock:3.9.1",
        "--disable-banner",
      ],
    ],
    stop: () => ["docker", ["rm", "-f", "ferrimock-bench-wiremock"]],
  },
];

/** Endpoints exercised by the load phase. */
export const SCENARIOS = [
  {
    id: "static",
    label: "Static JSON body",
    method: "GET",
    path: "/api/static",
  },
  {
    id: "path-param",
    label: "Path param in body",
    method: "GET",
    path: "/api/users/42",
    expect: (body) => String(JSON.parse(body).id) === "42",
  },
  {
    id: "dynamic",
    label: "Generated fake data (5 fields)",
    method: "GET",
    path: "/api/users/42/profile",
    // Two calls must differ somewhere, otherwise the tool is serving a fixed
    // example rather than generating data.
    varies: true,
    expect: (body) => {
      const json = JSON.parse(body);
      return typeof json.name === "string" && typeof json.email === "string";
    },
  },
  {
    id: "list-20",
    label: "20-item generated list",
    method: "GET",
    path: "/api/list",
    expect: (body) => JSON.parse(body).items?.length === 20,
  },
  {
    id: "conditional",
    label: "Header-conditional route",
    method: "GET",
    path: "/api/whoami",
    headers: { authorization: "Bearer admin-abc" },
    // The admin header must actually change the answer.
    expect: (body) => JSON.parse(body).role === "admin",
  },
  {
    id: "post-echo",
    label: "POST with JSON body echo",
    method: "POST",
    path: "/api/echo",
    headers: { "content-type": "application/json" },
    body: '{"name":"John","tags":["a","b"]}',
    // The request body must come back, not a canned example. Tools differ on
    // whether the echo lands as an object or as a JSON string; both count —
    // the capability under test is "echoes the request", not its encoding.
    expect: (body) => {
      const received = JSON.parse(body)?.received;
      if (received && typeof received === "object") return received.name === "John";
      return typeof received === "string" && JSON.parse(received)?.name === "John";
    },
  },
];
