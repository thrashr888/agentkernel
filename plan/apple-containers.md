# Apple Containers Backend

## Overview

Add Apple's `container` CLI as a native backend for macOS 26+. This provides:
- **True VM isolation** (one VM per container, like Firecracker)
- **Native macOS performance** (no Docker Desktop overhead)
- **OCI compatible** (same images work)

## Why This Matters

| Aspect | Docker Desktop | Apple Containers |
|--------|----------------|------------------|
| Architecture | Multiple containers in one VM | One VM per container |
| Isolation | Container namespace | Hardware VM isolation |
| Overhead | Docker daemon + VM | Native hypervisor |
| Boot time | ~150-300ms | TBD (potentially faster) |

## System Requirements

- macOS 26+ (shipped June 2025)
- Apple Silicon (M1/M2/M3/M4)
- `container` CLI installed

## Implementation Plan

### Phase 1: Backend Detection
**Files:** `src/setup.rs`, `src/vmm.rs`

1. Check for `container` CLI availability: `which container`
2. Verify macOS 26+: `sw_vers -productVersion`
3. Add to backend priority: Apple Containers > Docker > Podman (on macOS)

### Phase 2: Container Backend
**Files:** `src/apple_backend.rs` (new)

Implement the same interface as `docker_backend.rs`:

```rust
pub struct AppleContainer {
    name: String,
    image: String,
    // ...
}

impl AppleContainer {
    pub fn create(&self) -> Result<()>
    pub fn start(&self) -> Result<()>
    pub fn run_command(&self, cmd: &[String]) -> Result<String>
    pub fn stop(&self) -> Result<()>
    pub fn remove(&self) -> Result<()>
}
```

### Phase 3: CLI Mapping

| Docker Command | Apple Container Command |
|----------------|------------------------|
| `docker create` | `container create` |
| `docker start` | `container start` |
| `docker run` | `container run` |
| `docker stop` | `container stop` |
| `docker rm` | `container delete` |
| `docker ps` | `container ls` |
| `docker pull` | `container images pull` |

### Phase 4: Feature Parity

Ensure these features work:
- [ ] Volume mounts: `-v $(pwd):/app`
- [ ] Environment variables: `-e VAR=value`
- [ ] Port publishing: `-p 8080:80`
- [ ] Resource limits: `--cpus 2 --memory 1G`
- [ ] Interactive mode: `-it`
- [ ] Auto-remove: `--rm`

### Phase 5: Pool Support

Add container pool similar to Docker pool:
- Pre-create containers
- Reuse for fast execution
- Background cleanup

## Testing Strategy

1. Unit tests for command generation
2. Integration tests on macOS 26
3. Performance benchmarks vs Docker

## Performance Hypothesis

Since Apple containers use native virtualization:
- Boot time: Potentially 50-100ms (vs Docker's 150-300ms)
- Run latency: Lower (no Docker daemon overhead)
- Memory: Lower (no Docker VM overhead)

## Open Questions

1. Does `container run` support the same options as Docker?
2. Can we keep containers warm (stopped but ready)?
3. Is there a daemon mode or is each invocation standalone?
4. Network isolation options?

## References

- [Apple Container GitHub](https://github.com/apple/container)
- [Command Reference](https://github.com/apple/container/blob/main/docs/command-reference.md)
- [WWDC25 Session](https://developer.apple.com/videos/play/wwdc2025/346/)
- [Getting Started Guide](https://swapnasagarpradhan.medium.com/getting-started-with-apples-container-cli-on-macos-a-native-alternative-to-docker-fc303e08f5cd)
