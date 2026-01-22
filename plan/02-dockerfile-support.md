# Plan: Dockerfile Support for agentkernel

## Problem Statement

Users want to create custom sandbox environments beyond predefined runtimes. Currently, agentkernel only supports:
1. Pre-built runtime shorthand (python, node, go, rust, etc.)
2. Existing Docker Hub images via `base_image`
3. Pre-built Firecracker rootfs images

This limits users who need custom dependencies, specific tool versions, or reproducible environments.

## Current State

**Image Specification Methods:**
1. `runtime` shorthand in `agentkernel.toml` - maps to predefined Docker images
2. `base_image` field - explicit Docker image reference
3. CLI `--image` flag - override at runtime
4. Auto-detection via `languages.rs` from project files

**Limitations:**
- No support for custom Dockerfiles
- Firecracker limited to pre-built runtimes
- No build caching or layer optimization
- No registry integration

## Design Options

### Option A: Local Docker Build (Recommended)
- Build Dockerfiles locally using `docker build`
- Cache built images in local Docker registry
- Simple, familiar, works with existing tooling

### Option B: BuildKit Integration
- Use BuildKit for improved caching and parallel builds
- More efficient for complex multi-stage builds

### Option C: OCI Image Building
- Build OCI images directly (no Docker required)
- Future-proof for non-Docker environments

## Configuration Schema

```toml
[sandbox]
name = "my-custom-app"

[build]
dockerfile = "./Dockerfile"          # Path to Dockerfile
context = "."                        # Build context
target = "runtime"                   # Multi-stage target (optional)
args = { PYTHON_VERSION = "3.12" }   # Build args (optional)
cache_from = []                      # Cache sources (optional)
no_cache = false                     # Disable cache (optional)
push = false                         # Push to registry after build
registry = ""                        # Registry for push/pull
scan = true                          # Security scan built images

[security]
profile = "restrictive"
```

## Implementation Details

### Image Naming Convention
```
agentkernel-{project-hash}:{dockerfile-content-hash}
```
Deterministic names enable caching; content-based hashing ensures rebuilds when Dockerfile changes.

### Build Flow
1. Check for Dockerfile (explicit or auto-detect)
2. Calculate content hash
3. Check if image exists locally
4. If not, run docker build with progress output
5. Tag image with deterministic name
6. Pass image name to backend

### Firecracker Integration
1. Build Docker image from Dockerfile
2. Export image layers: `docker save | tar`
3. Create ext4 rootfs from layers
4. Inject guest agent
5. Use rootfs with Firecracker

### Backend Compatibility

| Backend | Dockerfile Support | Notes |
|---------|-------------------|-------|
| Docker | Full | Native support |
| Podman | Full | Compatible with Docker build |
| Firecracker | Partial | Requires image-to-rootfs conversion |
| Apple Containers | Full | Standard OCI images |
| Hyperlight | Limited | Wasm modules only |

## Implementation Phases

### Phase 1: Local Docker Build (MVP)
1. Detect `Dockerfile` in project directory
2. Build image on sandbox creation
3. Tag with deterministic name for caching
4. Works with existing Docker backend

### Phase 2: Build Configuration
1. Add `[build]` section to `agentkernel.toml`
2. Support build args, target stages, context paths
3. Add `.dockerignore` handling

### Phase 3: Firecracker Conversion
1. For Firecracker: Convert Docker image to rootfs
2. Extract layers and create ext4 image
3. Inject guest agent binary

### Phase 4: Advanced Features
- Registry push/pull for team sharing
- Security scanning (Trivy, Grype)
- Build caching optimization
- Multi-platform builds (amd64/arm64)

## Security Considerations

1. **Untrusted Dockerfiles**: Build in isolated environment
2. **Network during build**: Optional `--no-network`
3. **Build args secrets**: Support `--secret` and `--ssh`
4. **Image scanning**: Integrate vulnerability detection
5. **Privileged builds**: Warn users, require explicit opt-in

## Critical Files for Implementation

1. `src/config.rs` - Add `[build]` section parsing
2. `src/vmm.rs` - Orchestrate image building before sandbox creation
3. `src/setup.rs` - Pattern for Docker builds (has `build_rootfs()`)
4. `src/backend/docker.rs` - Add `build_image()` method
5. `src/languages.rs` - Add `detect_dockerfile()` function
