
# Backends

agentkernel supports multiple isolation backends. Each provides different tradeoffs between security, performance, and compatibility.

## Backend Comparison

| Backend | Isolation | Requirements / support |
|---------|-----------|------------------------|
| Docker / Podman | Containers sharing the runtime host's kernel | A working container runtime |
| Firecracker | MicroVM with a dedicated guest kernel | Linux/KVM; full-state operations have an additional x86_64 compatibility gate |
| Hyperlight | Wasm inside a hypervisor boundary | Experimental; Linux x86_64/KVM and the `hyperlight` Cargo feature |
| Apple | VM-backed Linux containers | Apple Silicon, macOS 26+, installed Apple `container` CLI |
| Kubernetes | Pod; stronger isolation depends on RuntimeClass | Cluster access and the `kubernetes` feature (enabled by default) |
| Nomad | Depends on the configured task driver | Cluster access and the `nomad` feature (enabled by default) |
| Daytona / Runloop / E2B / Modal | Provider-specific | Experimental hosted adapters; bridge dependencies and credentials |
| Agent Computer | Custom bridge contract | No bundled live adapter |

Use [benchmarks](../getting-started/benchmarks.md) for measurement scope and
[backend compatibility](../operations/backend-compatibility.md) for validation
gates. Backend availability does not mean every lifecycle operation is supported.

## Docker

The most widely compatible backend. Uses Docker Desktop on macOS or Docker Engine on Linux.

```bash
# Force Docker backend
agentkernel sandbox create my-sandbox --backend docker
```

**Pros:**
- Works everywhere Docker runs
- Familiar to most developers
- Large ecosystem of images

**Cons:**
- Shared kernel (container escape possible)
- Shares a kernel with other containers on the same runtime host

## Podman

Drop-in Docker replacement that runs rootless by default.

```bash
agentkernel sandbox create my-sandbox --backend podman
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
agentkernel sandbox create my-sandbox --backend firecracker
```

**Requirements:**
- Linux with KVM support (`/dev/kvm`)
- x86_64 architecture

**Pros:**
- Dedicated kernel per sandbox
- Hardware-enforced isolation
- Separate guest kernel per sandbox; guest memory allocation is additional to the VMM process

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
- Pool acquisition and Wasm execution can be measured separately

**Cons:**
- Runs WebAssembly modules only (not arbitrary shell commands)
- Linux x86_64 with KVM; see the [arm64 compatibility exception](../operations/dependency-compatibility.md#hyperlight-arm64-exception)
- Experimental

## Apple Containers

VM-backed Linux containers using the separately installed Apple `container` CLI on Apple Silicon with macOS 26+.

```bash
agentkernel sandbox create my-sandbox --backend apple
```

**Pros:**
- Native macOS integration
- No Docker Desktop required

**Cons:**
- macOS 26+ only
- Runtime compatibility must match the [tested Apple CLI](../operations/backend-compatibility.md)

## Kubernetes

Run sandboxes as Kubernetes Pods on any cluster. Uses the `kubernetes` feature, enabled in default builds.

```bash
cargo build --features kubernetes

agentkernel sandbox create my-sandbox --backend kubernetes --image alpine:3.24
```

**Requirements:**
- Kubernetes cluster access (kubeconfig)
- Build with `--features kubernetes`

**Pros:**
- Cluster scheduling and optional warm pools
- NetworkPolicy-based network isolation
- Optional RuntimeClass for gVisor/Kata isolation
- Kubernetes-native CRDs (AgentSandbox, AgentSandboxPool)

**Cons:**
- Requires cluster infrastructure
- Higher latency than local backends
- Never auto-detected (must specify `--backend kubernetes`)

See the [Orchestration Guide](../operations/index.md) for full configuration and deployment details.

## Nomad

Run sandboxes as HashiCorp Nomad job allocations. Uses the `nomad` feature, enabled in default builds.

```bash
cargo build --features nomad

agentkernel sandbox create my-sandbox --backend nomad --image alpine:3.24
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

See the [Orchestration Guide](../operations/index.md) for full configuration and deployment details.

## Remote Backends

`daytona`, `runloop`, `e2b`, `modal`, and `agentcomputer` use the shared remote sandbox substrate. They keep the same CLI and HTTP verbs as local backends, but route sandbox lifecycle, workspace sync, and service publishing through the remote bridge.

```bash
agentkernel sandbox create my-sandbox --backend daytona
agentkernel sandbox create my-sandbox --backend runloop
agentkernel sandbox create my-sandbox --backend e2b
agentkernel sandbox create my-sandbox --backend modal
```

**Common behavior:**
- `mount_cwd` syncs the local project into `/workspace`
- declared `ports` resolve to provider `endpoints`
- `attach` uses the shared remote sandbox session path
- persisted sandboxes reconnect by provider `remote_id`

**Requirements:**
- Node.js 20+ available on the host
- `scripts/remote-bridge.mjs` present, or `AGENTKERNEL_REMOTE_BRIDGE` set
- provider bridge dependencies installed with `npm install --prefix scripts`
- provider credentials exported in the environment

**Current provider support:**
- `daytona` is wired in the bundled bridge via `@daytonaio/sdk`
- `runloop` is wired in the bundled bridge via `@runloop/api-client`
- `e2b` is wired in the bundled bridge via the official `e2b` SDK
- `modal` is wired in the bundled bridge via the official `modal` SDK
- all shipped adapters support live lifecycle, `exec`, `attach`, file operations, managed `mount_cwd` sync, resolved endpoints, and workspace-level snapshot/restore
- credentials can come from exported provider env vars or from `[remote.daytona]` / `[remote.runloop]` / `[remote.e2b]` / `[remote.modal]` in `agentkernel.toml`
- explicit `-c path/to/agentkernel.toml` is persisted with the sandbox so later `start`, `exec`, and snapshot flows can reconnect with the same remote config
- set `[remote].bridge` when you run `agentkernel` outside the repository root and still want to use the bundled `scripts/remote-bridge.mjs`
- the bundled bridge still supports `AGENTKERNEL_REMOTE_BRIDGE_MODE=mock` for local testing
- `agentcomputer` still needs a provider-specific live adapter
- see the [Remote Backends Guide](../operations/remote.md) for setup and runnable examples

## Auto-Detection

For ordinary sandbox creation, AgentKernel prefers local backends in this order:

1. **Firecracker** on Linux when KVM and Firecracker are available.
2. **Apple Containers** on supported macOS hosts when the CLI is available.
3. **Podman** when available.
4. **Docker** when available.

If no local runtime is usable, a configured hosted backend may be selected:
Daytona, Runloop, E2B, Modal, then Agent Computer, in that order. The last option
requires a custom live bridge. Hyperlight, Kubernetes, and Nomad are not selected
by this detector; select them explicitly.

Use `--backend` when execution location matters. `run --fast` instead selects a
container-pool path; HTTP `/run` and MCP `sandbox_run` also default to that pool.
Their defaults differ from ordinary CLI execution.

```bash
# Inspect available runtimes and host prerequisites
agentkernel doctor

# Require a local Docker backend
agentkernel run --backend docker -- echo hello
```

## Backend Persistence

When you create a sandbox, the backend is saved with it. Subsequent operations automatically use the same backend:

```bash
# Create with Docker
agentkernel sandbox create my-sandbox --backend docker

# These automatically use Docker (no --backend needed)
agentkernel sandbox start my-sandbox
agentkernel exec my-sandbox -- echo hello
agentkernel sandbox list  # Shows BACKEND column
```

## Mixing Backends

You can have sandboxes using different backends:

```
$ agentkernel sandbox list
NAME          STATUS     BACKEND
project-a     running    docker
project-b     stopped    podman
test-vm       running    firecracker
k8s-sandbox   running    kubernetes
nomad-job     running    nomad
```

Each sandbox remembers its backend.
