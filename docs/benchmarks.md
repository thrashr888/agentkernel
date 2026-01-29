
# Benchmarks

agentkernel runs on five different backends across Linux and macOS. We benchmark all of them so you know exactly what to expect.

All numbers below are measured on real hardware -- an AMD EPYC server for Linux backends and an M3 Pro MacBook for macOS backends. No synthetic microbenchmarks. Every number represents the full end-to-end latency of `agentkernel run -- echo hello`, from command invocation to output.

## The headline numbers

| Backend | Latency | Throughput | Isolation |
|---------|---------|------------|-----------|
| **Hyperlight pool** (Linux) | **<1&micro;s** | ~3,300 RPS | Hypervisor + Wasm |
| **Firecracker daemon** (Linux) | **195ms** | ~5.1/sec | Full VM (separate kernel) |
| Docker (macOS) | ~220ms | ~4.5/sec | Container (shared kernel) |
| Docker pool (Linux) | ~250ms | ~4.0/sec | Container (shared kernel) |
| Podman (macOS) | ~300ms | ~3.3/sec | Container (rootless) |
| Podman (Linux) | ~310ms | ~3.2/sec | Container (rootless) |
| Firecracker cold (Linux) | 800ms | ~1.3/sec | Full VM (separate kernel) |
| Apple Containers (macOS 26+) | ~940ms | ~1.1/sec | Full VM (separate kernel) |

Pre-warmed pools make the fastest backends feel instant. Cold starts are still faster than most container runtimes.

## Where the time goes

Every sandbox execution has phases: boot the isolation boundary, wait for the environment to be ready, execute the command, then tear down. Here's how each backend breaks down:

| Backend | Boot | Ready | Exec | Shutdown |
|---------|------|-------|------|----------|
| **Hyperlight pool** | 0ms | <1&micro;s | <1ms | N/A |
| **Firecracker daemon** | 0ms | 0ms | 19ms | 0ms |
| Firecracker cold | 78ms | 110ms | 19ms | 20ms |
| Apple Containers | 860ms | 860ms | 95ms | 37ms |

Docker and Podman use a single `run --rm` operation internally, so their breakdown is a single combined step rather than separate phases.

The daemon and pool backends eliminate boot and shutdown by reusing pre-warmed instances. You pay the startup cost once, then every subsequent execution skips straight to the fast part.

## Firecracker vs Docker

The comparison that matters most -- VM isolation vs container isolation on the same Linux hardware:

| Metric | Docker | Firecracker | Winner |
|--------|--------|-------------|--------|
| Process start | 40ms | 46ms | Tie |
| Instance ready | 155ms | **110ms** | Firecracker |
| Command execution | 53ms | **19ms** | Firecracker (vsock) |
| Shutdown | 130ms | **20ms** | Firecracker (6.5x) |
| Memory per instance | ~50-100MB | **<10MB** | Firecracker (5-10x) |
| Isolation | Shared kernel | **Separate kernel** | Firecracker |

Firecracker uses vsock for command execution -- a direct host-to-VM communication channel that's 3x faster than Docker's exec path. Shutdown is 6.5x faster because there's no container runtime overhead.

And Firecracker's boot time was optimized from 961ms down to 110ms -- an **89% reduction** -- by disabling unnecessary kernel drivers:

| Optimization | Time saved |
|--------------|------------|
| Disable PS/2 keyboard driver (`i8042.nokbd`) | ~500ms |
| Skip PS/2 aux port probe (`i8042.noaux`) | ~260ms |
| Quiet boot (`quiet loglevel=4`) | ~90ms |

## Hyperlight: sub-microsecond execution

Hyperlight is the experimental backend that pushes the boundaries of what's possible. It uses Microsoft's hypervisor-isolated micro VMs to run WebAssembly modules with dual-layer security: a Wasm sandbox inside a hypervisor boundary.

The key number: **warm pool acquire takes 0.2&micro;s**. That's 50,000x faster than a cold Hyperlight startup (68ms) and over 1,000,000x faster than a Firecracker cold boot (800ms).

| Metric | Value |
|--------|-------|
| Cold startup | 68ms (avg), 67ms (p50) |
| Warm acquire | **0.2&micro;s** (avg), <1&micro;s (p50) |
| Function call | <1ms |
| 100 concurrent requests | **0.03s** (~3,333 RPS) |

For comparison, running 100 concurrent requests on other backends:

| Backend | 100 concurrent | RPS |
|---------|----------------|-----|
| **Hyperlight** | 0.03s | ~3,333 |
| Docker | 8.4s | ~12 |
| Podman | 18.2s | ~5.5 |

Hyperlight is 280x faster than Docker and 600x faster than Podman at concurrent workloads. The trade-off: it runs Wasm modules only, not arbitrary shell commands, and requires Linux with KVM.

## Apple Containers: VM isolation on macOS

Apple Containers (macOS 26+) give you Firecracker-like isolation on Apple Silicon without requiring Linux or KVM. Each container runs in its own VM with a separate kernel.

| Metric | Docker (macOS) | Apple Containers |
|--------|----------------|------------------|
| Isolation | Shared kernel | **Separate VM** |
| Boot time | ~175ms | ~860ms |
| Full lifecycle | ~500ms | ~940ms |
| Memory per instance | ~50MB | ~100MB+ |

Apple Containers are 2x slower than Docker on macOS, but they provide hardware-level isolation. If you're running untrusted code on macOS, that trade-off is worth it.

## Docker and Podman: the container backends

Both Docker and Podman use an optimized `run --rm` path that combines creation, execution, and cleanup into a single operation. This is 35x faster than the naive start-exec-stop cycle.

### macOS (M3 Pro)

| Backend | Latency | Cold start |
|---------|---------|------------|
| **Docker** | ~220ms | ~270ms |
| Podman | ~300ms | ~730ms |

Docker is ~30% faster on macOS due to its daemon architecture.

### Linux (AMD EPYC)

| Backend | Latency | Cold start |
|---------|---------|------------|
| **Podman** | ~310ms | ~350ms |
| Docker | ~350ms | ~550ms |

On Linux, Podman is ~10-15% faster because it runs daemonless -- no Docker daemon overhead.

## Daemon mode: 4x speedup for repeated commands

The daemon maintains a pool of 3-5 pre-booted Firecracker VMs. When you run a command, it grabs a warm VM from the pool, executes via vsock, and returns the VM for reuse.

| Metric | Ephemeral | Daemon | Speedup |
|--------|-----------|--------|---------|
| First command | 800ms | **195ms** | 4.1x |
| Subsequent | 800ms | **195ms** | 4.1x |
| 10 sequential | 8.0s | **1.95s** | 4.1x |
| VM reuse rate | 0% | ~95% | -- |

The daemon starts in ~3 seconds (pre-warms 3 VMs) and then every command benefits from the warm pool.

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

## Choosing a backend

| Use case | Recommended | Why |
|----------|-------------|-----|
| Interactive / API server | Firecracker daemon | 195ms latency, full VM isolation |
| High-throughput Wasm | Hyperlight pool | 3,300 RPS, sub-microsecond acquire |
| macOS development (speed) | Docker | Fastest macOS backend at ~220ms |
| macOS development (security) | Apple Containers | VM isolation on macOS |
| Linux CI/CD (no KVM) | Docker | Works without KVM |
| Untrusted code (Linux) | Firecracker | Separate kernel per sandbox |
| Untrusted code (macOS) | Apple Containers | Separate VM per sandbox |

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

Results are saved to `benchmark-results/` as JSON for comparison across runs.

## Test hardware

| Platform | CPU | Use |
|----------|-----|-----|
| Linux | AMD EPYC | Firecracker, Hyperlight, Docker, Podman |
| macOS | Apple M3 Pro | Docker, Podman, Apple Containers |
