
# Benchmarks

This page preserves historical measurements from different hosts and harnesses.
They are not a single current-release benchmark or an apples-to-apples ranking.
Some older results lack checked-in raw reports and complete version metadata;
treat them as observations to reproduce, not performance guarantees.

## Read the timing boundary first

| Measurement | What it includes | What it does not establish |
|-------------|------------------|----------------------------|
| Warm pool acquire | Retrieving an already prepared instance | Full command latency, image preparation, or VM boot |
| Guest boot / ready | One startup phase | Host setup, command execution, and cleanup |
| Exec on a running sandbox | A command in an existing environment | The cost of creating that environment |
| Full lifecycle | The phases included by the particular harness | A universal result across hosts and cold/warm states |
| Concurrent throughput | Completed work over a batch's wall time | The reciprocal of a single-request latency |

Use the [reproduction commands](#running-your-own-benchmarks) with the same
workload, timing boundary, runtime versions, and hardware when comparing
backends. Hyperlight runs Wasm workloads; its pool timings are not shell-command
or Firecracker lifecycle measurements.

## Where the time goes

The historical phase observations below came from different paths. They are not additive components of one end-to-end measurement; in particular, the Apple boot and ready measurements can overlap.

| Backend | Boot | Ready | Exec | Shutdown |
|---------|------|-------|------|----------|
| **Hyperlight pool** | 0ms | <1&micro;s | <1ms | N/A |
| **Firecracker daemon** | 0ms | 0ms | 19ms | 0ms |
| Firecracker cold | 78ms | 110ms | 19ms | 20ms |
| Apple Containers | 860ms | 860ms | 95ms | 37ms |

Docker and Podman use a single `run --rm` operation internally, so their breakdown is a single combined step rather than separate phases.

The daemon and pool backends eliminate boot and shutdown by reusing pre-warmed instances. You pay the startup cost once, then every subsequent execution skips straight to the fast part.

## Firecracker vs Docker

Historical per-operation observations on a Linux host. The memory row is retained as reported, but the original accounting boundary is unspecified; it must not be interpreted as total guest plus host memory.

| Metric | Docker | Firecracker | Winner |
|--------|--------|-------------|--------|
| Process start | 40ms | 46ms | Tie |
| Instance ready | 155ms | **110ms** | Firecracker |
| Command execution | 53ms | **19ms** | Firecracker (vsock) |
| Shutdown | 130ms | **20ms** | Firecracker (6.5x) |
| Memory per instance | ~50-100MB | **<10MB** | Firecracker (5-10x) |
| Isolation | Shared kernel | **Separate kernel** | Firecracker |

The reported exec and shutdown observations differ, but they do not isolate the cause or establish a general speedup for current builds. Repeat the same workload before drawing that conclusion.

And Firecracker's boot time was optimized from 961ms down to 110ms -- an **89% reduction** -- by disabling unnecessary kernel drivers:

| Optimization | Time saved |
|--------------|------------|
| Disable PS/2 keyboard driver (`i8042.nokbd`) | ~500ms |
| Skip PS/2 aux port probe (`i8042.noaux`) | ~260ms |
| Quiet boot (`quiet loglevel=4`) | ~90ms |

## Hyperlight: pool acquisition and Wasm calls

Hyperlight is the experimental backend that pushes the boundaries of what's possible. It uses Microsoft's hypervisor-isolated micro VMs to run WebAssembly modules with dual-layer security: a Wasm sandbox inside a hypervisor boundary.

The historical **0.2&micro;s warm acquire** figure measures acquiring a prepared pool entry. It excludes startup and guest execution and must not be presented as end-to-end sandbox execution latency.

| Metric | Value |
|--------|-------|
| Cold startup | 68ms (avg), 67ms (p50) |
| Warm acquire | **0.2&micro;s** (avg), <1&micro;s (p50) |
| Function call | <1ms |
| 100 concurrent requests | **0.03s** (~3,333 RPS) |

Historical 100-request batches are retained below. Workload equivalence between Wasm and container paths is not established:

| Backend | 100 concurrent | RPS |
|---------|----------------|-----|
| **Hyperlight** | 0.03s | ~3,333 |
| Docker | 8.4s | ~12 |
| Podman | 18.2s | ~5.5 |

These batches are not a supported cross-backend speedup claim. Hyperlight runs Wasm modules rather than arbitrary shell commands and requires a supported Linux/KVM build.

## Apple Containers: VM isolation on macOS

Apple Containers (macOS 26+) give you Firecracker-like isolation on Apple Silicon without requiring Linux or KVM. Each container runs in its own VM with a separate kernel.

| Metric | Docker (macOS) | Apple Containers |
|--------|----------------|------------------|
| Isolation | Shared kernel | **Separate VM** |
| Boot time | ~175ms | ~860ms |
| Full lifecycle | ~500ms | ~940ms |
| Memory per instance | ~50MB | ~100MB+ |

This historical lifecycle comparison uses different isolation boundaries. Choose the required boundary first, then measure the current runtime on your host.

### Measuring Apple startup locally

The backend starts the Apple container service on demand. `container system start`
waits for the API service to become responsive, so the backend does not add a
second fixed sleep after that command. Measure the complete lifecycle on a host
with an available Apple container runtime:

This readiness behavior is part of the Apple CLI's
[`system start` implementation](https://github.com/apple/container/blob/main/Sources/ContainerCommands/System/SystemStart.swift),
not an agentkernel assumption.

After removing AgentKernel's redundant 500 ms post-readiness delay, a
2026-08-23 verification run on an Apple M5 Max with macOS 27.0 and Apple
container 1.2.2 averaged **461.48 ms startup** and **483.44 ms total**. These
numbers are recorded separately from the historical M3 Pro table above because
they were measured on different hardware.

```bash
cargo run -- benchmark --backends apple --iterations 10 --warmup 2 --json \
  --output benchmark-results/apple-current.json
```

The report's `startup` metric covers sandbox creation and start, while `total`
covers the full command path. Record the macOS version, Apple container CLI
version, image tag, and whether the service was already running alongside the
JSON report. Do not compare a run that failed the runtime readiness check with
the historical measurements above.

## Docker and Podman: the container backends

Both Docker and Podman use an optimized `run --rm` path that combines creation, execution, and cleanup into a single operation. Measure this separately from a multi-command create/start/exec/stop workflow.

### macOS (M3 Pro)

| Backend | Latency | Cold start |
|---------|---------|------------|
| **Docker** | ~220ms | ~270ms |
| Podman | ~300ms | ~730ms |

The historical macOS timings favor Docker for this workload; they do not isolate daemon architecture as the cause.

### Linux (AMD EPYC)

| Backend | Latency | Cold start |
|---------|---------|------------|
| **Podman** | ~310ms | ~350ms |
| Docker | ~350ms | ~550ms |

The historical Linux timings favor Podman for this workload; this is not a general guarantee for other runtime versions or configurations.

## Daemon mode: historical warm-pool measurements

The daemon maintains a pool of 3-5 pre-booted Firecracker VMs. When you run a command, it grabs a warm VM from the pool, executes via vsock, and returns the VM for reuse.

| Metric | Ephemeral | Daemon | Speedup |
|--------|-----------|--------|---------|
| First command | 800ms | **195ms** | 4.1x |
| Subsequent | 800ms | **195ms** | 4.1x |
| 10 sequential | 8.0s | **1.95s** | 4.1x |
| VM reuse rate | 0% | ~95% | -- |

These historical command timings assume a ready pool. Daemon startup and pre-warming were reported separately at about 3 seconds and are excluded from the table. Pool exhaustion and workload changes require separate measurement.

## Stress test results

### Docker (macOS) -- 10 parallel sandboxes

| Metric | Value |
|--------|-------|
| Total time | 4.5s |
| Success rate | **100%** |
| Full lifecycle (avg) | 446ms |
| Create (avg) | 44ms |
| Start (avg) | 174ms |
| Exec (avg) | 83ms |
| Stop (avg) | 109ms |
| Remove (avg) | 41ms |

### Docker (macOS) -- 10 cycles, 5x2 iterations

| Metric | Value |
|--------|-------|
| Throughput | 1.8-2.0/sec |
| p50 latency | 498ms |
| p95 latency | 702ms |
| p99 latency | 1028ms |

### Docker (Linux) -- 100 cycles, 10x10 iterations

| Metric | Value |
|--------|-------|
| Total wall time | 119.4s |
| Success rate | **100%** |
| Avg lifecycle | 1,194ms |
| p50 | 1,178ms |
| p95 | 1,458ms |
| p99 | 1,705ms |
| Throughput | 0.84/sec |

## Orchestration backends: Kubernetes and Nomad

The orchestration backends run sandboxes on remote clusters instead of the local machine. This adds network overhead but enables team-scale and multi-tenant deployments.

Numbers below are measured on both platforms: an AMD EPYC server (16 cores, 57 GB) with k3d single-node and Nomad dev agent for Linux, and an M3 Pro MacBook (12 cores, 36 GB) with k3d and Nomad for macOS.

### Single sandbox lifecycle

Full create → start → exec → stop cycle, averaged over 5 iterations:

**Linux (AMD EPYC)**

| Operation | Kubernetes | Nomad | Docker (baseline) |
|-----------|-----------|-------|-------------------|
| Create | 92ms | 39ms | 47ms |
| Start | 904ms | 811ms | 198ms |
| Exec | 128ms | 165ms | 68ms |
| Stop | 101ms | 38ms | 152ms |
| **Total** | **1,225ms** | **1,053ms** | **465ms** |

**macOS (M3 Pro)**

| Operation | Kubernetes | Nomad | Docker (baseline) |
|-----------|-----------|-------|-------------------|
| Create | 100ms | 166ms | 104ms |
| Start | 785ms | 1,280ms | 231ms |
| Exec | 120ms | 392ms | 116ms |
| Stop | 78ms | 159ms | 169ms |
| **Total** | **1,083ms** | **1,997ms** | **620ms** |

In this table Docker has the lowest full-lifecycle total on both hosts. Exec results differ by host: Docker is lowest on Linux, while Kubernetes is lowest on macOS. These observations do not establish the cause.

### One-shot `run` command

`agentkernel run --backend <backend> -- echo hello` (full lifecycle in one command):

| Backend | Linux | macOS |
|---------|-------|-------|
| Kubernetes | 571ms | 594ms |
| Nomad | 569ms | 580ms |
| Docker | 580ms | 577ms |

The reported one-shot results are close for this workload. They are distinct from the sequential-exec throughput measurements below.

### Exec throughput

50 sequential `exec` calls on a single running sandbox:

| Backend | Linux avg | Linux RPS | macOS avg | macOS RPS |
|---------|-----------|-----------|-----------|-----------|
| Docker | 67ms | 14.8/sec | 103ms | 9.6/sec |
| **Kubernetes** | **128ms** | **7.7/sec** | **99ms** | **10.0/sec** |
| Nomad | 163ms | 6.1/sec | 365ms | 2.7/sec |

Kubernetes and Docker trade the lead depending on platform. Nomad's `alloc exec` CLI path adds overhead per call.

### Concurrent scale

How many sandboxes can run simultaneously on a single node:

**Kubernetes (k3d single-node)**

| Count | Create | Start | Running | Parallel exec |
|-------|--------|-------|---------|---------------|
| 5 | 116ms | 1.4s | 5/5 | 124ms |
| 10 | 134ms | 1.4s | 10/10 | 163ms |
| 20 | 241ms | 120s | 15/20 | 287ms |

The historical single-node k3d run started 15 of the 20 requested pods. Capacity depends on node resources and workload; this does not establish a production-cluster limit.

**Nomad (local dev agent)**

| Count | Create | Start | Running | Parallel exec |
|-------|--------|-------|---------|---------------|
| 5 | 46ms | 814ms | 5/5 | 171ms |
| 10 | 52ms | 834ms | 10/10 | 197ms |
| 20 | 107ms | 1.4s | 20/20 | 310ms |

Nomad started all 20 requested sandboxes in this run. Different runtime configurations and resource limits prevent treating this result as a general scheduler-resilience comparison.

## Choosing a backend

Use the [sandbox selection guide](choosing-a-sandbox.md) to choose by workload,
execution location, and isolation requirements. Then benchmark that backend on
the host you intend to operate. Historical timings alone are not a deployment
recommendation.

## Running your own benchmarks

```bash
# Stress test (parallel sandbox creation)
cargo test --test stress_test -- --nocapture --ignored

# Benchmark test (repeated lifecycle with statistics)
cargo test --test benchmark_test -- --nocapture --ignored

# Shell script (per-operation latency)
./scripts/benchmark.sh

# Throughput test (100 commands, 10 concurrent)
./scripts/stress-test.sh 100 10
```

Configure with environment variables:

```bash
# Stress test
STRESS_VM_COUNT=1000 STRESS_MAX_CONCURRENT=100 cargo test --test stress_test -- --nocapture --ignored

# Benchmark test
BENCH_SANDBOXES=20 BENCH_ITERATIONS=5 cargo test --test benchmark_test -- --nocapture --ignored
```

The Rust benchmark and stress tests write reports under `benchmark-results/`. The shell stress script uses a temporary results directory printed by the script. Record the host, OS, runtime versions, image, source revision, concurrency, and timing boundary with each report.

## Test hardware

| Platform | CPU | Use |
|----------|-----|-----|
| Linux | AMD EPYC (16 cores, 57 GB) | Firecracker, Hyperlight, Docker, Podman, Kubernetes (k3d), Nomad |
| macOS | Apple M3 Pro (12 cores, 36 GB) | Docker, Podman, Apple Containers, Kubernetes (k3d), Nomad |
