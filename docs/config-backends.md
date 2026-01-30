
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

## Auto-Detection

By default, agentkernel selects the best available backend:

1. **Hyperlight** - If KVM available and `--features hyperlight` built (Linux, Wasm only)
2. **Firecracker** - If KVM is available (Linux)
3. **Apple** - If Apple Containers available (macOS 26+)
4. **Docker** - If Docker is installed
5. **Podman** - If Podman is installed

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
```

Each sandbox remembers its backend.
