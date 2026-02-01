
# Backends

agentkernel supports multiple isolation backends. Each provides different tradeoffs between security, performance, and compatibility.

## Backend Comparison

| Backend | Isolation | Boot Time | Platform | Status |
|---------|-----------|-----------|----------|--------|
| Docker | Container | ~220ms | All | Stable |
| Podman | Container | ~300ms | Linux, macOS | Stable |
| Firecracker | MicroVM | <125ms | Linux (KVM) | Stable |
| Hyperlight | Wasm + Hypervisor | ~68ms | Linux (KVM) | Experimental |
| Apple | Container | ~940ms | macOS 26+ | Beta |
| Kubernetes | Pod | ~2-5s | Any K8s cluster | Stable |
| Nomad | Job allocation | ~2-5s | Any Nomad cluster | Stable |

## Docker

The most widely compatible backend. Uses Docker Desktop on macOS or Docker Engine on Linux.

```bash
# Force Docker backend
agentkernel create my-sandbox --backend docker
```

**Pros:**
- Works everywhere Docker runs
- Familiar to most developers
- Large ecosystem of images

**Cons:**
- Shared kernel (container escape possible)
- Slower than Firecracker

## Podman

Drop-in Docker replacement that runs rootless by default.

```bash
agentkernel create my-sandbox --backend podman
```

**Pros:**
- Rootless by default (better security)
- Docker-compatible
- No daemon required

**Cons:**
- Shared kernel
- Slightly less mature than Docker

## Firecracker

Amazon's microVM technology. Provides true hardware isolation with minimal overhead.

```bash
agentkernel create my-sandbox --backend firecracker
```

**Requirements:**
- Linux with KVM support (`/dev/kvm`)
- x86_64 architecture

**Pros:**
- Dedicated kernel per sandbox
- Hardware-enforced isolation
- Sub-125ms boot times
- Minimal memory overhead (~10MB)

**Cons:**
- Linux only
- Requires KVM

## Hyperlight (Wasm)

Microsoft's Hyperlight runs WebAssembly modules inside hypervisor-isolated micro VMs. Dual-layer security: Wasm sandbox + hardware boundary.

```bash
# Build with Hyperlight support
cargo build --features hyperlight

# Run a Wasm module
agentkernel run --backend hyperlight module.wasm
```

**Requirements:**
- Linux with KVM support (`/dev/kvm`)
- Build with `--features hyperlight`
- AOT-compiled Wasm modules for best performance

**Pros:**
- Dual-layer isolation (Wasm + hypervisor)
- ~68ms cold start, sub-microsecond with pre-warmed pool
- Smallest attack surface

**Cons:**
- Runs WebAssembly modules only (not arbitrary shell commands)
- Linux only, requires KVM
- Experimental

## Apple Containers

Native container support on macOS Tahoe (26+).

```bash
agentkernel create my-sandbox --backend apple
```

**Pros:**
- Native macOS integration
- Good performance
- No Docker Desktop required

**Cons:**
- macOS 26+ only
- Beta status

## Kubernetes

Run sandboxes as Kubernetes Pods on any cluster. Requires building with `--features kubernetes`.

```bash
cargo build --features kubernetes

agentkernel create my-sandbox --backend kubernetes --image alpine:3.20
```

**Requirements:**
- Kubernetes cluster access (kubeconfig)
- Build with `--features kubernetes`

**Pros:**
- Scales to thousands of sandboxes
- Warm pool for fast acquisition (~100ms vs ~2-5s cold start)
- NetworkPolicy-based network isolation
- Optional RuntimeClass for gVisor/Kata isolation
- Kubernetes-native CRDs (AgentSandbox, AgentSandboxPool)

**Cons:**
- Requires cluster infrastructure
- Higher latency than local backends
- Never auto-detected (must specify `--backend kubernetes`)

See the [Orchestration Guide](orchestration.md) for full configuration and deployment details.

## Nomad

Run sandboxes as HashiCorp Nomad job allocations. Requires building with `--features nomad`.

```bash
cargo build --features nomad

agentkernel create my-sandbox --backend nomad --image alpine:3.20
```

**Requirements:**
- Nomad cluster access (`nomad` CLI or `NOMAD_ADDR`)
- Build with `--features nomad`

**Pros:**
- Simpler cluster setup than Kubernetes
- Multiple task drivers (Docker, exec, raw_exec)
- Warm pool via parameterized batch jobs
- Integrates with Consul and Vault

**Cons:**
- Requires Nomad infrastructure
- Higher latency than local backends
- Never auto-detected (must specify `--backend nomad`)

See the [Orchestration Guide](orchestration.md) for full configuration and deployment details.

## Auto-Detection

By default, agentkernel selects the best available *local* backend:

1. **Hyperlight** - If KVM available and `--features hyperlight` built (Linux, Wasm only)
2. **Firecracker** - If KVM is available (Linux)
3. **Apple** - If Apple Containers available (macOS 26+)
4. **Docker** - If Docker is installed
5. **Podman** - If Podman is installed

Kubernetes and Nomad are never auto-detected. They must be specified explicitly with `--backend kubernetes` or `--backend nomad`.

```bash
# Check which backend is selected
agentkernel setup --check
```

## Backend Persistence

When you create a sandbox, the backend is saved with it. Subsequent operations automatically use the same backend:

```bash
# Create with Docker
agentkernel create my-sandbox --backend docker

# These automatically use Docker (no --backend needed)
agentkernel start my-sandbox
agentkernel exec my-sandbox -- echo hello
agentkernel list  # Shows BACKEND column
```

## Mixing Backends

You can have sandboxes using different backends:

```
$ agentkernel list
NAME          STATUS     BACKEND
project-a     running    docker
project-b     stopped    podman
test-vm       running    firecracker
k8s-sandbox   running    kubernetes
nomad-job     running    nomad
```

Each sandbox remembers its backend.
