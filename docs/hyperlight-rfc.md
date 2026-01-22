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

**Hyperlight promises:**
- **1-2ms startup** (targeting sub-millisecond)
- **Dual-layer security**: Wasm sandbox + hypervisor boundary
- **~10x faster** than our current best case

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

## Performance Targets

| Metric | Firecracker Daemon | Hyperlight Target |
|--------|-------------------|-------------------|
| Cold start | 800ms | **<5ms** |
| Warm start | 195ms | **<2ms** |
| Exec latency | 19ms | **<1ms** |
| Memory per sandbox | ~10MB | **<1MB** |

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
Current State (agentkernel run -- echo hello):
┌────────────────────┬──────────┬─────────────┐
│ Backend            │ Latency  │ Throughput  │
├────────────────────┼──────────┼─────────────┤
│ FC Daemon          │ 195ms    │ 5.1/sec     │
│ Docker Pool        │ 250ms    │ 4.0/sec     │
│ Apple Containers   │ 940ms    │ 1.1/sec     │
│ FC Ephemeral       │ 800ms    │ 1.3/sec     │
└────────────────────┴──────────┴─────────────┘

With Hyperlight (projected):
┌────────────────────┬──────────┬─────────────┐
│ Backend            │ Latency  │ Throughput  │
├────────────────────┼──────────┼─────────────┤
│ Hyperlight         │ <2ms     │ >500/sec    │  ← 100x improvement!
│ FC Daemon          │ 195ms    │ 5.1/sec     │
│ Docker Pool        │ 250ms    │ 4.0/sec     │
└────────────────────┴──────────┴─────────────┘
```
