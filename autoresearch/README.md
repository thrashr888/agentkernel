# Agentkernel autoresearch workspace

This folder contains the local scaffold for benchmark-driven startup and performance work on agentkernel.

## Goal

Use a deterministic benchmark loop to improve:
- end-to-end startup speed
- backend startup latency
- overall benchmark throughput / total latency

The primary optimization target is the scalar `total_score` emitted by the benchmark JSON report.

## Current files

- `program.md` — instructions for autonomous keep/discard optimization loops
- `run_eval.sh` — canonical local eval entrypoint
- `score_benchmark.py` — pretty-prints key metrics from a JSON report
- `results.tsv` — append-only experiment log of kept baselines/improvements
- `progress.png` — chart of score/latency/throughput history across runs
- `latest-report.json` — most recent local benchmark output
- `docker-report.json` — explicit docker comparison run artifact
- `apple-report.json` — explicit apple comparison run artifact

## Canonical eval command

```bash
./autoresearch/run_eval.sh
```

Environment overrides:

```bash
AK_BENCH_BACKENDS=docker
AK_BENCH_ITERATIONS=5
AK_BENCH_WARMUP=1
AK_BENCH_IMAGE=alpine:3.20
./autoresearch/run_eval.sh
```

## Current baseline

From `latest-report.json`:

- backend: docker
- total_score: 4.43
- startup_avg_ms: 201.08
- exec_avg_ms: 69.17
- total_avg_ms: 292.78
- throughput_per_second: 3.42

Notes:
- `total_avg_ms` now reflects end-to-end CLI `agentkernel run` timing from a temp working directory, not just internal lifecycle timing.
- the JSON also includes `lifecycle_total` for the slower create/start/exec/stop/remove path used for backend diagnostics.
- Apple end-to-end one-shot benchmarking now works via an Apple-native `container run --rm` fast path.

Latest comparison artifacts:
- docker: `autoresearch/docker-report.json` → total_score 7.17, total_avg_ms 196.05
- apple: `autoresearch/apple-report.json` → total_score 2.57, total_avg_ms 719.67
- broader history including repeat runs is visualized in `autoresearch/progress.png`

Recent read from repeat runs:
- docker remains much faster and more stable on this machine
- apple now works end-to-end, but shows very high startup variance with occasional severe cold-start outliers

## What counts as startup work

There are two related layers:

1. benchmarked backend lifecycle time
   - sandbox create/start/exec/stop/remove
2. CLI/process startup overhead before the backend work begins
   - config loading
   - backend detection
   - policy initialization
   - runtime probing
   - other repeated per-invocation setup

The first scaffold captured backend lifecycle metrics well. The next phase is to make the benchmark more sensitive to the second category too.

## Current hypothesis backlog

1. cache config and enterprise policy initialization on the `VmManager::with_backend` hot path
2. reduce repeated backend/runtime detection and sandbox-state probing during one-shot `run`
3. capture a true end-to-end `agentkernel run -- echo hello` metric in the benchmark report
4. compare backends explicitly instead of optimizing only for docker
5. keep backend-specific fast paths where they help without weakening correctness

## Backend notes

- `docker`: primary local backend on this macOS machine; best current repeatable baseline
- `podman`: should be benchmarked too when available, especially for daemonless behavior
- `apple`: important for native macOS isolation tradeoffs
- `firecracker`: likely needs separate handling because it falls back from ephemeral mode
- `hyperlight`: very different execution model; benchmark separately when available

## Working-session guidance

When resuming:

1. inspect `results.tsv`
2. inspect `latest-report.json`
3. run `./autoresearch/run_eval.sh`
4. compare against the logged baseline
5. keep only improvements or meaningful same-score simplifications

## Logging rule

Every kept change should add one row to `results.tsv` with:
- timestamp
- git sha
- backend
- total_score
- startup_avg_ms
- total_avg_ms
- throughput_per_second
- short note
