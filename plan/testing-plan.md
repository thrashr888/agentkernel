# Comprehensive Testing Plan

## Current State Analysis

### Unit Tests by File (44 total)
| File | Tests | Status |
|------|-------|--------|
| src/agents.rs | 3 | Has tests |
| src/apple_backend.rs | 1 | Minimal |
| src/build.rs | 1 | Minimal |
| src/config.rs | 8 | Good |
| src/docker_backend.rs | 0 | **NEEDS TESTS** |
| src/firecracker_client.rs | 2 | Minimal |
| src/http_api.rs | 0 | **NEEDS TESTS** |
| src/hyperlight_backend.rs | 2 | Minimal |
| src/languages.rs | 8 | Good |
| src/lib.rs | 0 | N/A (re-exports) |
| src/main.rs | 0 | **NEEDS TESTS** |
| src/mcp.rs | 0 | **NEEDS TESTS** |
| src/permissions.rs | 5 | Good |
| src/pool.rs | 0 | **NEEDS TESTS** |
| src/sandbox_pool.rs | 0 | **NEEDS TESTS** |
| src/seatbelt.rs | 3 | Minimal |
| src/setup.rs | 0 | **NEEDS TESTS** |
| src/validation.rs | 8 | Good |
| src/vmm.rs | 0 | **NEEDS TESTS** |
| src/vsock.rs | 3 | Minimal |

### Backend Module (0 tests)
| File | Tests | Status |
|------|-------|--------|
| src/backend/mod.rs | 0 | **NEEDS TESTS** |
| src/backend/apple.rs | 0 | **NEEDS TESTS** |
| src/backend/docker.rs | 0 | **NEEDS TESTS** |
| src/backend/firecracker.rs | 0 | **NEEDS TESTS** |
| src/backend/hyperlight.rs | 0 | **NEEDS TESTS** |

### Integration Tests in /tests
| File | Purpose |
|------|---------|
| benchmark_test.rs | Performance benchmarks |
| hyperlight_benchmark.rs | Hyperlight-specific benchmarks |
| pool_benchmark.rs | Container pool benchmarks |
| sandbox_pool_benchmark.rs | Sandbox pool benchmarks |
| stress_test.rs | Parallel sandbox stress test |

**Missing**: Basic integration tests for CLI commands, end-to-end workflows

---

## Phase 1: Unit Test Coverage (Priority: High)

### 1.1 Critical Path Tests
These files are core to functionality and need tests urgently:

- [ ] **src/vmm.rs** - VM Manager tests
  - Test sandbox CRUD operations
  - Test backend selection logic
  - Test permission application
  - Test file injection

- [ ] **src/backend/mod.rs** - Sandbox trait tests
  - Test path validation
  - Test backend detection
  - Test create_sandbox factory

- [ ] **src/backend/docker.rs** - Docker backend tests
  - Test container naming
  - Test config translation
  - Test persistent vs ephemeral modes

- [ ] **src/pool.rs** - Container pool tests
  - Test pool warming
  - Test acquire/release
  - Test concurrent access

### 1.2 API Tests
- [ ] **src/http_api.rs** - HTTP API tests
  - Test route handlers
  - Test request/response serialization
  - Test error handling

- [ ] **src/mcp.rs** - MCP server tests
  - Test JSON-RPC message handling
  - Test tool execution
  - Test error responses

### 1.3 Infrastructure Tests
- [ ] **src/setup.rs** - Setup tests
  - Test installation detection
  - Test kernel/rootfs discovery

- [ ] **src/sandbox_pool.rs** - Sandbox pool tests
  - Test pool lifecycle
  - Test backend integration

---

## Phase 2: Integration Tests (Priority: High)

### 2.1 CLI Command Tests
Create `tests/cli_test.rs`:

```rust
// Test basic CLI commands work
- agentkernel --help
- agentkernel --version
- agentkernel status
- agentkernel list
- agentkernel agents
```

### 2.2 Sandbox Lifecycle Tests
Create `tests/sandbox_lifecycle_test.rs`:

```rust
// Test full sandbox lifecycle
- create → start → exec → stop → remove
- create with different backends (Docker, Apple)
- create with different images
- create with permissions
```

### 2.3 File Operations Tests
Create `tests/file_operations_test.rs`:

```rust
// Test file operations
- cp host → sandbox
- cp sandbox → host
- write_file via API
- read_file via API
```

### 2.4 API Integration Tests
Create `tests/http_api_test.rs`:

```rust
// Test HTTP API endpoints
- POST /sandboxes (create)
- POST /sandboxes/{name}/start
- POST /sandboxes/{name}/exec
- POST /sandboxes/{name}/stop
- DELETE /sandboxes/{name}
- GET /sandboxes (list)
```

Create `tests/mcp_test.rs`:

```rust
// Test MCP protocol
- tools/list
- tools/call (exec)
- tools/call (create_sandbox)
```

---

## Phase 3: Manual Testing Matrix

### 3.1 Backend Testing Matrix

| Test | Docker | Apple | Firecracker |
|------|--------|-------|-------------|
| create | ☐ | ☐ | ☐ |
| start | ☐ | ☐ | ☐ |
| exec | ☐ | ☐ | ☐ |
| attach | ☐ | ☐ | ☐ |
| cp to | ☐ | ☐ | ☐ |
| cp from | ☐ | ☐ | ☐ |
| stop | ☐ | ☐ | ☐ |
| remove | ☐ | ☐ | ☐ |

### 3.2 Image/Runtime Testing Matrix

| Runtime | Docker | Apple | Firecracker |
|---------|--------|-------|-------------|
| alpine | ☐ | ☐ | ☐ |
| python | ☐ | ☐ | ☐ |
| node | ☐ | ☐ | ☐ |
| rust | ☐ | ☐ | ☐ |

### 3.3 Agent Integration Testing

| Agent | Install | Detection | Run | Sandbox |
|-------|---------|-----------|-----|---------|
| Claude Code | ☐ | ☐ | ☐ | ☐ |
| Gemini CLI | ☐ | ☐ | ☐ | ☐ |
| Codex | ☐ | ☐ | ☐ | ☐ |
| OpenCode | ☐ | ☐ | ☐ | ☐ |

### 3.4 Permission Profile Testing

| Profile | Network | Mount CWD | Mount Home | Read-only |
|---------|---------|-----------|------------|-----------|
| permissive | ☐ | ☐ | ☐ | ☐ |
| moderate | ☐ | ☐ | ☐ | ☐ |
| restrictive | ☐ | ☐ | ☐ | ☐ |

---

## Phase 4: CI/CD Integration

### 4.1 GitHub Actions Workflow
- [ ] Add test workflow that runs on PR
- [ ] Add test coverage reporting
- [ ] Add clippy/fmt checks
- [ ] Add cross-platform testing (Linux/macOS)

### 4.2 Local Testing Script
Create `scripts/test-all.sh`:
```bash
#!/bin/bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
cargo test --test cli_test
cargo test --test sandbox_lifecycle_test
```

---

## Implementation Order

1. **Week 1**: Unit tests for critical path (vmm.rs, backend/*.rs, pool.rs)
2. **Week 2**: Integration tests (CLI, sandbox lifecycle)
3. **Week 3**: API tests (HTTP, MCP) + manual testing
4. **Week 4**: CI/CD setup + documentation

---

## Success Criteria

- [ ] All source files have at least basic unit tests
- [ ] Integration tests cover main CLI workflows
- [ ] Manual testing matrix completed for Docker backend
- [ ] CI/CD runs tests on every PR
- [ ] Test coverage > 60%
