#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BACKENDS="${AK_BENCH_BACKENDS:-docker}"
ITERATIONS="${AK_BENCH_ITERATIONS:-5}"
WARMUP="${AK_BENCH_WARMUP:-1}"
IMAGE="${AK_BENCH_IMAGE:-alpine:3.24}"
REPORT="${AK_BENCH_REPORT:-autoresearch/latest-report.json}"

mkdir -p "$(dirname "$REPORT")"

cargo run -- benchmark \
  --backends "$BACKENDS" \
  --iterations "$ITERATIONS" \
  --warmup "$WARMUP" \
  --image "$IMAGE" \
  --json \
  --output "$REPORT" \
  > /tmp/agentkernel-benchmark.json

python3 ./autoresearch/score_benchmark.py "$REPORT"
