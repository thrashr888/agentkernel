# Agentkernel Benchmarks

Performance measurements for sandbox lifecycle operations.

## Quick Summary

### End-to-End Command Latency (`agentkernel run -- echo hello`)

This is what users experience - total time from command start to output:

| Mode | Platform | Latency | Throughput | Notes |
|------|----------|---------|------------|-------|
| Firecracker Daemon | Linux (AMD EPYC) | **195ms** | **~5.1/sec** | Pre-warmed VM pool (3-5 VMs) |
| Docker Pool | Linux (AMD EPYC) | ~250ms | ~4.0/sec | Container pool with `-F` flag |
| Docker Pool | macOS (M3 Pro) | ~300ms | ~3.3/sec | Container pool with `-F` flag |
| Docker Ephemeral | Linux (AMD EPYC) | ~450ms | ~2.2/sec | Uses optimized `run --rm` path |
| Docker Ephemeral | macOS (M3 Pro) | ~500ms | ~2.0/sec | Uses optimized `run --rm` path |
| Firecracker Ephemeral | Linux (AMD EPYC) | **800ms** | ~1.3/sec | Full VM lifecycle (cold start) |
| Apple Containers | macOS 26 (M3 Pro) | ~940ms | ~1.1/sec | Optimized single-operation path |
| Apple Containers (--keep) | macOS 26 (M3 Pro) | ~2200ms | ~0.5/sec | Multi-step lifecycle |

**Key insight**: Daemon mode with pre-warmed VMs provides the best latency (195ms) with full VM isolation. For ephemeral usage, Docker and Apple Containers now use optimized single-operation paths (`run --rm`) by default. Apple Containers provide true VM isolation on macOS but with higher latency than Docker containers.

### Component Breakdown

| Backend | Platform | Boot | Ready | Exec | Shutdown | Throughput |
|---------|----------|------|-------|------|----------|------------|
| Docker | macOS (M3 Pro) | 188ms | 188ms | 83ms | 109ms | 2.0/sec |
| Docker | Linux (AMD EPYC) | 155ms | 155ms | 53ms | 130ms | ~4/sec |
| Firecracker | Linux (AMD EPYC) | 78ms | 110ms | 19ms | 20ms | ~9/sec |
| **FC Daemon** | Linux (AMD EPYC) | 0ms | 0ms | 19ms | 0ms | **~5/sec** |
| Apple Containers | macOS 26 (M3 Pro) | 860ms | 860ms | 95ms | 37ms | ~1/sec |

The daemon mode eliminates boot/ready/shutdown overhead by reusing pre-warmed VMs.

## Docker Backend (macOS)

Tested on Apple Silicon (M3 Pro) with Docker via Colima. This is the fallback backend for systems without KVM.

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

| Metric | Baseline | Optimized | Notes |
|--------|----------|-----------|-------|
| Firecracker API ready | 46ms | 46ms | Process start to socket available |
| Instance start | 78ms | 78ms | VM started (hypervisor overhead only) |
| Agent ready | 961ms | **110ms** | 89% faster with boot arg optimizations |
| Command execution | 19ms | 19ms | Via vsock (3x faster than Docker exec) |
| Shutdown | 20ms | 20ms | 6x faster than Docker cleanup |

### Boot Optimization Details

The 89% speedup came from kernel boot args that disable unnecessary PS/2 keyboard drivers:

```
quiet loglevel=4 i8042.nokbd i8042.noaux
```

| Optimization | Time Saved | Notes |
|--------------|------------|-------|
| i8042.nokbd | ~500ms | Disable PS/2 keyboard driver |
| i8042.noaux | ~260ms | Skip PS/2 aux port probe |
| quiet loglevel=4 | ~90ms | Reduce console output |

### Docker vs Firecracker Comparison (Linux)

| Metric | Docker | Firecracker | Winner |
|--------|--------|-------------|--------|
| Process start | 40ms | 46ms | Tie |
| Instance/container up | 155ms | 78ms | **Firecracker** |
| Ready to execute | 155ms | **110ms** | **Firecracker** (optimized) |
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

| Use Case | Recommended Mode | Why |
|----------|------------------|-----|
| Interactive/API use | Daemon mode | 195ms latency, VM isolation |
| Batch processing | Firecracker ephemeral | Clean VM per job |
| macOS development (speed) | Docker pool (`-F`) | Fastest on macOS |
| macOS development (security) | Apple Containers | True VM isolation |
| Security-critical | Daemon, Firecracker, or Apple | True VM isolation |
| CI/CD | Docker ephemeral | No KVM in most runners |

## Apple Containers Backend (macOS 26+)

Apple Containers use the native macOS hypervisor to run lightweight VMs (one VM per container), providing Firecracker-like isolation on Apple Silicon without requiring KVM.

### Measured Performance (M3 Pro, macOS 26.3)

| Metric | Time | Notes |
|--------|------|-------|
| Create container | ~190ms | Container definition created |
| Start (VM boot) | **~860ms** | Main overhead - VM boot time |
| Exec (in running) | ~95ms | Fast once VM is running |
| Stop | ~37ms | Quick shutdown |
| Remove | ~100ms | Cleanup |
| **Full `run --rm`** | **~940ms** | Single operation (optimal) |
| **Via agentkernel** | **~940ms** | Uses optimized single-operation path |
| **Via agentkernel (--keep)** | ~2200ms | Multi-step: create+start+exec+stop |

### Optimized Execution Path

As of v0.1.2, `agentkernel run` automatically uses the optimized single-operation path for ephemeral runs:

```bash
# Uses single `container run --rm` internally (~940ms)
agentkernel run -- echo hello

# Multi-step path only used when --keep is specified (~2200ms)
agentkernel run --keep -- echo hello
```

This reduces Apple Containers latency from ~2200ms to ~940ms (57% faster).

### Comparison: Apple Containers vs Docker (macOS)

| Metric | Docker | Apple Containers | Winner |
|--------|--------|------------------|--------|
| Isolation | Shared kernel (namespaces) | Separate VM per container | **Apple** (stronger) |
| Boot time | ~175ms | ~778ms | **Docker** (4x faster) |
| Full lifecycle | ~500ms | ~1000ms | **Docker** (2x faster) |
| Memory overhead | ~50MB | ~100MB+ | **Docker** |
| Security | Container escapes possible | Hardware isolation | **Apple** (more secure) |

### When to Use Apple Containers

**Use Apple Containers when:**
- Strong isolation is required (untrusted code)
- Running on macOS 26+ without Docker
- Security is more important than speed

**Use Docker when:**
- Speed is the priority
- Running trusted code
- Memory is constrained

### Requirements

- macOS 26.0 or later (Tahoe)
- Apple Silicon (arm64)
- `container` CLI from https://github.com/apple/container/releases

```bash
# Install Apple Containers CLI
# Download from: https://github.com/apple/container/releases
sudo installer -pkg container-installer-signed.pkg -target /

# Setup and verify (auto-starts Apple container system)
agentkernel setup
agentkernel status
```

Note: `agentkernel setup` automatically starts the Apple container system service and pre-pulls the Alpine base image. The service start is also triggered automatically on first `agentkernel run` when using the Apple backend.

## Daemon Mode (Linux)

The daemon maintains a pool of pre-warmed Firecracker VMs for fast execution.

### How It Works

```
┌─────────────────────────────────────────┐
│           agentkernel daemon            │
│                                         │
│  ┌────┐ ┌────┐ ┌────┐    ┌────┐       │
│  │ VM │ │ VM │ │ VM │    │ VM │       │
│  │warm│ │warm│ │warm│    │use │       │
│  └────┘ └────┘ └────┘    └────┘       │
│       Warm Pool           In Use       │
└─────────────────────────────────────────┘
```

- Pool maintains 3-5 pre-booted VMs
- `run` command acquires VM from pool (~0ms)
- Executes command via vsock (~19ms)
- Returns VM to pool for reuse (~0ms)

### Measured Performance (AMD EPYC, KVM)

```bash
# Start daemon (pre-warms 3 VMs)
agentkernel daemon start

# Run commands (uses warm pool)
time agentkernel run -- echo hello
```

| Metric | Value | Notes |
|--------|-------|-------|
| Daemon startup | ~3s | Boots 3 warm VMs |
| Command latency | **195ms** | Acquire + exec + release |
| Concurrent requests | ✓ | Tested 3 parallel jobs |
| VM reuse | ~95% | Only cold start when pool exhausted |

### Comparison: Daemon vs Ephemeral

| Metric | Ephemeral | Daemon | Speedup |
|--------|-----------|--------|---------|
| First command | 800ms | 195ms | **4.1x** |
| Subsequent commands | 800ms | 195ms | **4.1x** |
| 10 sequential commands | 8.0s | 1.95s | **4.1x** |

### When to Use Daemon Mode

**Use daemon when:**
- Running many commands (API server, interactive use)
- Low latency matters more than memory
- You have a long-running service

**Use ephemeral when:**
- Running occasional one-off commands
- Memory is constrained
- You want clean VM per execution

### Daemon Commands

```bash
agentkernel daemon start   # Start daemon, pre-warm pool
agentkernel daemon status  # Show pool stats
agentkernel daemon stop    # Graceful shutdown
```

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
# Default: 10 sandboxes, 50 concurrent
cargo test --test stress_test -- --nocapture --ignored

# Large run: 1000 sandboxes, 100 concurrent
STRESS_VM_COUNT=1000 STRESS_MAX_CONCURRENT=100 cargo test --test stress_test -- --nocapture --ignored

# Results saved to benchmark-results/stress_*.json
```

Environment variables:
- `STRESS_VM_COUNT` - Number of sandboxes (default: 10)
- `STRESS_MAX_CONCURRENT` - Max concurrent operations (default: 50)

### Benchmark Test

Tests repeated sandbox lifecycle with statistics:

```bash
# Default: 10 sandboxes x 10 iterations, 50 concurrent
cargo test --test benchmark_test -- --nocapture --ignored

# Configure via environment
BENCH_SANDBOXES=20 BENCH_ITERATIONS=5 BENCH_MAX_CONCURRENT=100 cargo test --test benchmark_test -- --nocapture --ignored

# Results saved to benchmark-results/benchmark_*.json
```

Environment variables:
- `BENCH_SANDBOXES` - Sandboxes per iteration (default: 10)
- `BENCH_ITERATIONS` - Number of iterations (default: 10)
- `BENCH_MAX_CONCURRENT` - Max concurrent operations (default: 50)

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

- **macOS 26+**: Auto-selects Apple Containers for VM isolation (~1s boot). Falls back to Docker if not available.
- **macOS <26**: Uses Docker or Podman (no KVM). Expect 150-250ms boot times.
- **Linux with KVM**: Uses Firecracker. Achieves **110ms boot times** (beat <125ms target).
- **Linux without KVM**: Falls back to Docker. Similar to macOS performance.
- **CI/CD**: GitHub Actions runners don't have KVM. Use Docker backend.

## Contributing Benchmarks

If you run benchmarks on different hardware, please share:

1. Platform (OS, CPU, memory)
2. Backend (Docker/Firecracker)
3. Results JSON files
4. Any relevant configuration

Open an issue or PR with your results to help build a comprehensive benchmark database.
