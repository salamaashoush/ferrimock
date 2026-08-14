#!/usr/bin/env bash
# Run the fuzz targets.
#
#   scripts/fuzz.sh            # every target, 60s each -- what CI runs
#   scripts/fuzz.sh 300        # every target, 300s each
#   scripts/fuzz.sh 0 consolidate   # one target, until you stop it
#
# cargo-fuzz needs nightly for the sanitizer instrumentation, so the toolchain
# pinned in rust-toolchain.toml is deliberately overridden here.
set -euo pipefail

SECONDS_PER_TARGET="${1:-60}"
ONLY_TARGET="${2:-}"

cd "$(dirname "$0")/.."

if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
  echo "cargo-fuzz is not installed for nightly. Install it with:" >&2
  echo "  cargo +nightly install cargo-fuzz" >&2
  exit 1
fi

targets=(consolidate normalize_path response_envelope)
if [[ -n "$ONLY_TARGET" ]]; then
  targets=("$ONLY_TARGET")
fi

for target in "${targets[@]}"; do
  echo "==> fuzzing $target"
  if [[ "$SECONDS_PER_TARGET" == "0" ]]; then
    cargo +nightly fuzz run "$target"
  else
    # `-runs` is not a time bound, so cap wall clock instead: a target that
    # finds nothing in the budget has earned its pass for this run.
    cargo +nightly fuzz run "$target" -- -max_total_time="$SECONDS_PER_TARGET"
  fi
done

echo "==> all targets clean"
