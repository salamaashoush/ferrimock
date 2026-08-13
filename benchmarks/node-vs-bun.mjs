#!/usr/bin/env node
// Superseded by fair.mjs — see the note in interceptor.mjs. This script loaded
// both interceptors into one process, so its runtime comparison carried the
// same ordering penalty. fair.mjs covers both runtimes via BENCH_RUNTIME.
console.error("node-vs-bun.mjs is retired. Use: BENCH_RUNTIME=node node fair.mjs\n");
process.exit(1);
