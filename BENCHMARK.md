# Agentkernel Benchmarks

Performance measurements for sandbox lifecycle operations.

## Quick Summary

| Backend | Platform | Avg Boot | Ready Time | Exec Latency | Throughput |
|---------|----------|----------|------------|--------------|------------|
| Docker | macOS (M1) | 188ms | 188ms | 83ms | 2.0 sandboxes/sec |
| Docker | Linux (AMD EPYC) | 155ms | 155ms | 53ms | ~4 sandboxes/sec |
| Firecracker | Linux (AMD EPYC) | 78ms | 1015ms | 19ms | ~1 sandbox/sec |

**Key insight**: Firecracker hypervisor overhead is just 78ms (faster than Docker). The 1015ms "ready" time includes full kernel boot + userspace init + guest agent startup. Once running, Firecracker has 3x lower exec latency.

## Docker Backend (macOS)

Tested on Apple Silicon (M1) with Docker via Colima. This is the fallback backend for systems without KVM.

### Stress Test Results (10 parallel sandboxes)

```
cargo test --test stress_test -- --nocapture --ignored
```

| Metric | Value |
|--------|-------|
| Total time | 4.5s (10 sandboxes) |
| Success rate | 100% |
| Avg create | 44ms |
| Avg start | 174ms |
| Avg exec | 83ms |
| Avg stop | 109ms |
| Avg remove | 41ms |
| **Full lifecycle** | **446ms** |

### Benchmark Test Results (10 cycles, 5x2 iterations)

```
BENCH_SANDBOXES=5 BENCH_ITERATIONS=2 cargo test --test benchmark_test -- --nocapture --ignored
```

| Metric | Value |
|--------|-------|
| Throughput | 1.8-2.0 sandboxes/sec |
| p50 latency | 498ms |
| p95 latency | 702ms |
| p99 latency | 1028ms |

### Optimizations Applied

The Docker backend includes several optimizations:

1. **Fast container cleanup**: Uses `rm -f` instead of checking existence first
2. **Auto-remove flag**: `--rm` for automatic cleanup on stop
3. **Short stop timeout**: 1-second timeout for ephemeral containers
4. **Combined stop+remove**: Single `rm -f` kills and removes

**Before optimizations**: 6.70s total, 258ms start, 172ms stop
**After optimizations**: 4.50s total, 174ms start, 109ms stop (33% faster)

### Docker Backend Limits

The ~175ms start time is the practical floor for Docker. Remaining overhead comes from:
- Docker daemon communication (~20ms)
- Linux namespace creation (~50ms)
- cgroup setup (~30ms)
- Filesystem layering (~50ms)
- Process spawn overhead (~25ms)

## Firecracker Backend (Linux)

Firecracker microVMs provide stronger isolation (separate kernel per VM) and lower exec latency via vsock.

### Measured Performance (AMD EPYC, KVM)

| Metric | Measured | Notes |
|--------|----------|-------|
| Firecracker API ready | 46ms | Process start to socket available |
| Instance start | 78ms | VM started (hypervisor overhead only) |
| Agent ready | 1015ms | Full boot: kernel + init + agent |
| Command execution | 19ms | Via vsock (3x faster than Docker exec) |
| Shutdown | 20ms | 6x faster than Docker cleanup |

### Docker vs Firecracker Comparison (Linux)

| Metric | Docker | Firecracker | Winner |
|--------|--------|-------------|--------|
| Process start | 40ms | 46ms | Tie |
| Instance/container up | 155ms | 78ms | **Firecracker** |
| Ready to execute | 155ms | 1015ms | **Docker** (no kernel boot) |
| Command execution | 53ms | 19ms | **Firecracker** (vsock) |
| Shutdown/cleanup | 130ms | 20ms | **Firecracker** (6x faster) |

### Why Firecracker

| Aspect | Docker | Firecracker |
|--------|--------|-------------|
| Isolation | Shared kernel, namespaces | Separate kernel per VM |
| Boot path | Container runtime → namespaces → cgroups | KVM → minimal kernel → init |
| Overhead | Docker daemon, containerd, runc | Direct KVM hypercalls |
| Memory | ~50-100MB per container | ~10MB per VM |
| Security | Container escapes possible | Hardware isolation via KVM |

### Use Case Recommendations

- **Short-lived tasks (<5 commands)**: Docker is faster for total cycle time
- **Longer sessions**: Firecracker is better (lower per-command latency)
- **Security-critical code**: Firecracker required (true VM isolation)
- **Untrusted code**: Firecracker strongly recommended

### Requirements

Firecracker requires:
- Linux host (x86_64 or aarch64)
- KVM enabled (`/dev/kvm` accessible)
- Root or `kvm` group membership

```bash
# Check KVM availability
ls -la /dev/kvm
agentkernel status
```

## Running Benchmarks

### Stress Test

Tests parallel sandbox creation and execution:

```bash
# Default: 10 sandboxes
cargo test --test stress_test -- --nocapture --ignored

# Results saved to benchmark-results/stress_*.json
```

### Benchmark Test

Tests repeated sandbox lifecycle with statistics:

```bash
# Default: 10 sandboxes x 10 iterations
cargo test --test benchmark_test -- --nocapture --ignored

# Configure via environment
BENCH_SANDBOXES=20 BENCH_ITERATIONS=5 cargo test --test benchmark_test -- --nocapture --ignored

# Results saved to benchmark-results/benchmark_*.json
```

### Shell Scripts

```bash
./scripts/benchmark.sh           # Latency per operation
./scripts/stress-test.sh 100 10  # Throughput (100 cmds, 10 concurrent)
```

## Benchmark Results Directory

Results are saved to `benchmark-results/`:

```
benchmark-results/
├── stress_20260120_010517.json         # Summary stats
├── stress_20260120_010517_details.json # Per-sandbox details
├── benchmark_20260120_010228.json      # Summary stats
└── benchmark_20260120_010228_details.json
```

## Comparing Results

To compare performance across runs:

```bash
# View latest results
cat benchmark-results/stress_*.json | jq -s 'sort_by(.total_time) | .[-1]'

# Compare start times
cat benchmark-results/stress_*_details.json | jq '[.[].start_time] | add / length'
```

## Environment Notes

- **macOS**: Uses Docker or Podman (no KVM). Expect 150-250ms boot times.
- **Linux with KVM**: Uses Firecracker. Target <125ms boot times.
- **Linux without KVM**: Falls back to Docker. Similar to macOS performance.
- **CI/CD**: GitHub Actions runners don't have KVM. Use Docker backend.

## Contributing Benchmarks

If you run benchmarks on different hardware, please share:

1. Platform (OS, CPU, memory)
2. Backend (Docker/Firecracker)
3. Results JSON files
4. Any relevant configuration

Open an issue or PR with your results to help build a comprehensive benchmark database.
