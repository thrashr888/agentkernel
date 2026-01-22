# RFC: Hyperlight WebAssembly Backend for Agentkernel

**Status**: Draft
**Author**: Paul Shortcut (@thrashr888)
**Date**: 2026-01-21
**Branch**: `pault/hyperlight-wasm`

## Summary

Add Hyperlight as a new sandbox backend for agentkernel, enabling sub-millisecond sandbox startup times using WebAssembly + hypervisor-based isolation.

## Motivation

Current agentkernel latencies:

| Backend | Latency | Isolation |
|---------|---------|-----------|
| Firecracker Daemon | 195ms | Full VM (KVM) |
| Docker Pool | 250-300ms | Namespaces |
| Apple Containers | 940ms | VM (Hypervisor.framework) |

**Hyperlight measured (runtime only):**
- **68ms startup** (not sub-millisecond as advertised)
- **Dual-layer security**: Wasm sandbox + hypervisor boundary
- **~3x faster** than Firecracker Daemon (195ms)

Note: Microsoft's sub-millisecond claim appears to be for function calls after
runtime is loaded, not for sandbox startup.

Azure team at HashiCorp QBR specifically recommended this for our sandbox use case.

## Technical Overview

### What is Hyperlight?

Hyperlight is a Microsoft open-source Rust library that creates micro virtual machines without traditional OS overhead. Instead of booting a kernel, it provides "a linear slice of memory and a CPU" to run WebAssembly modules.

```
┌─────────────────────────────────────────┐
│           Host Application              │
│              (agentkernel)              │
├─────────────────────────────────────────┤
│          Hyperlight VMM                 │
│   (creates memory slice, loads guest)   │
├─────────────────────────────────────────┤
│       Hyperlight Wasm Guest             │
│     (wasmtime running Wasm module)      │
├─────────────────────────────────────────┤
│         Hypervisor (KVM/WHP)            │
└─────────────────────────────────────────┘
```

### Security Model

Two layers of isolation:
1. **WebAssembly sandbox** - Software-defined memory isolation
2. **Hypervisor boundary** - Hardware-enforced VM isolation

This is stronger than Docker (shared kernel) and comparable to Firecracker (separate kernel per VM).

### Platform Support

| Platform | Hypervisor | Status |
|----------|------------|--------|
| Linux | KVM | ✅ Supported |
| Windows | WHP | ✅ Supported |
| WSL2 | KVM | ✅ Supported |
| Azure Linux | mshv | ✅ Supported |
| **macOS** | - | ❌ **Not supported** |

**Critical gap**: macOS is not supported. This means Hyperlight would be Linux/Windows-only.

## Integration Design

### Backend Priority (Updated)

```rust
pub enum Backend {
    Hyperlight,      // NEW: Sub-millisecond, Wasm-based (Linux/Windows)
    Firecracker,     // ~195ms daemon, ~800ms cold (Linux KVM)
    Apple,           // ~940ms (macOS 26+)
    Container(ContainerRuntime), // ~250-500ms (fallback)
}
```

Selection logic:
```rust
fn detect_backend() -> Backend {
    if hyperlight_available() {      // KVM + hyperlight crate
        Backend::Hyperlight
    } else if kvm_available() {
        Backend::Firecracker
    } else if apple_containers_available() && macos_26_plus() {
        Backend::Apple
    } else if docker_available() {
        Backend::Container(Docker)
    } else {
        bail!("No backend available")
    }
}
```

### Execution Model Differences

**Current model** (shell commands):
```bash
agentkernel run -- python script.py
agentkernel run -- node index.js
```

**Hyperlight model** (Wasm modules):
```bash
agentkernel run --wasm module.wasm
agentkernel run --wasm-component component.wasm
```

**Challenge**: AI agents expect shell environments. Options:

1. **WASI CLI** - Use WASI for filesystem/process access (limited)
2. **Hybrid approach** - Hyperlight for code execution, traditional backend for shell
3. **Wasm runtimes** - Use Python/Node Wasm builds (e.g., pyodide, quickjs)

### Proposed API

```rust
// src/hyperlight_backend.rs

pub struct HyperlightSandbox {
    sandbox: MultiUseSandbox,
    name: String,
}

impl HyperlightSandbox {
    pub fn new(name: &str) -> Result<Self>;

    /// Run a Wasm module directly
    pub async fn run_wasm(&self, module: &[u8], args: &[String]) -> Result<String>;

    /// Run code in an interpreter (Python, JS via Wasm)
    pub async fn run_code(&self, language: &str, code: &str) -> Result<String>;

    /// Execute a function exported from a Wasm component
    pub async fn call_function<T>(&self, name: &str, args: &[Value]) -> Result<T>;
}
```

### Use Cases

**Best fit for Hyperlight:**
- Code evaluation/execution (Python, JS snippets)
- Untrusted code sandboxing
- High-frequency, short-lived tasks
- Security-critical operations

**Still need traditional backends:**
- Full shell access
- Complex filesystem operations
- Long-running processes
- macOS users

## Implementation Plan

### Phase 1: Foundation (This PR)
- [ ] Add hyperlight-wasm dependency (feature-gated)
- [ ] Create `hyperlight_backend.rs` scaffold
- [ ] Implement basic Wasm module execution
- [ ] Add `--wasm` flag to CLI

### Phase 2: Language Support
- [ ] Integrate quickjs-wasm for JavaScript
- [ ] Integrate rustpython-wasm for Python
- [ ] Implement WASI filesystem access

### Phase 3: AI Agent Integration
- [ ] Design Wasm-based code execution API
- [ ] MCP tool for code evaluation
- [ ] Benchmark against current backends

## Benchmark Results (Measured)

Tested on AMD EPYC (rookery), KVM, hyperlight-wasm 0.12.0:

| Metric | Measured | Notes |
|--------|----------|-------|
| Runtime startup | **68ms** (avg), 67ms (p50) | SandboxBuilder + load_runtime() |
| Min/Max | 67ms / 77ms | |
| p95/p99 | 77ms | |

### Comparison

| Backend | Startup | Speedup |
|---------|---------|---------|
| **Hyperlight** | **68ms** | baseline |
| Firecracker Daemon | 195ms | 2.9x slower |
| Docker Pool | 250ms | 3.7x slower |
| Apple Containers | 940ms | 13.8x slower |

### Analysis

The measured 68ms is **not** the sub-millisecond advertised by Microsoft. The sub-millisecond
claim likely refers to:
1. **Function calls** after runtime is loaded (not startup)
2. **AOT-compiled modules** with mmap (vs JIT compilation overhead)

However, 68ms is still valuable:
- Faster than all current agentkernel backends
- Once loaded, function calls should be sub-millisecond
- Good fit for code evaluation with warm sandbox reuse

## Risks and Mitigations

### 1. macOS Not Supported
**Risk**: macOS users can't use Hyperlight
**Mitigation**: Keep Apple Containers as macOS backend; Hyperlight is Linux/Windows optimization

### 2. Experimental Status
**Risk**: API changes, stability issues
**Mitigation**: Feature-gate behind `--features hyperlight`; don't make it default yet

### 3. WASI Limitations
**Risk**: Limited filesystem/network access vs full shell
**Mitigation**: Hybrid approach - use Hyperlight for code exec, traditional for shell

### 4. Wasm Compilation Required
**Risk**: Users need to compile code to Wasm
**Mitigation**: Bundle pre-compiled interpreters (Python, JS, shell)

## Open Questions

1. **Should Hyperlight be the default on Linux?** Or opt-in via flag?
2. **How do AI agents interact with Wasm sandboxes?** MCP tools? Custom protocol?
3. **What WASI capabilities do we expose?** Filesystem? Network? Env vars?
4. **Can we support shell commands via Wasm shell implementation?**

## References

- [Hyperlight announcement](https://opensource.microsoft.com/blog/2025/03/26/hyperlight-wasm-fast-secure-and-os-free)
- [hyperlight-dev/hyperlight](https://github.com/hyperlight-dev/hyperlight)
- [hyperlight-dev/hyperlight-wasm](https://github.com/hyperlight-dev/hyperlight-wasm)
- [WASI specification](https://wasi.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)

## Appendix: Benchmark Comparison

```
Measured Results (agentkernel, hyperlight-wasm 0.12.0):
┌────────────────────┬──────────┬──────────────────┐
│ Backend            │ Latency  │ Notes            │
├────────────────────┼──────────┼──────────────────┤
│ Hyperlight         │ 68ms     │ Runtime startup  │
│ FC Daemon          │ 195ms    │ Pre-warmed VM    │
│ Docker Pool        │ 250ms    │ Container pool   │
│ Apple Containers   │ 940ms    │ macOS 26+        │
│ FC Ephemeral       │ 800ms    │ Cold start       │
└────────────────────┴──────────┴──────────────────┘

Speedups vs Hyperlight:
- vs Firecracker Daemon: 2.9x faster
- vs Docker Pool: 3.7x faster
- vs Apple Containers: 13.8x faster
```
