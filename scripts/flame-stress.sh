#!/usr/bin/env bash
set -euo pipefail

# Generate a flamegraph for the concurrent stress test.
# Usage:
#   scripts/flame-stress.sh [--dev-profile] [mode] [concurrency] [total] [outdir]
# Modes: enqueue (default), drain, or mixed
# Defaults: mode=enqueue, concurrency=32, total=20000, outdir=flamegraphs

# Optional: dev profile toggle
DEV_FLAG=""
if [ "${1:-}" = "--dev-profile" ]; then
  DEV_FLAG="--dev"
  shift
fi

# Parse optional mode
MODE="${1:-${SQEW_STRESS_MODE:-enqueue}}"
if [ "$MODE" = "enqueue" ] || [ "$MODE" = "drain" ] || [ "$MODE" = "mixed" ]; then
  shift || true
else
  MODE="enqueue"
fi

CONC="${1:-${SQEW_STRESS_CONCURRENCY:-32}}"
TOTAL="${2:-${SQEW_STRESS_TOTAL:-20000}}"
OUTDIR="${3:-flamegraphs}"
mkdir -p "$OUTDIR"

STAMP=$(date +%Y%m%d-%H%M%S)
OUT="$OUTDIR/flame-stress-${MODE}-${CONC}-${TOTAL}-${STAMP}.svg"

if ! command -v cargo-flamegraph >/dev/null 2>&1; then
  echo "cargo-flamegraph not found. Install with:"
  echo "  cargo install flamegraph"
  echo "Linux requires 'perf'; macOS uses dtrace and typically needs sudo."
  exit 1
fi

OS="$(uname -s)"
SUDO=""
INLINE_FLAG=""
if [ "$OS" = "Darwin" ]; then
  # macOS flamegraph typically needs root (dtrace); -E preserves env vars
  SUDO="sudo -E"
  echo "[info] macOS detected: running with sudo for dtrace access."
elif [ "$OS" = "Linux" ]; then
  INLINE_FLAG="--no-inline" # supported by cargo-flamegraph on Linux/perf
fi

PROFILE="${DEV_FLAG:+dev}"; PROFILE="${PROFILE:-release}"
echo "[info] Generating flamegraph -> $OUT (profile=$PROFILE mode=$MODE concurrency=$CONC total=$TOTAL)"

# Ensure debug symbols are present in release builds for readable stacks
RUSTFLAGS="${RUSTFLAGS:-} -C debuginfo=2 -C force-frame-pointers=yes -C lto=no -C codegen-units=1" \
  SQEW_STRESS_CONCURRENCY="$CONC" \
  SQEW_STRESS_TOTAL="$TOTAL" \
  $SUDO cargo flamegraph $DEV_FLAG $INLINE_FLAG --output "$OUT" --test stress_tests -- --ignored --nocapture \
  "$([ "$MODE" = "drain" ] && echo concurrent_enqueue_and_drain_no_loss || ([ "$MODE" = "mixed" ] && echo concurrent_mixed_produce_consume_counts_ok || echo concurrent_enqueue_no_loss))"

echo "[done] Flamegraph written to: $OUT"
