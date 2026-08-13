#!/usr/bin/env node
// Superseded by fair.mjs.
//
// This script used to measure ferrimock and MSW in a single process. That is
// not a valid comparison: whichever interceptor loads second is penalised — 8x
// in the case measured here (MSW 28.9us alone, 232.5us when it followed
// ferrimock). Every cross-library number it produced was an ordering artifact.
//
// fair.mjs runs each library in its own process and alternates which goes
// first. Kept as a pointer so old invocations fail loudly instead of silently
// printing a wrong ratio.
console.error(
  "interceptor.mjs is retired: measuring both libraries in one process penalises\n" +
    "whichever loads second (measured at 8x). Use:\n\n" +
    "  node fair.mjs                      # bun\n" +
    "  BENCH_RUNTIME=node node fair.mjs   # node\n",
);
process.exit(1);
