# Agentkernel startup/perf autoresearch

Goal: improve startup speed and overall benchmark performance for agentkernel using the checked-in benchmark harness as the single source of truth.

Primary metric
- `total_score` from `agentkernel benchmark --json`
- Higher is better
- Keep a change only if `total_score` improves, or ties while making the implementation simpler / safer

Default eval command
```bash
./autoresearch/run_eval.sh
```

Default target on this machine
- backend: docker
- image: alpine:3.20
- measured iterations: 5
- warmup iterations: 1

Why docker first
- It is the most repeatable backend on the current macOS laptop
- It is the main fallback backend for local development
- It maps cleanly to the startup/latency numbers documented in `BENCHMARK.md`

Files you may edit
- `src/backend/docker.rs`
- `src/docker_backend.rs`
- `src/vmm.rs`
- `src/benchmark.rs`
- `src/main.rs`
- `tests/benchmark_test.rs`
- `README.md`
- `BENCHMARK.md`
- files under `autoresearch/`

Files you should avoid editing unless a human explicitly asks
- unrelated product/API features
- issue tracking files except for normal bd or Dolt metadata noise
- large docs/plans outside benchmark/autoresearch scope

Loop
1. Run `./autoresearch/run_eval.sh` and inspect `autoresearch/latest-report.json`
2. Pick one narrow hypothesis
3. Make the smallest plausible code change
4. Run targeted tests first
5. Re-run `./autoresearch/run_eval.sh`
6. If `total_score` improved, record a row in `autoresearch/results.tsv` and keep the change
7. If the score regressed or stayed flat without a clear simplification win, revert the change

Good hypotheses
- reduce repeated process / manager setup on the hot path
- shrink docker CLI argument overhead on ephemeral runs
- reduce unnecessary filesystem or config work before start/exec
- improve benchmark harness determinism without weakening it
- improve startup path reuse for repeated commands

Guardrails
- Do not weaken the benchmark to get a better score
- Do not switch to a different backend mid-loop unless the run command explicitly says to
- Prefer deterministic changes over flaky concurrency tricks
- Keep diffs reviewable
- Preserve correctness and cleanup behavior

Reporting
- Log each kept attempt in `autoresearch/results.tsv`
- Include: timestamp, git sha, backend, score, startup_avg_ms, total_avg_ms, throughput_per_second, note

Quick commands
```bash
cargo test benchmark::tests -- --nocapture
cargo test --test benchmark_test -- --ignored --nocapture
./autoresearch/run_eval.sh
python3 ./autoresearch/score_benchmark.py autoresearch/latest-report.json
```
