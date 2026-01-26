
# Backends

agentkernel supports multiple isolation backends. Each provides different tradeoffs between security, performance, and compatibility.

## Backend Comparison

| Backend | Isolation | Boot Time | Platform | Status |
|---------|-----------|-----------|----------|--------|
| Docker | Container | ~200ms | All | Stable |
| Podman | Container | ~200ms | Linux, macOS | Stable |
| Firecracker | MicroVM | <125ms | Linux (KVM) | Stable |
| Apple | Container | ~150ms | macOS 26+ | Beta |

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

1. **Firecracker** - If KVM is available (Linux)
2. **Apple** - If Apple Containers available (macOS 26+)
3. **Docker** - If Docker is installed
4. **Podman** - If Podman is installed

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
